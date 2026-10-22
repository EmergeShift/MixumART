//! mixum-artd — host-side Unix socket API for MixumART.
//!
//! The daemon deliberately shells out to the sibling `mixum` binary with
//! `--json`. That keeps state, port allocation, MAP pairing and error codes in
//! one implementation instead of duplicating the CLI rules in a server.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use clap::Parser;
use serde_json::{json, Value};

#[derive(Debug, Parser)]
#[command(name = "mixum-artd", version, about = "MixumART host control socket")]
struct Args {
    /// Unix socket path. Defaults to <data-dir>/artd.sock.
    #[arg(long)]
    socket: Option<PathBuf>,
    /// State directory used by the CLI.
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();
    if let Err(error) = run(args) {
        eprintln!("mixum-artd: {error}");
        std::process::exit(1);
    }
}

fn run(args: Args) -> Result<(), String> {
    let data_dir = match args.data_dir {
        Some(dir) => dir,
        None => dirs::data_dir()
            .ok_or("cannot determine user data directory")?
            .join("dev.mixum.agentenv")
            .join("mixum-cli"),
    };
    fs::create_dir_all(&data_dir).map_err(|error| format!("create data dir: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&data_dir, fs::Permissions::from_mode(0o700));
    }
    let socket = args.socket.unwrap_or_else(|| data_dir.join("artd.sock"));
    if socket.exists() {
        // A stale socket is common after a crash. Only remove a socket, never a
        // regular file the user might have placed there.
        let metadata =
            fs::symlink_metadata(&socket).map_err(|error| format!("inspect socket: {error}"))?;
        if !metadata.file_type().is_socket() {
            return Err(format!("{} exists and is not a socket", socket.display()));
        }
        fs::remove_file(&socket).map_err(|error| format!("remove stale socket: {error}"))?;
    }
    let listener = UnixListener::bind(&socket)
        .map_err(|error| format!("bind {}: {error}", socket.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("chmod socket: {error}"))?;
    }
    eprintln!("mixum-artd listening on {}", socket.display());
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let data_dir = data_dir.clone();
                thread::spawn(move || {
                    if let Err(error) = handle(stream, &data_dir) {
                        eprintln!("mixum-artd client: {error}");
                    }
                });
            }
            Err(error) => eprintln!("mixum-artd accept: {error}"),
        }
    }
    Ok(())
}

fn handle(stream: UnixStream, data_dir: &Path) -> Result<(), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|error| error.to_string())?);
    let mut writer = stream;
    let mut line = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|error| format!("read request: {error}"))?;
        if read == 0 {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle_request(request, data_dir),
            Err(error) => json!({
                "id": Value::Null,
                "ok": false,
                "error": {"code": 2, "message": format!("invalid JSON: {error}")}
            }),
        };
        let text = serde_json::to_string(&response).map_err(|error| error.to_string())?;
        writeln!(writer, "{text}").map_err(|error| format!("write response: {error}"))?;
        writer
            .flush()
            .map_err(|error| format!("flush response: {error}"))?;
    }
}

fn handle_request(request: Value, data_dir: &Path) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(json!({}));
    match call(method, &params, data_dir) {
        Ok(result) => json!({"id": id, "ok": true, "result": result}),
        Err((code, message)) => {
            json!({"id": id, "ok": false, "error": {"code": code, "message": message}})
        }
    }
}

fn call(method: &str, params: &Value, data_dir: &Path) -> Result<Value, (i32, String)> {
    let mut args: Vec<String> = vec![
        "--json".into(),
        "--data-dir".into(),
        data_dir.display().to_string(),
    ];
    match method {
        "doctor" => args.push("doctor".into()),
        "runtime.list" => args.extend(["runtime", "list"].map(String::from)),
        "env.list" => {
            args.extend(["env", "list"].map(String::from));
            if params.get("all").and_then(Value::as_bool).unwrap_or(false) {
                args.push("--all".into());
            }
        }
        "env.create" => {
            let title = string_param(params, "title")?;
            args.extend(["env".into(), "create".into(), "--title".into(), title]);
            if let Some(method) = optional_string(params, "method") {
                args.extend(["--method".into(), method]);
            }
            if let Some(profile) = optional_string(params, "profile") {
                args.extend(["--profile".into(), profile]);
            }
            if let Some(image) = optional_string(params, "image") {
                args.extend(["--image".into(), image]);
            }
            if let Some(runtime) = optional_string(params, "runtime") {
                args.extend(["--runtime".into(), runtime]);
            }
            if let Some(port) = params.get("port").and_then(Value::as_u64) {
                args.extend(["--port".into(), port.to_string()]);
            }
            if params
                .get("noStart")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                args.push("--no-start".into());
            }
            if let Some(wait) = params.get("waitMap").and_then(Value::as_u64) {
                args.extend(["--wait-map".into(), wait.to_string()]);
            }
        }
        "env.show" | "env.start" | "env.stop" => {
            let selector = string_param(params, "selector")?;
            let sub = method.strip_prefix("env.").unwrap_or("show");
            args.extend(["env".into(), sub.into(), selector]);
        }
        "env.remove" => {
            let selector = string_param(params, "selector")?;
            args.extend(["env".into(), "remove".into(), selector]);
            if params
                .get("force")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                args.push("--force".into());
            }
        }
        "env.windows" => {
            let selector = string_param(params, "selector")?;
            args.extend(["env".into(), "windows".into(), selector]);
            if let Some(timeout) = params.get("timeout").and_then(Value::as_u64) {
                args.extend(["--timeout".into(), timeout.to_string()]);
            }
        }
        "env.logs" => {
            let selector = string_param(params, "selector")?;
            args.extend(["env".into(), "logs".into(), selector]);
            if let Some(tail) = params.get("tail").and_then(Value::as_u64) {
                args.extend(["--tail".into(), tail.to_string()]);
            }
        }
        "snapshot.list" => {
            args.extend(["env", "snapshot", "list"].map(String::from));
            args.push(string_param(params, "selector")?);
        }
        "snapshot.create" => {
            args.extend(["env", "snapshot", "create"].map(String::from));
            args.push(string_param(params, "selector")?);
            if let Some(label) = optional_string(params, "label") {
                args.extend(["--label".into(), label]);
            }
        }
        "port.check" => {
            let port = params
                .get("port")
                .and_then(Value::as_u64)
                .ok_or((2, "port is required".into()))?;
            args.extend(["port".into(), "check".into(), port.to_string()]);
        }
        "port.list" => args.extend(["port", "list"].map(String::from)),
        _ => return Err((2, format!("unknown method {method}"))),
    }
    run_cli(&args)
}

fn string_param(params: &Value, name: &str) -> Result<String, (i32, String)> {
    params
        .get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or((2, format!("{name} is required")))
}

fn optional_string(params: &Value, name: &str) -> Option<String> {
    params
        .get(name)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn run_cli(args: &[String]) -> Result<Value, (i32, String)> {
    let executable = std::env::current_exe()
        .map_err(|error| (1, format!("resolve mixum-artd path: {error}")))?;
    let mixum = executable
        .parent()
        .map(|directory| directory.join("mixum"))
        .filter(|path| path.exists())
        .or_else(|| std::env::var_os("MIXUM_BIN").map(PathBuf::from))
        .ok_or((1, "cannot find sibling mixum binary; set MIXUM_BIN".into()))?;
    let output = Command::new(mixum)
        .args(args)
        .output()
        .map_err(|error| (1, format!("run mixum: {error}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let value = serde_json::from_str::<Value>(&stdout).map_err(|error| {
        (
            1,
            format!(
                "mixum returned invalid JSON ({error}); stderr: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )
    })?;
    if !output.status.success() {
        let code = value
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_i64)
            .unwrap_or(1) as i32;
        let message = value
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("mixum failed")
            .to_string();
        return Err((code, message));
    }
    Ok(value)
}
