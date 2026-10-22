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
            json!({"selector":{"type":"string"},"label":{"type":"string"}}),
            vec!["selector"],
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
