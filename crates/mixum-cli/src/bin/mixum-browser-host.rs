//! Chrome native-messaging host for the Mixum browser bridge.
//!
//! Chrome starts this process and speaks length-prefixed JSON on stdin/stdout.
//! A local Unix socket lets `mixum browser export` ask the same host for a
//! user-approved cookie/tab lease. Nothing is written to disk except the socket.

use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

const MAX_MESSAGE: usize = 16 * 1024 * 1024;

type Pending = Arc<Mutex<HashMap<String, mpsc::Sender<Value>>>>;
type Writer = Arc<Mutex<io::Stdout>>;

fn main() {
    let socket = socket_path();
    if let Some(parent) = socket.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::remove_file(&socket);
    let listener = match UnixListener::bind(&socket) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("mixum-browser-host: bind {}: {error}", socket.display());
            std::process::exit(1);
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600));
    }
    eprintln!("mixum-browser-host: listening on {}", socket.display());

    let writer: Writer = Arc::new(Mutex::new(io::stdout()));
    let pending: Pending = Arc::new(Mutex::new(HashMap::new()));

    // Native-message reader (Chrome -> host).
    let read_pending = Arc::clone(&pending);
    std::thread::spawn(move || {
        let mut stdin = io::stdin();
        while let Ok(message) = read_native(&mut stdin) {
            let id = message
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if id.is_empty() {
                continue;
            }
            if let Some(sender) = read_pending.lock().unwrap().remove(&id) {
                let _ = sender.send(message);
            }
        }
        std::process::exit(0);
    });

    // Local CLI requests (Unix socket -> host -> extension).
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let writer = Arc::clone(&writer);
                let pending = Arc::clone(&pending);
                std::thread::spawn(move || handle_client(stream, writer, pending));
            }
            Err(error) => eprintln!("mixum-browser-host: accept: {error}"),
        }
    }
}

fn handle_client(mut stream: UnixStream, writer: Writer, pending: Pending) {
    let mut line = String::new();
    let mut reader = io::BufReader::new(&mut stream);
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let request: Value = match serde_json::from_str(line.trim()) {
        Ok(request) => request,
        Err(error) => {
            let _ = writeln!(
                stream,
                "{}",
                json!({"ok": false, "error": format!("bad request: {error}")})
            );
            return;
        }
    };
    let id = format!("req-{}", ulid::Ulid::new());
    let (tx, rx) = mpsc::channel();
    pending.lock().unwrap().insert(id.clone(), tx);
    let message = json!({
        "id": id,
        "type": request.get("type").and_then(Value::as_str).unwrap_or("export"),
        "domains": request.get("domains").cloned().unwrap_or_else(|| json!([])),
    });
    if let Err(error) = write_native(&writer, &message) {
        pending.lock().unwrap().remove(&id);
        let _ = writeln!(
            stream,
            "{}",
            json!({"ok": false, "error": format!("native write: {error}")})
        );
        return;
    }
    match rx.recv_timeout(Duration::from_secs(15)) {
        Ok(response) => {
            let _ = writeln!(stream, "{response}");
        }
        Err(_) => {
            pending.lock().unwrap().remove(&id);
            let _ = writeln!(
                stream,
                "{}",
                json!({"ok": false, "error": "Chrome extension did not answer; open Chrome and check the Mixum Browser Bridge extension"})
            );
        }
    }
}

fn write_native(writer: &Writer, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    let mut stdout = writer.lock().unwrap();
    stdout.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    stdout.write_all(&bytes)?;
    stdout.flush()
}

fn read_native(reader: &mut impl Read) -> io::Result<Value> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_ne_bytes(len) as usize;
    if len == 0 || len > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid native message length",
        ));
    }
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MIXUM_BROWSER_SOCKET") {
        return PathBuf::from(path);
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("dev.mixum.agentenv")
        .join("mixum-cli")
        .join("browser.sock")
}
