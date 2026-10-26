//! MixumART MAP client: WSS + TOFU, display.list/attach/input and PNG frames.
//!
//! This is the host side of the B route. It deliberately mirrors the small
//! subset of MAP that the Tauri surface windows need; the protocol source of
//! truth remains Mixum-Agent-Protocol.

use std::{
    collections::HashMap,
    fmt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use mixum_env_surface::{Frame, SurfaceInfo, FORMAT_PNG};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{verify_tls12_signature, verify_tls13_signature, WebPkiSupportedAlgorithms},
    pki_types::{CertificateDer, ServerName, UnixTime},
    DigitallySignedStruct, Error as RustlsError, SignatureScheme,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc, oneshot, Notify};
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
    Connector,
};
use ulid::Ulid;

const CLIENT_NAME: &str = "mixumart/0.1";
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum MapError {
    #[error("连接地址无效: {0}")]
    Url(#[from] url::ParseError),
    #[error("WebSocket: {0}")]
    WebSocket(String),
    #[error("协议: {0}")]
    Protocol(String),
    #[error("服务器错误 {code}: {message}")]
    Remote { code: String, message: String },
    #[error("请求超时: {0}")]
    Timeout(String),
    #[error("配置: {0}")]
    Config(String),
    #[error("IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    v: u32,
    id: String,
    op: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reply_to: Option<String>,
    /// RFC 0003:gateway 路由目标;直连节点忽略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    node: Option<String>,
    ts: i64,
    #[serde(default)]
    payload: Value,
}

impl Envelope {
    fn request(op: &str, payload: Value, node: Option<&str>) -> Self {
        Self {
            v: 1,
            id: Ulid::new().to_string(),
            op: op.to_owned(),
            seq: None,
            reply_to: None,
            node: node.map(ToOwned::to_owned),
            ts: now_ms(),
            payload,
        }
    }
}

/// 配对码换取的设备凭据(RFC 0003)。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairClaim {
    pub token: String,
    pub device_id: String,
}

/// gateway.device.list 的一条设备记录(owner 隔离由服务端保证)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub jti: String,
    #[serde(default)]
    pub name: String,
    pub sub: String,
    pub owner: String,
    #[serde(default)]
    pub iat: i64,
}

fn remote_error(reply: &Envelope) -> MapError {
    let code = reply
        .payload
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("error")
        .to_owned();
    let message = reply
        .payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    MapError::Remote { code, message }
}

fn parse_pair_claim(reply: Envelope) -> Result<PairClaim, MapError> {
    if reply.op.ends_with(".err") {
        return Err(remote_error(&reply));
    }
    let token = reply
        .payload
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| MapError::Protocol("pair.claim 未返回 token".into()))?
        .to_owned();
    let device_id = reply
        .payload
        .get("deviceId")
        .and_then(Value::as_str)
        .ok_or_else(|| MapError::Protocol("pair.claim 未返回 deviceId".into()))?
        .to_owned();
    Ok(PairClaim { token, device_id })
}

fn parse_nodes(reply: &Envelope) -> Vec<String> {
    reply
        .payload
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WindowInfo {
    id: u32,
    #[serde(default)]
    app_id: String,
    #[serde(default)]
    title: String,
    width: u32,
    height: u32,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default)]
    windows: Vec<WindowInfo>,
}

#[derive(Debug, Clone)]
pub struct PushEvent {
    pub op: String,
    pub payload: Value,
}

#[derive(Clone)]
pub struct MapClient {
    tx: mpsc::Sender<Message>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Envelope>>>>,
    frames: broadcast::Sender<Frame>,
    events: broadcast::Sender<PushEvent>,
    /// 进行中的 file.get：transfer id → 累积缓冲。
    files: Arc<Mutex<HashMap<u32, FileTransfer>>>,
    /// RFC 0003:gateway 路由目标;None = 直连节点或由 gateway 默认路由。
    node: Arc<Mutex<Option<String>>>,
    /// hello.ok 的 server 字段表明对端是 gateway(而非直连 agentd)。
    gateway: Arc<AtomicBool>,
    pub fingerprint: String,
}

struct FileTransfer {
    data: Vec<u8>,
    expected: u64,
    done: bool,
    notify: Arc<Notify>,
}

impl fmt::Debug for MapClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MapClient")
            .field("fingerprint", &self.fingerprint)
            .finish_non_exhaustive()
    }
}

type MapSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// 建立 WSS + 指纹固定连接,返回 socket 与观测到的服务器指纹。
async fn open_socket(
    addr: &str,
    fingerprint: Option<&str>,
) -> Result<(MapSocket, String), MapError> {
    let address = ws_addr(addr)?;
    let known_path = known_hosts_path();
    let known = load_known(&known_path)?;
    let explicit_fingerprint = fingerprint.is_some();
    let expected = fingerprint
        .map(ToOwned::to_owned)
        .or_else(|| known.get(addr).cloned());
    let first_connection = expected.is_none();
    let observed = Arc::new(Mutex::new(None));
    let provider = rustls::crypto::ring::default_provider();
    let verifier = PinVerifier {
        expected,
        observed: Arc::clone(&observed),
        supported_algorithms: provider.signature_verification_algorithms,
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| MapError::Config(error.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let connector = Connector::Rustls(Arc::new(config));
    let mut request = address
        .into_client_request()
        .map_err(|error| MapError::WebSocket(error.to_string()))?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_static("mixum.v1"),
    );
    let (stream, _) = connect_async_tls_with_config(request, None, false, Some(connector))
        .await
        .map_err(|error| MapError::WebSocket(error.to_string()))?;
    let fingerprint = observed
        .lock()
        .expect("fingerprint lock")
        .clone()
        .ok_or_else(|| MapError::Config("TLS 未返回服务器证书".into()))?;
    if first_connection {
        eprintln!("首次连接 {addr}，服务器证书指纹：{fingerprint}");
    }
    if !known.contains_key(addr) || explicit_fingerprint {
        save_known(&known_path, addr, &fingerprint)?;
    }
    Ok((stream, fingerprint))
}

impl MapClient {
    /// Connect, pin/verify the server certificate, then perform session.hello.
    pub async fn connect(
        addr: &str,
        token: &str,
        fingerprint: Option<&str>,
    ) -> Result<Self, MapError> {
        let (stream, fingerprint) = open_socket(addr, fingerprint).await?;
        let (mut write, mut read) = stream.split();
        let (tx, mut outgoing) = mpsc::channel::<Message>(64);
        let (frame_tx, _) = broadcast::channel(128);
        let (event_tx, _) = broadcast::channel(256);
        let pending: Arc<Mutex<HashMap<String, oneshot::Sender<Envelope>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let reader_pending = Arc::clone(&pending);
        let reader_frames = frame_tx.clone();
        let reader_events = event_tx.clone();
        let reader_files: Arc<Mutex<HashMap<u32, FileTransfer>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let files_for_reader = Arc::clone(&reader_files);
        tokio::spawn(async move {
            while let Some(message) = read.next().await {
                match message {
                    Ok(Message::Text(text)) => {
                        let Ok(envelope) = serde_json::from_str::<Envelope>(&text) else {
                            continue;
                        };
                        if envelope.op == "file.get.done" {
                            if let Some(transfer) =
                                envelope.payload.get("transfer").and_then(Value::as_u64)
                            {
                                let mut files = files_for_reader.lock().expect("file lock");
                                let entry =
                                    files
                                        .entry(transfer as u32)
                                        .or_insert_with(|| FileTransfer {
                                            data: Vec::new(),
                                            expected: 0,
                                            done: false,
                                            notify: Arc::new(Notify::new()),
                                        });
                                entry.done = true;
                                entry.notify.notify_waiters();
                            }
                            continue;
                        }
                        if let Some(reply_to) = envelope.reply_to.clone() {
                            if let Some(sender) = reader_pending
                                .lock()
                                .expect("pending lock")
                                .remove(&reply_to)
                            {
                                let _ = sender.send(envelope);
                                continue;
                            }
                        }
                        let _ = reader_events.send(PushEvent {
                            op: envelope.op,
                            payload: envelope.payload,
                        });
                    }
                    Ok(Message::Binary(data)) => {
                        if let Some((transfer, offset, chunk)) = parse_file_chunk(&data) {
                            let mut files = files_for_reader.lock().expect("file lock");
                            let entry = files.entry(transfer).or_insert_with(|| FileTransfer {
                                data: Vec::new(),
                                expected: 0,
                                done: false,
                                notify: Arc::new(Notify::new()),
                            });
                            if offset as usize == entry.data.len() {
                                entry.data.extend_from_slice(chunk);
                                entry.notify.notify_one();
                            }
                        } else if let Some(frame) = parse_video(&data) {
                            let _ = reader_frames.send(frame);
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
        });
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                if write.send(message).await.is_err() {
                    break;
                }
            }
        });

        let client = Self {
            tx,
            pending,
            frames: frame_tx,
            events: event_tx,
            files: reader_files,
            node: Arc::new(Mutex::new(None)),
            gateway: Arc::new(AtomicBool::new(false)),
            fingerprint,
        };
        let hello = client
            .call(
                "session.hello",
                json!({ "token": token, "client": CLIENT_NAME }),
            )
            .await?;
        if hello.op != "session.hello.ok" {
            return Err(MapError::Protocol(format!("handshake: {}", hello.op)));
        }
        if hello
            .payload
            .get("server")
            .and_then(Value::as_str)
            .is_some_and(|server| server.starts_with("mixum-gateway"))
        {
            client.gateway.store(true, Ordering::Relaxed);
        }
        Ok(client)
    }

    pub fn frames(&self) -> broadcast::Receiver<Frame> {
        self.frames.subscribe()
    }

    /// 对端是 gateway 时,控制面 op(node.list/pair/device)才可用。
    pub fn is_gateway(&self) -> bool {
        self.gateway.load(Ordering::Relaxed)
    }

    /// 设置后续信封的 gateway 路由目标(None = 让 gateway 默认路由)。
    pub fn set_node(&self, node: Option<String>) {
        *self.node.lock().expect("node lock") = node;
    }

    pub fn node(&self) -> Option<String> {
        self.node.lock().expect("node lock").clone()
    }

    /// gateway.node.list → 在线节点名(RFC 0003)。
    pub async fn node_list(&self) -> Result<Vec<String>, MapError> {
        let reply = self.call("gateway.node.list", json!({})).await?;
        Ok(parse_nodes(&reply))
    }

    /// 用户会话生成一次性配对码,返回 (code, expiresIn 秒)。
    pub async fn pair_create(&self) -> Result<(String, u64), MapError> {
        let reply = self.call("gateway.pair.create", json!({})).await?;
        let code = reply
            .payload
            .get("code")
            .and_then(Value::as_str)
            .ok_or_else(|| MapError::Protocol("pair.create 未返回 code".into()))?
            .to_owned();
        let expires = reply
            .payload
            .get("expiresIn")
            .and_then(Value::as_u64)
            .unwrap_or(300);
        Ok((code, expires))
    }

    /// 设备列表(owner 隔离由服务端保证)。
    pub async fn device_list(&self) -> Result<Vec<DeviceInfo>, MapError> {
        let reply = self.call("gateway.device.list", json!({})).await?;
        let devices = reply
            .payload
            .get("devices")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        Ok(serde_json::from_value(devices)?)
    }

    /// 吊销设备令牌(owner 或 admin)。
    pub async fn device_revoke(&self, jti: &str) -> Result<bool, MapError> {
        let reply = self
            .call("gateway.device.revoke", json!({ "jti": jti }))
            .await?;
        Ok(reply
            .payload
            .get("revoked")
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// RFC 0003:未认证设备用配对码换设备令牌(首条消息),随后用该 token connect。
    pub async fn pair_claim(
        addr: &str,
        code: &str,
        device_name: &str,
        fingerprint: Option<&str>,
    ) -> Result<PairClaim, MapError> {
        let (mut stream, _fingerprint) = open_socket(addr, fingerprint).await?;
        let request = Envelope::request(
            "gateway.pair.claim",
            json!({ "code": code, "deviceName": device_name }),
            None,
        );
        stream
            .send(Message::Text(serde_json::to_string(&request)?.into()))
            .await
            .map_err(|error| MapError::WebSocket(error.to_string()))?;
        let reply = tokio::time::timeout(CLIENT_TIMEOUT, async {
            loop {
                match stream.next().await {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(envelope) = serde_json::from_str::<Envelope>(&text) else {
                            continue;
                        };
                        if envelope.reply_to.as_deref() == Some(request.id.as_str()) {
                            break Ok(envelope);
                        }
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => break Err(MapError::WebSocket(error.to_string())),
                    None => break Err(MapError::WebSocket("连接在应答前关闭".into())),
                }
            }
        })
        .await
        .map_err(|_| MapError::Timeout("gateway.pair.claim".into()))??;
        let _ = stream.close(None).await;
        parse_pair_claim(reply)
    }

    pub async fn list(&self) -> Result<Vec<SurfaceInfo>, MapError> {
        let reply = self.call("display.list", json!({})).await?;
        let response: ListResponse = serde_json::from_value(reply.payload)?;
        Ok(response
            .windows
            .into_iter()
            .map(|window| SurfaceInfo {
                id: window.id,
                app_id: window.app_id,
                title: window.title,
                width: window.width,
                height: window.height,
            })
            .collect())
    }

    pub async fn attach(&self, surface: u32) -> Result<(), MapError> {
        self.call(
            "display.attach",
            json!({ "window": surface, "fps": 10, "codec": "png" }),
        )
        .await?;
        Ok(())
    }

    /// Pending requests, for clients that connect after the broadcast.
    pub async fn privilege_list(&self) -> Result<Vec<Value>, MapError> {
        let reply = self.call("privilege.list", json!({})).await?;
        Ok(reply
            .payload
            .get("requests")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Approve or deny one pending host privilege request.
    pub async fn privilege_decision(
        &self,
        id: &str,
        approve: bool,
        reason: &str,
    ) -> Result<(), MapError> {
        self.call(
            "privilege.decision",
            json!({ "id": id, "approve": approve, "reason": reason, "scope": "once" }),
        )
        .await?;
        Ok(())
    }

    /// Pull the node's current clipboard text (B2 remote selection).
    pub async fn detach(&self, surface: u32) -> Result<(), MapError> {
        self.call("display.detach", json!({ "window": surface }))
            .await?;
        Ok(())
    }

    /// Escape hatch for MAP input variants this client does not model yet.
    pub async fn input_raw(&self, surface: u32, mut payload: Value) -> Result<(), MapError> {
        if let Value::Object(map) = &mut payload {
            map.insert("window".into(), json!(surface));
        }
        self.call("display.input", payload).await?;
        Ok(())
    }

    pub async fn clipboard(&self) -> Result<String, MapError> {
        let reply = self.call("display.clipboard", json!({})).await?;
        let text = reply
            .payload
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(text)
    }

    /// Subscribe to push envelopes (task events, window events, ...).
    pub fn events(&self) -> broadcast::Receiver<PushEvent> {
        self.events.subscribe()
    }

    pub async fn subscribe_task_events(&self) -> Result<(), MapError> {
        self.call("task.events", json!({})).await?;
        Ok(())
    }

    /// Fetch a task snapshot (`task.get`), used as a completion fallback when a
    /// very fast task's state event races the event subscription.
    pub async fn task_get(&self, id: &str) -> Result<Value, MapError> {
        Ok(self.call("task.get", json!({ "id": id })).await?.payload)
    }

    /// Run a command through agentd's shell task driver; returns the task id.
    pub async fn submit_shell_task(&self, command: &str) -> Result<String, MapError> {
        let reply = self
            .call(
                "task.submit",
                json!({
                    "instruction": command,
                    "agent": "shell",
                    "params": { "cmd": command }
                }),
            )
            .await?;
        reply
            .payload
            .get("id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| MapError::Protocol("task.submit 未返回 id".into()))
    }

    /// Snapshot tree for a connected node (`snap.tree`).
    pub async fn snapshot_tree(&self) -> Result<Value, MapError> {
        Ok(self.call("snap.tree", json!({})).await?.payload)
    }

    /// Create a disk snapshot. `label` is optional; `auto=false` because this
    /// is an explicit user action.
    pub async fn snapshot_create(&self, label: Option<&str>) -> Result<Value, MapError> {
        self.snapshot_create_kind("disk", label).await
    }

    /// 创建指定类型的快照（disk / memory / hybrid）。
    pub async fn snapshot_create_kind(
        &self,
        kind: &str,
        label: Option<&str>,
    ) -> Result<Value, MapError> {
        let mut payload = json!({ "kind": kind, "auto": false });
        if let Some(label) = label.filter(|label| !label.trim().is_empty()) {
            payload["label"] = json!(label);
        }
        Ok(self.call("snap.create", payload).await?.payload)
    }

    /// Restore a snapshot in place. Callers must confirm because this is
    /// destructive for the current worktree.
    pub async fn snapshot_restore(&self, node: &str) -> Result<Value, MapError> {
        self.call(
            "snap.restore",
            json!({ "node": node, "strategy": "inplace" }),
        )
        .await
        .map(|reply| reply.payload)
    }

    /// Fork a snapshot into a new branch.
    pub async fn snapshot_fork(&self, node: &str, branch: &str) -> Result<Value, MapError> {
        self.call("snap.fork", json!({ "node": node, "branch": branch }))
            .await
            .map(|reply| reply.payload)
    }

    /// 导出节点为 `.mart`（mixumOS 扩展 op）。返回 `{file,size,sha256}`。
    pub async fn snapshot_export(&self, node: &str) -> Result<Value, MapError> {
        self.call("snap.export", json!({ "node": node, "format": "mart" }))
            .await
            .map(|reply| reply.payload)
    }

    /// 从 transfers 里的 `.mart` 导入节点。返回 `{node}`。
    pub async fn snapshot_import(&self, file: &str, origin: &str) -> Result<Value, MapError> {
        self.call("snap.import", json!({ "file": file, "origin": origin }))
            .await
            .map(|reply| reply.payload)
    }

    /// 下载 work_root 下的文件（file.get 流）。用于迁移包。
    pub async fn get_file(&self, path: &str) -> Result<Vec<u8>, MapError> {
        let reply = self.call("file.get", json!({ "path": path })).await?;
        let transfer = reply
            .payload
            .get("transfer")
            .and_then(Value::as_u64)
            .ok_or_else(|| MapError::Protocol("file.get 未返回 transfer".into()))?
            as u32;
        let size = reply
            .payload
            .get("size")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let notify = {
            let mut files = self.files.lock().expect("file lock");
            let entry = files.entry(transfer).or_insert_with(|| FileTransfer {
                data: Vec::new(),
                expected: 0,
                done: false,
                notify: Arc::new(Notify::new()),
            });
            entry.expected = size;
            entry.notify.clone()
        };
        let deadline = tokio::time::Instant::now() + Duration::from_secs(300);
        loop {
            {
                let mut files = self.files.lock().expect("file lock");
                if let Some(entry) = files.get(&transfer) {
                    if entry.done {
                        let entry = files.remove(&transfer).expect("entry checked");
                        if entry.data.len() as u64 != entry.expected {
                            return Err(MapError::Protocol("file.get 大小不符".into()));
                        }
                        return Ok(entry.data);
                    }
                }
            }
            if tokio::time::timeout_at(deadline, notify.notified())
                .await
                .is_err()
            {
                self.files.lock().expect("file lock").remove(&transfer);
                return Err(MapError::Timeout("file.get".into()));
            }
        }
    }

    /// Claim/release a surface for the user. `force` is needed when an agent
    /// currently owns the surface.
    pub async fn takeover(
        &self,
        surface: u32,
        action: &str,
        force: bool,
    ) -> Result<String, MapError> {
        let reply = self
            .call(
                "display.takeover",
                json!({ "window": surface, "action": action, "force": force }),
            )
            .await?;
        reply
            .payload
            .get("mode")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| MapError::Protocol("display.takeover 未返回 mode".into()))
    }

    pub async fn input(
        &self,
        surface: u32,
        event: &mixum_env_surface::InputEvent,
    ) -> Result<(), MapError> {
        let mut payload = serde_json::to_value(event)?;
        if let Value::Object(map) = &mut payload {
            map.insert("window".into(), json!(surface));
        }
        self.call("display.input", payload).await?;
        Ok(())
    }

    /// Upload a file into the node's work_root/transfers directory.
    pub async fn put_file(&self, name: &str, data: &[u8]) -> Result<String, MapError> {
        let reply = self
            .call("file.put", json!({ "name": name, "size": data.len() }))
            .await?;
        let transfer = reply
            .payload
            .get("transfer")
            .and_then(Value::as_u64)
            .ok_or_else(|| MapError::Protocol("file.put 未返回 transfer".into()))?
            as u32;
        let mut offset = 0u64;
        for chunk in data.chunks(256 * 1024) {
            let mut frame = Vec::with_capacity(13 + chunk.len());
            frame.push(0x05);
            frame.extend_from_slice(&transfer.to_le_bytes());
            frame.extend_from_slice(&offset.to_le_bytes());
            frame.extend_from_slice(chunk);
            self.tx
                .send(Message::Binary(frame.into()))
                .await
                .map_err(|_| MapError::WebSocket("文件块发送失败".into()))?;
            offset += chunk.len() as u64;
        }
        let digest = Sha256::digest(data);
        let hash = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let reply = self
            .call(
                "file.put.commit",
                json!({ "transfer": transfer, "sha256": hash }),
            )
            .await?;
        reply
            .payload
            .get("path")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| MapError::Protocol("file.put.commit 未返回 path".into()))
    }

    async fn call(&self, op: &str, payload: Value) -> Result<Envelope, MapError> {
        let node = self.node.lock().expect("node lock").clone();
        let request = Envelope::request(op, payload, node.as_deref());
        let (sender, receiver) = oneshot::channel();
        self.pending
            .lock()
            .expect("pending lock")
            .insert(request.id.clone(), sender);
        let text = serde_json::to_string(&request)?;
        if self.tx.send(Message::Text(text.into())).await.is_err() {
            self.pending
                .lock()
                .expect("pending lock")
                .remove(&request.id);
            return Err(MapError::WebSocket("发送失败，连接已关闭".into()));
        }
        let reply = tokio::time::timeout(CLIENT_TIMEOUT, receiver)
            .await
            .map_err(|_| MapError::Timeout(op.to_owned()))?
            .map_err(|_| MapError::WebSocket("连接在应答前关闭".into()))?;
        if reply.op.ends_with(".err") {
            return Err(remote_error(&reply));
        }
        Ok(reply)
    }
}

/// file 通道：`0x05 + transfer(u32 LE) + offset(u64 LE) + data`。
fn parse_file_chunk(data: &[u8]) -> Option<(u32, u64, &[u8])> {
    if data.len() < 13 || data[0] != 0x05 {
        return None;
    }
    let transfer = u32::from_le_bytes(data[1..5].try_into().ok()?);
    let offset = u64::from_le_bytes(data[5..13].try_into().ok()?);
    Some((transfer, offset, &data[13..]))
}

fn parse_video(data: &[u8]) -> Option<Frame> {
    if data.len() < 27 || data[0] != 0x01 {
        return None;
    }
    let read_u32 = |offset: usize| {
        u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ])
    };
    let surface = read_u32(1);
    let full_w = read_u32(5);
    let full_h = read_u32(9);
    let size = read_u32(13) as usize;
    let flags = u16::from_le_bytes([data[17], data[18]]);
    if 27 + size > data.len() {
        return None;
    }
    let patch = flags & 0x0004 != 0;
    let (x, y, w, h, format, start) = if patch {
        if data.len() < 47 || size < 20 {
            return None;
        }
        let x = read_u32(27);
        let y = read_u32(31);
        let w = read_u32(35);
        let h = read_u32(39);
        let format = read_u32(43);
        (x, y, w, h, format, 47)
    } else {
        (0, 0, full_w, full_h, FORMAT_PNG as u32, 27)
    };
    if format != FORMAT_PNG as u32 || start > 27 + size {
        return None;
    }
    Some(Frame {
        surface,
        full_w,
        full_h,
        x,
        y,
        w,
        h,
        format: format as u8,
        data: data[start..27 + size].to_vec(),
    })
}

fn ws_addr(addr: &str) -> Result<String, MapError> {
    let raw = if addr.contains("://") {
        addr.to_owned()
    } else {
        format!("wss://{addr}")
    };
    let mut url = url::Url::parse(&raw)?;
    if url.path().is_empty() || url.path() == "/" {
        url.set_path("/ws");
    }
    Ok(url.to_string())
}

fn known_hosts_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".mixum/known-hosts")
}

fn load_known(path: &Path) -> Result<HashMap<String, String>, MapError> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let text = std::fs::read_to_string(path)?;
    serde_json::from_str(&text).map_err(MapError::from)
}

/// Forget a TOFU entry. Used when a new environment reuses a host port and
/// therefore presents a new server certificate.
pub fn forget_known_host(addr: &str) -> Result<(), MapError> {
    let path = known_hosts_path();
    let mut known = load_known(&path)?;
    if known.remove(addr).is_some() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&known)?)?;
    }
    Ok(())
}

fn save_known(path: &Path, addr: &str, fingerprint: &str) -> Result<(), MapError> {
    let mut known = load_known(path)?;
    known.insert(addr.to_owned(), fingerprint.to_owned());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&known)?)?;
    Ok(())
}

fn normalize_fingerprint(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[derive(Debug)]
struct PinVerifier {
    expected: Option<String>,
    observed: Arc<Mutex<Option<String>>>,
    supported_algorithms: WebPkiSupportedAlgorithms,
}

impl PinVerifier {
    fn format(der: &[u8]) -> String {
        Sha256::digest(der)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, RustlsError> {
        let got = Self::format(end_entity.as_ref());
        *self.observed.lock().expect("observed lock") = Some(got.clone());
        if let Some(want) = &self.expected {
            if normalize_fingerprint(want) != normalize_fingerprint(&got) {
                return Err(RustlsError::General(format!(
                    "证书指纹不匹配: got {got} want {want}"
                )));
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls12_signature(message, cert, dss, &self.supported_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, RustlsError> {
        verify_tls13_signature(message, cert, dss, &self.supported_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_png_frame() {
        let mut data = vec![0u8; 27 + 4];
        data[0] = 1;
        data[1..5].copy_from_slice(&7u32.to_le_bytes());
        data[5..9].copy_from_slice(&100u32.to_le_bytes());
        data[9..13].copy_from_slice(&50u32.to_le_bytes());
        data[13..17].copy_from_slice(&4u32.to_le_bytes());
        data[17..19].copy_from_slice(&1u16.to_le_bytes());
        data[27..].copy_from_slice(b"PNG!");
        let frame = parse_video(&data).unwrap();
        assert_eq!(frame.surface, 7);
        assert_eq!((frame.full_w, frame.full_h), (100, 50));
        assert_eq!((frame.x, frame.y, frame.w, frame.h), (0, 0, 100, 50));
        assert_eq!(frame.data, b"PNG!");
    }

    #[test]
    fn parses_patch_frame() {
        let payload = b"PNG!";
        let mut data = vec![0u8; 47 + payload.len()];
        data[0] = 1;
        data[1..5].copy_from_slice(&7u32.to_le_bytes());
        data[5..9].copy_from_slice(&100u32.to_le_bytes());
        data[9..13].copy_from_slice(&50u32.to_le_bytes());
        data[13..17].copy_from_slice(&(20u32 + payload.len() as u32).to_le_bytes());
        data[17..19].copy_from_slice(&4u16.to_le_bytes());
        data[27..31].copy_from_slice(&10u32.to_le_bytes());
        data[31..35].copy_from_slice(&20u32.to_le_bytes());
        data[35..39].copy_from_slice(&30u32.to_le_bytes());
        data[39..43].copy_from_slice(&40u32.to_le_bytes());
        data[43..47].copy_from_slice(&(FORMAT_PNG as u32).to_le_bytes());
        data[47..].copy_from_slice(payload);
        let frame = parse_video(&data).unwrap();
        assert_eq!((frame.x, frame.y, frame.w, frame.h), (10, 20, 30, 40));
        assert_eq!(frame.data, payload);
    }

    #[tokio::test]
    #[ignore = "需要运行中的 MAP 节点与 title=map-input 的 foot 窗口"]
    async fn live_input_roundtrip() {
        let url = std::env::var("MAP_URL").expect("MAP_URL");
        let token = std::env::var("MAP_TOKEN").expect("MAP_TOKEN");
        let fingerprint = std::env::var("MAP_FP").ok();
        let client = MapClient::connect(&url, &token, fingerprint.as_deref())
            .await
            .expect("connect");
        let surfaces = client.list().await.expect("list");
        let surface = surfaces
            .iter()
            .find(|surface| surface.title == "map-input")
            .expect("map-input surface");
        let mut frames = client.frames();
        client.attach(surface.id).await.expect("attach");
        let event = |event: mixum_env_surface::InputEvent| event;
        client
            .input(
                surface.id,
                &event(mixum_env_surface::InputEvent::PointerMotion {
                    surface: surface.id,
                    x: 120.0,
                    y: 80.0,
                }),
            )
            .await
            .expect("pointer motion");
        client
            .input(
                surface.id,
                &event(mixum_env_surface::InputEvent::PointerButton {
                    surface: surface.id,
                    button: "left".into(),
                    action: "click".into(),
                }),
            )
            .await
            .expect("pointer button");
        client
            .input(
                surface.id,
                &event(mixum_env_surface::InputEvent::Key {
                    surface: surface.id,
                    keysym: "a".into(),
                    action: "click".into(),
                }),
            )
            .await
            .expect("key a");
        client
            .input(
                surface.id,
                &event(mixum_env_surface::InputEvent::Key {
                    surface: surface.id,
                    keysym: "Return".into(),
                    action: "click".into(),
                }),
            )
            .await
            .expect("key return");
        let frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
            .await
            .expect("frame after input")
            .expect("frame channel");
        eprintln!("input frame {}x{}", frame.w, frame.h);
    }

    #[tokio::test]
    #[ignore = "需要运行中的 MAP 节点：MAP_URL/MAP_TOKEN/MAP_FP"]
    async fn live_list_and_first_frame() {
        let url = std::env::var("MAP_URL").expect("MAP_URL");
        let token = std::env::var("MAP_TOKEN").expect("MAP_TOKEN");
        let fingerprint = std::env::var("MAP_FP").ok();
        let client = MapClient::connect(&url, &token, fingerprint.as_deref())
            .await
            .expect("connect");
        let surfaces = client.list().await.expect("list");
        eprintln!("surfaces: {surfaces:?}");
        assert!(!surfaces.is_empty(), "至少一个 surface");
        let surface = surfaces.last().unwrap().id;
        let mut frames = client.frames();
        client.attach(surface).await.expect("attach");
        let frame = tokio::time::timeout(Duration::from_secs(5), frames.recv())
            .await
            .expect("frame timeout")
            .expect("frame channel");
        eprintln!(
            "first frame {}x{} patch {}x{}@{}",
            frame.full_w, frame.full_h, frame.w, frame.h, frame.x
        );
    }

    #[test]
    fn normalizes_fingerprint() {
        assert_eq!(
            normalize_fingerprint("AA:bb:01"),
            normalize_fingerprint("aa:BB:01")
        );
    }

    #[test]
    fn envelope_node_is_optional_and_skipped_when_none() {
        let with = Envelope::request("display.list", json!({}), Some("home-01"));
        let text = serde_json::to_string(&with).unwrap();
        assert!(text.contains("\"node\":\"home-01\""), "{text}");

        let without = Envelope::request("display.list", json!({}), None);
        let text = serde_json::to_string(&without).unwrap();
        assert!(!text.contains("\"node\""), "{text}");
    }

    #[test]
    fn parses_gateway_pair_claim_and_nodes() {
        let claim: Envelope = serde_json::from_str(
            r#"{"v":1,"id":"x","op":"gateway.pair.claim.ok","replyTo":"r","ts":0,
                "payload":{"token":"t","deviceId":"dev-1"}}"#,
        )
        .unwrap();
        let parsed = parse_pair_claim(claim).unwrap();
        assert_eq!(
            (parsed.token.as_str(), parsed.device_id.as_str()),
            ("t", "dev-1")
        );

        let nodes: Envelope = serde_json::from_str(
            r#"{"v":1,"id":"x","op":"gateway.node.list.ok","ts":0,
                "payload":{"nodes":["a","b"]}}"#,
        )
        .unwrap();
        assert_eq!(parse_nodes(&nodes), vec!["a", "b"]);

        let err: Envelope = serde_json::from_str(
            r#"{"v":1,"id":"x","op":"gateway.pair.claim.err","ts":0,
                "payload":{"code":"conflict","message":"频繁"}}"#,
        )
        .unwrap();
        assert!(matches!(
            parse_pair_claim(err),
            Err(MapError::Remote { code, .. }) if code == "conflict"
        ));
    }
}

#[cfg(test)]
mod file_chunk_tests {
    use super::*;

    #[test]
    fn parses_file_chunk_header() {
        let mut data = vec![0x05];
        data.extend_from_slice(&7u32.to_le_bytes());
        data.extend_from_slice(&1024u64.to_le_bytes());
        data.extend_from_slice(b"payload");
        let (transfer, offset, chunk) = parse_file_chunk(&data).unwrap();
        assert_eq!(transfer, 7);
        assert_eq!(offset, 1024);
        assert_eq!(chunk, b"payload");
        assert!(parse_file_chunk(b"short").is_none());
    }
}
