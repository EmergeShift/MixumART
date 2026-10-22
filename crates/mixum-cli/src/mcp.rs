//! Minimal stdio MCP server. It shells out to the same `mixum` binary with
//! `--json`, so there is exactly one implementation of state/port/MAP rules.
//! Terminal and arbitrary shell execution are intentionally not exposed.

use std::io::{self, BufRead, Write};
use std::process::Command;

use serde_json::{json, Value};

use crate::output::{CliError, Result};

const PROTOCOL_VERSION: &str = "2024-11-05";

pub fn serve() -> Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| CliError::io(format!("MCP stdin: {error}")))?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                write_message(
                    &mut stdout,
                    &json_rpc_error(Value::Null, -32700, &error.to_string()),
                )?;
                continue;
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let response = match method {
            "initialize" => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "mixum", "version": env!("CARGO_PKG_VERSION") },
                }
            })),
            "notifications/initialized" | "notifications/cancelled" => None,
            "ping" => Some(json!({"jsonrpc":"2.0","id":id,"result":{}})),
            "tools/list" => Some(json!({"jsonrpc":"2.0","id":id,"result":{"tools": tools()}})),
            "tools/call" => Some(handle_tool(&id, &request)),
            _ => Some(json_rpc_error(id, -32601, "method not found")),
        };
        if let Some(response) = response {
            write_message(&mut stdout, &response)?;
        }
    }
    Ok(())
}

fn write_message(stdout: &mut io::Stdout, value: &Value) -> Result<()> {
    let text = serde_json::to_string(value)
        .map_err(|error| CliError::io(format!("MCP encode: {error}")))?;
    writeln!(stdout, "{text}").map_err(|error| CliError::io(format!("MCP stdout: {error}")))?;
    stdout
        .flush()
        .map_err(|error| CliError::io(format!("MCP stdout: {error}")))
}

fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

fn tool(name: &str, description: &str, properties: Value, required: Vec<&str>) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }
    })
}

fn tools() -> Vec<Value> {
    vec![
        tool(
            "mixum_env_list",
            "List managed MixumOS environments. Set all=true to include every runtime container.",
            json!({"all":{"type":"boolean","default":false}}),
            vec![],
        ),
        tool(
            "mixum_env_create",
            "Create a MixumOS environment with MAP. Prefer profile=mixumos. Returns runtime, MAP address and window count; never returns the token.",
            json!({
                "title": {"type":"string","description":"Human title, 1-120 characters"},
                "method": {"type":"string","enum":["subagent","as-tool","isolated"],"default":"subagent"},
                "profile": {"type":"string","enum":["mixumos","generic"],"default":"mixumos"},
                "image": {"type":"string","description":"Optional local image reference; required for generic"},
                "runtime": {"type":"string","enum":["docker","wsl2","apple"]},
                "port": {"type":"integer","minimum":0,"maximum":65535,"description":"0 allocates 17600-17999"},
                "noStart": {"type":"boolean","default":false},
                "waitMap": {"type":"integer","minimum":0,"maximum":300,"default":30}
            }),
            vec!["title"],
        ),
        tool(
            "mixum_env_show",
            "Show one environment, its MAP fingerprint and windows.",
            json!({"selector":{"type":"string","description":"id, name, title or unique prefix"}}),
            vec!["selector"],
        ),
        tool(
            "mixum_env_windows",
            "List MAP windows for a running environment.",
            json!({"selector":{"type":"string"},"timeout":{"type":"integer","minimum":1,"maximum":300,"default":30}}),
            vec!["selector"],
        ),
        tool(
            "mixum_env_start",
            "Start an environment.",
            json!({"selector":{"type":"string"}}),
            vec!["selector"],
        ),
        tool(
            "mixum_env_stop",
            "Stop an environment.",
            json!({"selector":{"type":"string"}}),
            vec!["selector"],
        ),
        tool(
            "mixum_snapshot_list",
            "List snapshots for an environment.",
            json!({"selector":{"type":"string"}}),
            vec!["selector"],
        ),
        tool(
            "mixum_snapshot_create",
            "Create a disk snapshot for an environment.",
            json!({"selector":{"type":"string"},"label":{"type":"string"},"kind":{"type":"string","enum":["disk","memory","hybrid"],"default":"disk"}}),
            vec!["selector"],
        ),
        tool(
            "mixum_window_screenshot",
            "Capture one window as a PNG image. Returns MCP image content; this is the agent's screen for computer use.",
            json!({"selector":{"type":"string"},"window":{"type":"integer"}}),
            vec!["selector","window"],
        ),
        tool(
            "mixum_window_click",
            "Click inside a container window using window-local coordinates.",
            json!({"selector":{"type":"string"},"window":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"},"button":{"type":"string","enum":["left","middle","right"],"default":"left"}}),
            vec!["selector","window","x","y"],
        ),
        tool(
            "mixum_window_move",
            "Move the pointer inside a container window.",
            json!({"selector":{"type":"string"},"window":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"}}),
            vec!["selector","window","x","y"],
        ),
        tool(
            "mixum_window_type",
            "Commit composed text into a container window (IME-safe; use for arbitrary Unicode).",
            json!({"selector":{"type":"string"},"window":{"type":"integer"},"text":{"type":"string"}}),
            vec!["selector","window","text"],
        ),
        tool(
            "mixum_window_key",
            "Send one keysym such as Return or a (with mods) to a container window.",
            json!({"selector":{"type":"string"},"window":{"type":"integer"},"keysym":{"type":"string"},"mods":{"type":"array","items":{"type":"string"}}}),
            vec!["selector","window","keysym"],
        ),
        tool(
            "mixum_window_scroll",
            "Scroll a container window at a point.",
            json!({"selector":{"type":"string"},"window":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"},"direction":{"type":"string","enum":["up","down"],"default":"down"}}),
            vec!["selector","window","x","y"],
        ),
        tool(
            "mixum_window_focus",
            "Focus a container window before typing or key input.",
            json!({"selector":{"type":"string"},"window":{"type":"integer"}}),
            vec!["selector","window"],
        ),
        tool(
            "mixum_snapshot_restore",
            "Restore a snapshot in place. Destructive: confirm must be exactly RESTORE.",
            json!({"selector":{"type":"string"},"node":{"type":"string"},"confirm":{"type":"string"}}),
            vec!["selector","node","confirm"],
        ),
        tool(
            "mixum_snapshot_fork",
            "Fork a snapshot into a new branch.",
            json!({"selector":{"type":"string"},"node":{"type":"string"},"branch":{"type":"string"}}),
            vec!["selector","node","branch"],
        ),
        tool(
            "mixum_snapshot_export",
            "Export a snapshot as a .mart bundle to a host path.",
            json!({"selector":{"type":"string"},"node":{"type":"string"},"out":{"type":"string"}}),
            vec!["selector","node","out"],
        ),
        tool(
            "mixum_snapshot_import",
            "Import a .mart bundle from a host path into an environment.",
            json!({"selector":{"type":"string"},"file":{"type":"string"},"origin":{"type":"string"}}),
            vec!["selector","file"],
        ),
        tool(
            "mixum_port_check",
            "Check whether a localhost port is free.",
            json!({"port":{"type":"integer","minimum":1,"maximum":65535}}),
            vec!["port"],
        ),
    ]
}

fn handle_tool(id: &Value, request: &Value) -> Value {
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
    if name == "mixum_window_screenshot" {
        return match call_tool(name, &arguments) {
            Ok(value) => {
                if let Some(data) = value.get("pngBase64").and_then(Value::as_str) {
                    json!({
                        "jsonrpc":"2.0",
                        "id":id,
                        "result": {
                            "content": [
                                {"type":"image","data":data,"mimeType":"image/png"},
                                {"type":"text","text":serde_json::to_string_pretty(&json!({
                                    "selector": arguments.get("selector"),
                                    "window": arguments.get("window"),
                                    "width": value.get("width"),
                                    "height": value.get("height"),
                                    "bytes": value.get("bytes")
                                })).unwrap_or_default()}
                            ],
                            "isError": false
                        }
                    })
                } else {
                    json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"screenshot returned no PNG"}],"isError":true}})
                }
            }
            Err(error) => json!({
                "jsonrpc":"2.0","id":id,
                "result":{"content":[{"type":"text","text":error}],"isError":true}
            }),
        };
    }
    match call_tool(name, &arguments) {
        Ok(value) => json!({
            "jsonrpc":"2.0",
            "id":id,
            "result": {
                "content": [{"type":"text","text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())}],
                "isError": false
            }
        }),
        Err(error) => json!({
            "jsonrpc":"2.0",
            "id":id,
            "result": {
                "content": [{"type":"text","text": error}],
                "isError": true
            }
        }),
    }
}

fn call_tool(name: &str, arguments: &Value) -> std::result::Result<Value, String> {
    let mut args: Vec<String> = vec!["--json".into()];
    match name {
        "mixum_env_list" => {
            args.extend(["env", "list"].map(String::from));
            if arguments
                .get("all")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                args.push("--all".into());
            }
        }
        "mixum_env_create" => {
            let title = arguments
                .get("title")
                .and_then(Value::as_str)
                .ok_or("title is required")?;
            let method = arguments
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or("subagent");
            let profile = arguments
                .get("profile")
                .and_then(Value::as_str)
                .unwrap_or("mixumos");
            args.extend([
                "env".into(),
                "create".into(),
                "--title".into(),
                title.into(),
                "--method".into(),
                method.into(),
                "--profile".into(),
                profile.into(),
            ]);
            if let Some(image) = arguments.get("image").and_then(Value::as_str) {
                args.extend(["--image".into(), image.into()]);
            }
            if let Some(runtime) = arguments.get("runtime").and_then(Value::as_str) {
                args.extend(["--runtime".into(), runtime.into()]);
            }
            if let Some(port) = arguments.get("port").and_then(Value::as_u64) {
                args.extend(["--port".into(), port.to_string()]);
            }
            if arguments
                .get("noStart")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                args.push("--no-start".into());
            }
            if let Some(wait) = arguments.get("waitMap").and_then(Value::as_u64) {
                args.extend(["--wait-map".into(), wait.to_string()]);
            }
        }
        "mixum_env_show" | "mixum_env_start" | "mixum_env_stop" | "mixum_snapshot_list" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            let sub = match name {
                "mixum_env_show" => vec!["env", "show"],
                "mixum_env_start" => vec!["env", "start"],
                "mixum_env_stop" => vec!["env", "stop"],
                _ => vec!["env", "snapshot", "list"],
            };
            args.extend(sub.iter().map(|item| item.to_string()));
            args.push(selector.into());
        }
        "mixum_env_windows" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            args.extend(["env".into(), "windows".into(), selector.into()]);
            if let Some(timeout) = arguments.get("timeout").and_then(Value::as_u64) {
                args.extend(["--timeout".into(), timeout.to_string()]);
            }
        }
        "mixum_snapshot_create" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            args.extend([
                "env".into(),
                "snapshot".into(),
                "create".into(),
                selector.into(),
            ]);
            if let Some(label) = arguments.get("label").and_then(Value::as_str) {
                args.extend(["--label".into(), label.into()]);
            }
            if let Some(kind) = arguments.get("kind").and_then(Value::as_str) {
                args.extend(["--kind".into(), kind.into()]);
            }
        }
        "mixum_snapshot_restore"
        | "mixum_snapshot_fork"
        | "mixum_snapshot_export"
        | "mixum_snapshot_import" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            let sub = match name {
                "mixum_snapshot_restore" => "restore",
                "mixum_snapshot_fork" => "fork",
                "mixum_snapshot_export" => "export",
                _ => "import",
            };
            args.extend(["env".into(), "snapshot".into(), sub.into(), selector.into()]);
            if name == "mixum_snapshot_import" {
                let file = arguments
                    .get("file")
                    .and_then(Value::as_str)
                    .ok_or("file is required")?;
                args.extend(["--file".into(), file.into()]);
                if let Some(origin) = arguments.get("origin").and_then(Value::as_str) {
                    args.extend(["--origin".into(), origin.into()]);
                }
            } else {
                let node = arguments
                    .get("node")
                    .and_then(Value::as_str)
                    .ok_or("node is required")?;
                args.push(node.into());
                match name {
                    "mixum_snapshot_restore" => {
                        if arguments.get("confirm").and_then(Value::as_str) != Some("RESTORE") {
                            return Err("confirm must be exactly RESTORE".into());
                        }
                        args.push("--yes".into());
                    }
                    "mixum_snapshot_fork" => {
                        let branch = arguments
                            .get("branch")
                            .and_then(Value::as_str)
                            .ok_or("branch is required")?;
                        args.extend(["--branch".into(), branch.into()]);
                    }
                    "mixum_snapshot_export" => {
                        let out = arguments
                            .get("out")
                            .and_then(Value::as_str)
                            .ok_or("out is required")?;
                        args.extend(["--out".into(), out.into()]);
                    }
                    _ => {}
                }
            }
        }
        "mixum_window_screenshot" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            let window = arguments
                .get("window")
                .and_then(Value::as_u64)
                .ok_or("window is required")?;
            args.extend([
                "env".into(),
                "screenshot".into(),
                selector.into(),
                "--window".into(),
                window.to_string(),
            ]);
        }
        "mixum_window_click" | "mixum_window_move" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            let window = arguments
                .get("window")
                .and_then(Value::as_u64)
                .ok_or("window is required")?;
            let x = arguments
                .get("x")
                .and_then(Value::as_f64)
                .ok_or("x is required")?;
            let y = arguments
                .get("y")
                .and_then(Value::as_f64)
                .ok_or("y is required")?;
            let sub = if name == "mixum_window_click" {
                "click"
            } else {
                "move"
            };
            args.extend([
                "env".into(),
                sub.into(),
                selector.into(),
                "--window".into(),
                window.to_string(),
                "--x".into(),
                x.to_string(),
                "--y".into(),
                y.to_string(),
            ]);
            if name == "mixum_window_click" {
                if let Some(button) = arguments.get("button").and_then(Value::as_str) {
                    args.extend(["--button".into(), button.into()]);
                }
            }
        }
        "mixum_window_type" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            let window = arguments
                .get("window")
                .and_then(Value::as_u64)
                .ok_or("window is required")?;
            let text = arguments
                .get("text")
                .and_then(Value::as_str)
                .ok_or("text is required")?;
            args.extend([
                "env".into(),
                "type".into(),
                selector.into(),
                "--window".into(),
                window.to_string(),
                text.into(),
            ]);
        }
        "mixum_window_key" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            let window = arguments
                .get("window")
                .and_then(Value::as_u64)
                .ok_or("window is required")?;
            let keysym = arguments
                .get("keysym")
                .and_then(Value::as_str)
                .ok_or("keysym is required")?;
            args.extend([
                "env".into(),
                "key".into(),
                selector.into(),
                "--window".into(),
                window.to_string(),
                keysym.into(),
            ]);
            if let Some(mods) = arguments.get("mods").and_then(Value::as_array) {
                let mods = mods
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",");
                if !mods.is_empty() {
                    args.extend(["--mods".into(), mods]);
                }
            }
        }
        "mixum_window_scroll" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            let window = arguments
                .get("window")
                .and_then(Value::as_u64)
                .ok_or("window is required")?;
            let x = arguments
                .get("x")
                .and_then(Value::as_f64)
                .ok_or("x is required")?;
            let y = arguments
                .get("y")
                .and_then(Value::as_f64)
                .ok_or("y is required")?;
            let direction = arguments
                .get("direction")
                .and_then(Value::as_str)
                .unwrap_or("down");
            args.extend([
                "env".into(),
                "scroll".into(),
                selector.into(),
                "--window".into(),
                window.to_string(),
                "--x".into(),
                x.to_string(),
                "--y".into(),
                y.to_string(),
                "--direction".into(),
                direction.into(),
            ]);
        }
        "mixum_window_focus" => {
            let selector = arguments
                .get("selector")
                .and_then(Value::as_str)
                .ok_or("selector is required")?;
            let window = arguments
                .get("window")
                .and_then(Value::as_u64)
                .ok_or("window is required")?;
            args.extend([
                "env".into(),
                "focus".into(),
                selector.into(),
                "--window".into(),
                window.to_string(),
            ]);
        }
        "mixum_port_check" => {
            let port = arguments
                .get("port")
                .and_then(Value::as_u64)
                .ok_or("port is required")?;
            args.extend(["port".into(), "check".into(), port.to_string()]);
        }
        _ => return Err(format!("unknown tool {name}")),
    }
    run_cli(&args)
}

fn run_cli(args: &[String]) -> std::result::Result<Value, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("mixum executable: {error}"))?;
    let output = Command::new(executable)
        .args(args)
        .output()
        .map_err(|error| format!("run mixum: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        if let Ok(value) = serde_json::from_str::<Value>(&stdout) {
            if let Some(message) = value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
            {
                return Err(message.to_string());
            }
        }
        return Err(format!(
            "mixum exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_str(&stdout).map_err(|error| format!("mixum returned invalid JSON: {error}"))
}
