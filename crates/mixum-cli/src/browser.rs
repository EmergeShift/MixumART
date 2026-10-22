//! Read-only MAP browser monitor for one surface.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::Path;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use mixum_env_core::ContainerState;
use mixum_env_map::MapClient;
use mixum_env_surface::FORMAT_PNG;
use rand::RngCore;
use serde::Deserialize;
use serde_json::json;

use crate::env;
use crate::map_ops;
use crate::output::{self, CliError, Result};

#[derive(Clone)]
struct Monitor {
    client: MapClient,
    surface: u32,
}

#[derive(Deserialize)]
struct TokenQuery {
    token: String,
}

pub async fn open(
    data_dir: &Path,
    selector: &str,
    window: Option<u32>,
    port: u16,
    no_open: bool,
    json: bool,
) -> Result<()> {
    let registry = env::registry();
    let resolved = env::find_with(&registry, data_dir, selector).await?;
    if resolved.container.state != ContainerState::Running {
        return Err(CliError::conflict(format!(
            "environment {} is not running",
            resolved.container.name
        )));
    }
    let map_port = resolved
        .container
        .ports
        .iter()
        .find(|mapping| mapping.container == crate::ports::MAP_PORT)
        .map(|mapping| mapping.host)
        .ok_or_else(|| CliError::map("environment has no published MAP port"))?;
    let client = map_ops::connect(
        &registry,
        resolved.kind,
        &resolved.container.id,
        map_port,
        resolved
            .record
            .as_ref()
            .and_then(|record| record.fingerprint.as_deref()),
    )
    .await?;
    let surfaces = client
        .list()
        .await
        .map_err(|error| CliError::map(format!("display.list failed: {error}")))?;
    let surface = match window {
        Some(id) => surfaces
            .iter()
            .find(|surface| surface.id == id)
            .cloned()
            .ok_or_else(|| {
                CliError::not_found(format!(
                    "window {id} not found; available: {}",
                    surfaces
                        .iter()
                        .map(|surface| surface.id.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?,
        None if surfaces.len() == 1 => surfaces[0].clone(),
        None if surfaces.is_empty() => {
            return Err(CliError::not_found("environment has no windows"))
        }
        None => {
            return Err(CliError::usage(format!(
                "environment has multiple windows; pass --window: {}",
                surfaces
                    .iter()
                    .map(|surface| format!("{} ({})", surface.id, surface.title))
                    .collect::<Vec<_>>()
                    .join(", ")
            )))
        }
    };
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let listener = bind(port).await?;
    let actual = listener
        .local_addr()
        .map_err(|error| CliError::io(format!("cannot read listener port: {error}")))?;
    let monitor = Arc::new(Monitor {
        client,
        surface: surface.id,
    });
    let app = Router::new()
        .route("/", get(index))
        .route("/ws", get(ws))
        .with_state((monitor, token.clone()));
    let url = format!("http://127.0.0.1:{}/?token={}", actual.port(), token);
    if json {
        output::print_json(&json!({
            "url": url,
            "environment": resolved.container.name,
            "window": surface.id,
            "title": surface.title,
            "readOnly": true,
        }));
        eprintln!("read-only monitor; press Ctrl-C to stop");
    } else if no_open {
        println!("{url}");
        eprintln!("read-only monitor; press Ctrl-C to stop");
    } else {
        println!("mixum monitor: {url}");
        println!("read-only monitor; press Ctrl-C to stop");
        if let Err(error) = open_browser(&url) {
            eprintln!("could not open browser: {error}; open the URL manually");
        }
    }
    axum::serve(listener, app)
        .await
        .map_err(|error| CliError::runtime(format!("monitor server failed: {error}")))
}

fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(url);
        command
    };
    let status = command
        .status()
        .map_err(|error| CliError::io(format!("open browser: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(CliError::io(format!(
            "browser launcher exited with {status}"
        )))
    }
}

async fn bind(port: u16) -> Result<tokio::net::TcpListener> {
    let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| CliError::io(format!("cannot bind monitor on 127.0.0.1:{port}: {error}")))
}

async fn index() -> impl IntoResponse {
    Html(INDEX)
}

async fn ws(
    ws: WebSocketUpgrade,
    Query(query): Query<TokenQuery>,
    State((monitor, token)): State<(Arc<Monitor>, String)>,
) -> impl IntoResponse {
    if query.token != token {
        return StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| stream(socket, monitor))
        .into_response()
}

async fn stream(mut socket: WebSocket, monitor: Arc<Monitor>) {
    // Subscribe before attach so the initial full-frame replay is not lost.
    let mut frames = monitor.client.frames();
    if let Err(error) = monitor.client.attach(monitor.surface).await {
        eprintln!("monitor attach failed: {error}");
        return;
    }
    loop {
        tokio::select! {
            frame = frames.recv() => {
                match frame {
                    Ok(frame) if frame.surface == monitor.surface && frame.format == FORMAT_PNG => {
                        let mut payload = Vec::with_capacity(24 + frame.data.len());
                        for value in [frame.x, frame.y, frame.w, frame.h, frame.full_w, frame.full_h] {
                            payload.extend_from_slice(&value.to_be_bytes());
                        }
                        payload.extend_from_slice(&frame.data);
                        if socket.send(Message::Binary(payload.into())).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            message = socket.recv() => {
                match message {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }
        }
    }
}

const INDEX: &str = r#"<!doctype html>
<html><head><meta charset="utf-8"><title>mixum monitor</title>
<style>html,body{margin:0;background:#111;color:#ddd;font:13px system-ui}canvas{display:block;max-width:100vw;max-height:100vh;margin:auto}header{padding:8px 12px;background:#1b1b1d;color:#aaa}</style>
</head><body><header>mixum · read-only window</header><canvas id="c"></canvas>
<script>
const token = new URLSearchParams(location.search).get('token');
const c = document.getElementById('c');
const ctx = c.getContext('2d');
let sized = false;
const ws = new WebSocket(`ws://${location.host}/ws?token=${encodeURIComponent(token)}`);
ws.binaryType = 'arraybuffer';
ws.onmessage = (event) => {
  const b = event.data;
  if (b.byteLength < 24) return;
  const dv = new DataView(b);
  const x = dv.getUint32(0), y = dv.getUint32(4), w = dv.getUint32(8), h = dv.getUint32(12);
  const fw = dv.getUint32(16), fh = dv.getUint32(20);
  if (!sized) { c.width = fw; c.height = fh; sized = true; }
  const png = new Uint8Array(b, 24);
  const img = new Image();
  img.onload = () => { ctx.drawImage(img, x, y, w, h); URL.revokeObjectURL(img.src); };
  img.src = URL.createObjectURL(new Blob([png], {type:'image/png'}));
};
ws.onclose = () => { document.querySelector('header').textContent = 'mixum · disconnected'; };
</script></body></html>"#;
