use std::path::Path;
use std::time::{Duration, Instant};

use mixum_env_core::{BackendRegistry, RuntimeKind};
use mixum_env_map::MapClient;
use serde_json::Value;

use crate::env;
use crate::output::{self, CliError, Result};

pub struct MapReady {
    pub fingerprint: String,
    pub windows: usize,
}

pub async fn connect(
    registry: &BackendRegistry,
    kind: RuntimeKind,
    container_id: &str,
    port: u16,
    fingerprint: Option<&str>,
) -> Result<MapClient> {
    if port == 0 {
        return Err(CliError::map(
            "environment has no published MAP port; recreate it with a MAP port",
        ));
    }
    let backend = registry
        .get(kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    let token = backend
        .admin_token(container_id)
        .await
        .map_err(|error| CliError::map(format!("cannot read agentd token: {error}")))?;
    MapClient::connect(&format!("127.0.0.1:{port}"), &token, fingerprint)
        .await
        .map_err(|error| CliError::map(format!("MAP handshake failed: {error}")))
}

pub async fn wait_ready(
    registry: &BackendRegistry,
    kind: RuntimeKind,
    container_id: &str,
    name: &str,
    port: u16,
    timeout: u64,
) -> Result<MapReady> {
    let _ = mixum_env_map::forget_known_host(&format!("127.0.0.1:{port}"));
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let last_error = match connect(registry, kind, container_id, port, None).await {
            Ok(client) => match client.list().await {
                Ok(windows) => {
                    return Ok(MapReady {
                        fingerprint: client.fingerprint,
                        windows: windows.len(),
                    })
                }
                Err(error) => error.to_string(),
            },
            Err(error) => error.message,
        };
        if Instant::now() >= deadline {
            return Err(CliError::map(format!(
                "MAP for {name} was not ready within {timeout}s: {last_error}"
            )));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

pub(crate) async fn env_client(
    data_dir: &Path,
    selector: &str,
) -> Result<(env::ResolvedEnv, MapClient)> {
    let registry = env::registry();
    let resolved = env::find_with(&registry, data_dir, selector).await?;
    if resolved.container.state != mixum_env_core::ContainerState::Running {
        return Err(CliError::conflict(format!(
            "environment {} is {}; start it first",
            resolved.container.name,
            env::state_label(resolved.container.state)
        )));
    }
    let port = resolved
        .container
        .ports
        .iter()
        .find(|mapping| mapping.container == crate::ports::MAP_PORT)
        .map(|mapping| mapping.host)
        .ok_or_else(|| CliError::map("environment has no published MAP port"))?;
    let client = connect(
        &registry,
        resolved.kind,
        &resolved.container.id,
        port,
        resolved
            .record
            .as_ref()
            .and_then(|record| record.fingerprint.as_deref()),
    )
    .await?;
    Ok((resolved, client))
}

pub async fn windows(data_dir: &Path, selector: &str, timeout: u64, json: bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(timeout.max(1));
    let (resolved, client) = loop {
        match env_client(data_dir, selector).await {
            Ok(value) => break value,
            Err(error) if Instant::now() < deadline && error.code == 6 => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(error) => return Err(error),
        }
    };
    let surfaces = client
        .list()
        .await
        .map_err(|error| CliError::map(format!("display.list failed: {error}")))?;
    if json {
        output::print_json(&serde_json::json!({
            "environment": resolved.container.name,
            "fingerprint": client.fingerprint,
            "windows": surfaces,
        }));
    } else if surfaces.is_empty() {
        println!("{}: no windows", resolved.container.name);
    } else {
        let rows = surfaces
            .iter()
            .map(|surface| {
                vec![
                    surface.id.to_string(),
                    surface.title.clone(),
                    surface.app_id.clone(),
                    format!("{}x{}", surface.width, surface.height),
                    if surface.id >= 1000 {
                        "enhanced"
                    } else {
                        "compat"
                    }
                    .to_string(),
                ]
            })
            .collect::<Vec<_>>();
        output::table(&["ID", "TITLE", "APP", "SIZE", "MODE"], &rows);
    }
    Ok(())
}

pub async fn snapshot_list(data_dir: &Path, selector: &str, json: bool) -> Result<()> {
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let tree = client
        .snapshot_tree()
        .await
        .map_err(|error| CliError::map(format!("snap.tree failed: {error}")))?;
    if json {
        output::print_json(&tree);
    } else {
        print_snapshot_tree(&tree);
    }
    Ok(())
}

pub async fn snapshot_create(
    data_dir: &Path,
    selector: &str,
    label: Option<&str>,
    kind: &str,
    json: bool,
) -> Result<()> {
    if !matches!(kind, "disk" | "memory" | "hybrid") {
        return Err(CliError::usage(
            "snapshot kind must be disk, memory or hybrid",
        ));
    }
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let result = client
        .snapshot_create_kind(kind, label)
        .await
        .map_err(|error| CliError::map(format!("snap.create failed: {error}")))?;
    if json {
        output::print_json(&result);
    } else {
        println!(
            "snapshot created: {}",
            result.get("id").and_then(Value::as_str).unwrap_or("?")
        );
    }
    Ok(())
}

pub async fn snapshot_restore(
    data_dir: &Path,
    selector: &str,
    node: &str,
    yes: bool,
    json: bool,
) -> Result<()> {
    if !yes {
        return Err(CliError::usage(
            "snapshot restore is destructive; pass --yes after verifying the environment",
        ));
    }
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let result = client
        .snapshot_restore(node)
        .await
        .map_err(|error| CliError::map(format!("snap.restore failed: {error}")))?;
    if json {
        output::print_json(&result);
    } else {
        println!("restored snapshot {node}");
    }
    Ok(())
}

pub async fn snapshot_fork(
    data_dir: &Path,
    selector: &str,
    node: &str,
    branch: &str,
    json: bool,
) -> Result<()> {
    if branch.trim().is_empty() {
        return Err(CliError::usage("--branch cannot be empty"));
    }
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let result = client
        .snapshot_fork(node, branch)
        .await
        .map_err(|error| CliError::map(format!("snap.fork failed: {error}")))?;
    if json {
        output::print_json(&result);
    } else {
        println!("forked {node} into branch {branch}");
    }
    Ok(())
}

pub async fn snapshot_export(
    data_dir: &Path,
    selector: &str,
    node: &str,
    out: &Path,
    json: bool,
) -> Result<()> {
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let result = client
        .snapshot_export(node)
        .await
        .map_err(|error| CliError::map(format!("snap.export failed: {error}")))?;
    let file = result
        .get("file")
        .and_then(Value::as_str)
        .ok_or_else(|| CliError::map("snap.export did not return a file"))?;
    let remote_path = if file.starts_with("transfers/") {
        file.to_owned()
    } else {
        format!("transfers/{file}")
    };
    let data = client
        .get_file(&remote_path)
        .await
        .map_err(|error| CliError::map(format!("file.get failed: {error}")))?;
    std::fs::write(out, &data)
        .map_err(|error| CliError::io(format!("write {}: {error}", out.display())))?;
    let value = serde_json::json!({
        "node": node,
        "path": out,
        "bytes": data.len(),
        "remote": result,
    });
    if json {
        output::print_json(&value);
    } else {
        println!(
            "exported {node} to {} ({} bytes)",
            out.display(),
            data.len()
        );
    }
    Ok(())
}

pub async fn snapshot_import(
    data_dir: &Path,
    selector: &str,
    file: &Path,
    origin: &str,
    json: bool,
) -> Result<()> {
    let data = std::fs::read(file)
        .map_err(|error| CliError::io(format!("read {}: {error}", file.display())))?;
    if data.is_empty() {
        return Err(CliError::usage("snapshot bundle is empty"));
    }
    let name = file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("snapshot.mart");
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let remote = client
        .put_file(name, &data)
        .await
        .map_err(|error| CliError::map(format!("file.put failed: {error}")))?;
    let result = client
        .snapshot_import(&remote, origin)
        .await
        .map_err(|error| CliError::map(format!("snap.import failed: {error}")))?;
    if json {
        output::print_json(&serde_json::json!({
            "path": file,
            "bytes": data.len(),
            "result": result,
        }));
    } else {
        println!("imported {} into {selector}", file.display());
    }
    Ok(())
}

fn print_snapshot_tree(value: &Value) {
    let nodes = value
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if nodes.is_empty() {
        println!("no snapshots");
        return;
    }
    let rows = nodes
        .iter()
        .map(|node| {
            vec![
                node.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                node.get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                node.get("branch")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                node.get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ]
        })
        .collect::<Vec<_>>();
    output::table(&["ID", "KIND", "BRANCH", "LABEL"], &rows);
}

pub async fn privilege_watch(data_dir: &Path, selector: &str, json: bool) -> Result<()> {
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let mut events = client.events();
    if !json {
        println!("watching privilege requests for {selector}; Ctrl-C to stop");
    }
    loop {
        match events.recv().await {
            Ok(event) if event.op == "privilege.request" => {
                if json {
                    output::print_json(&event.payload);
                } else {
                    let id = event
                        .payload
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("?");
                    let risk = event
                        .payload
                        .get("risk")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    let program = event
                        .payload
                        .get("program")
                        .and_then(Value::as_str)
                        .unwrap_or("?");
                    let args = event
                        .payload
                        .get("args")
                        .and_then(Value::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(" ")
                        })
                        .unwrap_or_default();
                    println!("{id}  [{risk}]  {program} {args}");
                    if let Some(reason) = event.payload.get("reason").and_then(Value::as_str) {
                        if !reason.is_empty() {
                            println!("  reason: {reason}");
                        }
                    }
                    println!("  approve: mixum privilege approve {selector} {id}");
                }
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                if !json {
                    eprintln!("warning: skipped {skipped} events");
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }
    Ok(())
}

pub async fn privilege_decide(
    data_dir: &Path,
    selector: &str,
    id: &str,
    approve: bool,
    reason: &str,
    json: bool,
) -> Result<()> {
    if approve {
        authenticate_host()?;
    }
    let (_resolved, client) = env_client(data_dir, selector).await?;
    client
        .privilege_decision(id, approve, reason)
        .await
        .map_err(|error| CliError::map(format!("privilege.decision failed: {error}")))?;
    if json {
        output::print_json(&serde_json::json!({
            "id": id,
            "approved": approve,
            "reason": reason,
        }));
    } else {
        println!(
            "{} privilege request {id}",
            if approve { "approved" } else { "denied" }
        );
    }
    Ok(())
}

/// Host-password gate for the CLI-only approval path.
/// `sudo -v` validates the current OS user without running a privileged command;
/// the agent cannot invoke this because it runs inside the container.
fn authenticate_host() -> Result<()> {
    let status = std::process::Command::new("sudo")
        .arg("-v")
        .status()
        .map_err(|error| CliError::runtime(format!("host authentication unavailable: {error}")))?;
    if !status.success() {
        return Err(CliError::conflict(
            "host authentication failed; request was not approved",
        ));
    }
    Ok(())
}

pub async fn screenshot(
    data_dir: &Path,
    selector: &str,
    window: u32,
    out: Option<&Path>,
    json: bool,
) -> Result<()> {
    use base64::Engine as _;
    use tokio::sync::broadcast::error::RecvError;

    let (_resolved, client) = env_client(data_dir, selector).await?;
    let mut frames = client.frames();
    client
        .attach(window)
        .await
        .map_err(|error| CliError::map(format!("display.attach failed: {error}")))?;
    let frame = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            match frames.recv().await {
                Ok(frame) if frame.surface == window => return Some(frame),
                Ok(_) => continue,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => return None,
            }
        }
    })
    .await
    .map_err(|_| CliError::map("screenshot timed out waiting for the first PNG frame"))?
    .ok_or_else(|| CliError::map("frame stream closed before a screenshot arrived"))?;
    let _ = client.detach(window).await;
    if frame.format != mixum_env_surface::FORMAT_PNG || frame.data.is_empty() {
        return Err(CliError::map(
            "node did not return a PNG frame; screenshot requires codec=png support",
        ));
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(&frame.data);
    if let Some(path) = out {
        std::fs::write(path, &frame.data)
            .map_err(|error| CliError::io(format!("write screenshot: {error}")))?;
    }
    if json {
        let mut value = serde_json::json!({
            "window": window,
            "width": frame.full_w,
            "height": frame.full_h,
            "bytes": frame.data.len(),
        });
        if let Some(path) = out {
            value["path"] = serde_json::json!(path);
        } else {
            value["pngBase64"] = serde_json::json!(encoded);
        }
        output::print_json(&value);
    } else if let Some(path) = out {
        println!(
            "wrote {} ({}x{})",
            path.display(),
            frame.full_w,
            frame.full_h
        );
    } else {
        return Err(CliError::usage(
            "screenshot requires --out, or --json for base64 output",
        ));
    }
    Ok(())
}

pub async fn click(
    data_dir: &Path,
    selector: &str,
    window: u32,
    x: f64,
    y: f64,
    button: &str,
    json: bool,
) -> Result<()> {
    use mixum_env_surface::InputEvent;
    let (_resolved, client) = env_client(data_dir, selector).await?;
    client
        .input(
            window,
            &InputEvent::PointerMotion {
                surface: window,
                x,
                y,
            },
        )
        .await
        .map_err(|error| CliError::map(format!("pointer_motion failed: {error}")))?;
    client
        .input(
            window,
            &InputEvent::PointerButton {
                surface: window,
                button: button.to_owned(),
                action: "click".into(),
            },
        )
        .await
        .map_err(|error| CliError::map(format!("pointer_button failed: {error}")))?;
    output_ok(
        json,
        serde_json::json!({"window": window, "clicked": [x, y]}),
    )
}

pub async fn move_pointer(
    data_dir: &Path,
    selector: &str,
    window: u32,
    x: f64,
    y: f64,
    json: bool,
) -> Result<()> {
    use mixum_env_surface::InputEvent;
    let (_resolved, client) = env_client(data_dir, selector).await?;
    client
        .input(
            window,
            &InputEvent::PointerMotion {
                surface: window,
                x,
                y,
            },
        )
        .await
        .map_err(|error| CliError::map(format!("pointer_motion failed: {error}")))?;
    output_ok(json, serde_json::json!({"window": window, "moved": [x, y]}))
}

pub async fn type_text(
    data_dir: &Path,
    selector: &str,
    window: u32,
    text: &str,
    json: bool,
) -> Result<()> {
    use mixum_env_surface::InputEvent;
    let (_resolved, client) = env_client(data_dir, selector).await?;
    client
        .input(
            window,
            &InputEvent::Text {
                surface: window,
                text: text.to_owned(),
            },
        )
        .await
        .map_err(|error| CliError::map(format!("text input failed: {error}")))?;
    output_ok(
        json,
        serde_json::json!({"window": window, "typed": text.chars().count()}),
    )
}

pub async fn key(
    data_dir: &Path,
    selector: &str,
    window: u32,
    keysym: &str,
    mods: &[String],
    json: bool,
) -> Result<()> {
    let (_resolved, client) = env_client(data_dir, selector).await?;
    client
        .input_raw(
            window,
            serde_json::json!({
                "type": "key",
                "keysym": keysym,
                "action": "click",
                "mods": mods,
            }),
        )
        .await
        .map_err(|error| CliError::map(format!("key input failed: {error}")))?;
    output_ok(
        json,
        serde_json::json!({"window": window, "keysym": keysym, "mods": mods}),
    )
}

pub async fn scroll(
    data_dir: &Path,
    selector: &str,
    window: u32,
    x: f64,
    y: f64,
    direction: &str,
    json: bool,
) -> Result<()> {
    let action = match direction {
        "up" | "scroll_up" => "scroll_up",
        "down" | "scroll_down" => "scroll_down",
        other => {
            return Err(CliError::usage(format!(
                "unsupported scroll direction: {other}"
            )))
        }
    };
    let (_resolved, client) = env_client(data_dir, selector).await?;
    client
        .input_raw(
            window,
            serde_json::json!({"type":"pointer_motion","x":x,"y":y}),
        )
        .await
        .map_err(|error| CliError::map(format!("pointer_motion failed: {error}")))?;
    client
        .input_raw(
            window,
            serde_json::json!({"type":"pointer_axis","action":action}),
        )
        .await
        .map_err(|error| CliError::map(format!("pointer_axis failed: {error}")))?;
    output_ok(
        json,
        serde_json::json!({"window": window, "scrolled": action}),
    )
}

pub async fn focus(data_dir: &Path, selector: &str, window: u32, json: bool) -> Result<()> {
    use mixum_env_surface::InputEvent;
    let (_resolved, client) = env_client(data_dir, selector).await?;
    client
        .input(window, &InputEvent::Focus { surface: window })
        .await
        .map_err(|error| CliError::map(format!("focus failed: {error}")))?;
    output_ok(json, serde_json::json!({"window": window, "focused": true}))
}

fn output_ok(json: bool, value: serde_json::Value) -> Result<()> {
    if json {
        output::print_json(&value);
    } else {
        println!("ok");
    }
    Ok(())
}
