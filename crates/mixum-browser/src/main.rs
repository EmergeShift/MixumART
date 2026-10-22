//! Apply a user-approved browser lease to a container Chromium profile.
//!
//! The lease is read from a file, cookies are injected over CDP before tabs are
//! opened, and the file is expected to be removed by the caller. No cookie
//! value is printed or written to logs.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    if command == "--help" || command == "-h" {
        println!("usage: mixum-browser apply /path/to/browser-lease.json");
        return;
    }
    if command != "apply" {
        eprintln!("mixum-browser: only `apply <lease.json>` is supported");
        std::process::exit(2);
    }
    let Some(path) = args.next() else {
        eprintln!("mixum-browser: missing lease path");
        std::process::exit(2);
    };
    if let Err(error) = apply(PathBuf::from(path)).await {
        eprintln!("mixum-browser: {error}");
        std::process::exit(1);
    }
}

async fn apply(path: PathBuf) -> Result<(), String> {
    let raw = tokio::fs::read(&path)
        .await
        .map_err(|error| format!("read lease: {error}"))?;
    let lease: Value =
        serde_json::from_slice(&raw).map_err(|error| format!("parse lease: {error}"))?;
    let port = std::env::var("MIXUM_CHROMIUM_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(9222);
    let chromium = std::env::var("MIXUM_CHROMIUM").unwrap_or_else(|_| {
        [
            "/usr/bin/chromium",
            "/usr/sbin/chromium",
            "/usr/bin/chromium-browser",
        ]
        .iter()
        .find(|path| std::path::Path::new(path).exists())
        .map(|path| (*path).to_owned())
        .unwrap_or_else(|| "chromium".into())
    });
    let profile = std::env::var("MIXUM_BROWSER_PROFILE")
        .unwrap_or_else(|_| "/var/lib/mixum/browser/profile".into());
    if chromium.contains('/') && !std::path::Path::new(&chromium).exists() {
        return Err(format!(
            "Chromium not found at {chromium}; install it in the base with `mixum base shell`"
        ));
    }
    if !devtools_ready(port).await {
        launch(&chromium, &profile, port)?;
        wait_devtools(port).await?;
    }
    eprintln!("mixum-browser: devtools ready");
    let ws_url = devtools_ws(port).await?;
    eprintln!("mixum-browser: websocket {ws_url}");
    let (ws, _) = connect_async(&ws_url)
        .await
        .map_err(|error| format!("CDP connect: {error}"))?;
    let (mut sink, mut source) = ws.split();
    let mut sequence = 0u64;
    let cookies = lease
        .get("cookies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Normalize Chrome's cookie shape to CDP Storage.Cookie.
    let cdp_cookies: Vec<Value> = cookies
        .iter()
        .filter_map(|cookie| {
            let name = cookie.get("name")?.as_str()?;
            let value = cookie.get("value")?.as_str()?;
            let domain = cookie.get("domain")?.as_str()?;
            let path = cookie.get("path").and_then(Value::as_str).unwrap_or("/");
            let mut out = json!({
                "name": name,
                "value": value,
                "domain": domain,
                "path": path,
                "secure": cookie.get("secure").and_then(Value::as_bool).unwrap_or(false),
                "httpOnly": cookie.get("httpOnly").and_then(Value::as_bool).unwrap_or(false),
                "sameSite": cookie.get("sameSite").and_then(Value::as_str).unwrap_or("Lax"),
            });
            if let Some(expires) = cookie.get("expirationDate").and_then(Value::as_f64) {
                out["expires"] = json!(expires);
            }
            if let Some(partition) = cookie.get("partitionKey") {
                if !partition.is_null() {
                    out["partitionKey"] = partition.clone();
                }
            }
            Some(out)
        })
        .collect();
    eprintln!("mixum-browser: setting {} cookies", cdp_cookies.len());
    if !cdp_cookies.is_empty() {
        cdp_call(
            &mut sink,
            &mut source,
            &mut sequence,
            "Storage.setCookies",
            json!({"cookies": cdp_cookies}),
        )
        .await?;
    }
    eprintln!("mixum-browser: cookies done");
    let mut opened = 0usize;
    if let Some(tabs) = lease.get("tabs").and_then(Value::as_array) {
        for tab in tabs {
            let Some(url) = tab.get("url").and_then(Value::as_str) else {
                continue;
            };
            if !url.starts_with("http://") && !url.starts_with("https://") {
                continue;
            }
            cdp_call(
                &mut sink,
                &mut source,
                &mut sequence,
                "Target.createTarget",
                json!({"url": url}),
            )
            .await?;
            opened += 1;
        }
    }
    println!(
        "{} cookies, {} tabs applied to {}",
        cdp_cookies.len(),
        opened,
        profile
    );
    Ok(())
}

async fn cdp_call(
    sink: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
        Message,
    >,
    source: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>,
    >,
    sequence: &mut u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    *sequence += 1;
    let id = *sequence;
    let message = json!({"id": id, "method": method, "params": params});
    sink.send(Message::Text(message.to_string().into()))
        .await
        .map_err(|error| format!("CDP send {method}: {error}"))?;
    loop {
        let Some(message) = source.next().await else {
            return Err(format!("CDP closed while waiting for {method}"));
        };
        let message = message.map_err(|error| format!("CDP recv {method}: {error}"))?;
        if let Message::Text(text) = message {
            let value: Value =
                serde_json::from_str(&text).map_err(|error| format!("CDP JSON: {error}"))?;
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = value.get("error") {
                    return Err(format!("{method} failed: {error}"));
                }
                return Ok(value.get("result").cloned().unwrap_or(Value::Null));
            }
        }
    }
}

fn launch(chromium: &str, profile: &str, port: u16) -> Result<(), String> {
    let _ = std::fs::create_dir_all("/tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions("/tmp", std::fs::Permissions::from_mode(0o1777));
    }
    std::fs::create_dir_all(profile).map_err(|error| format!("create profile: {error}"))?;
    let display = std::env::var("WAYLAND_DISPLAY").unwrap_or_else(|_| {
        if std::path::Path::new("/run/mixum/wayland-0").exists() {
            "wayland-0".into()
        } else {
            String::new()
        }
    });
    let mut command = Command::new(chromium);
    command
        .arg(format!("--remote-debugging-port={port}"))
        .arg("--remote-allow-origins=*")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-dev-shm-usage")
        // Chromium refuses to start as uid 0 without this. The controller runs
        // inside the container; a non-root sidecar can drop this flag later.
        .arg("--no-sandbox")
        .arg(format!("--user-data-dir={profile}"))
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if !display.is_empty() {
        command
            .env("WAYLAND_DISPLAY", &display)
            .env("XDG_RUNTIME_DIR", "/run/mixum")
            .arg("--ozone-platform=wayland");
    } else {
        command.arg("--headless=new");
    }
    command
        .spawn()
        .map_err(|error| format!("launch Chromium: {error}"))?;
    Ok(())
}

async fn devtools_ready(port: u16) -> bool {
    devtools_ws(port).await.is_ok()
}

async fn wait_devtools(port: u16) -> Result<(), String> {
    for _ in 0..40 {
        if devtools_ready(port).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(format!(
        "Chromium CDP did not become ready on 127.0.0.1:{port}"
    ))
}

async fn devtools_ws(port: u16) -> Result<String, String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|error| format!("connect CDP: {error}"))?;
    stream
        .write_all(
            format!(
                "GET /json/version HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .map_err(|error| format!("CDP HTTP: {error}"))?;
    let mut response = Vec::new();
    let mut expected = None;
    loop {
        let mut chunk = [0u8; 8192];
        let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut chunk))
            .await
            .map_err(|_| "CDP HTTP read timeout".to_string())?
            .map_err(|error| format!("CDP read: {error}"))?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&chunk[..read]);
        if let Some(header_end) = find_bytes(&response, b"\r\n\r\n") {
            if expected.is_none() {
                let headers = String::from_utf8_lossy(&response[..header_end]);
                expected = headers.lines().find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                });
            }
            if let Some(length) = expected {
                if response.len() >= header_end + 4 + length {
                    break;
                }
            }
        }
    }
    let header_end = find_bytes(&response, b"\r\n\r\n")
        .ok_or_else(|| "CDP HTTP response has no headers".to_string())?;
    let body = &response[header_end + 4..];
    let value: Value =
        serde_json::from_slice(body).map_err(|error| format!("CDP version JSON: {error}"))?;
    value
        .get("webSocketDebuggerUrl")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "CDP version has no webSocketDebuggerUrl".into())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
