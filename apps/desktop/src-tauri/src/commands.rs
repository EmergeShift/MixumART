//! Tauri 命令层：只做「取状态、转错误、交给前端」，不写平台逻辑。
//!
//! 新增命令的顺序：
//! 1. 需要的领域类型先加在 `mixum-env-core`；
//! 2. 命令写在这里；
//! 3. `lib.rs` 的 `generate_handler!` 注册；
//! 4. 前端 `src/api.ts` 包一层带类型的 invoke。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mixum_env_core::{
    BackendStatus, Container, CreateSpec, MartRef, MigrationManifest, MigrationSource,
    InspectedContainer, PortMapping, RuntimeKind, MIGRATION_VERSION,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;

use crate::AppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    /// `std::env::consts::OS`：macos / windows / linux。
    pub os: &'static str,
    pub arch: &'static str,
    pub app_version: &'static str,
}

#[tauri::command]
pub fn host_info() -> HostInfo {
    HostInfo {
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        app_version: env!("CARGO_PKG_VERSION"),
    }
}

/// 逐个探测所有后端。串行执行，失败不短路——某个后端报错不应影响其它后端。
#[tauri::command]
pub async fn runtime_status(state: State<'_, AppState>) -> Result<Vec<BackendStatus>, String> {
    Ok(state.registry.statuses().await)
}

#[tauri::command]
pub async fn list_containers(
    kind: RuntimeKind,
    state: State<'_, AppState>,
) -> Result<Vec<Container>, String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .list_containers()
        .await
        .map_err(|err| err.to_string())
}

#[tauri::command]
pub async fn environment_container_spec(
    kind: RuntimeKind,
    container_id: String,
    state: State<'_, AppState>,
) -> Result<InspectedContainer, String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .inspect_container(&container_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn runtime_images(
    kind: RuntimeKind,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend.images().await.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn runtime_disk_usage(
    kind: RuntimeKind,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .disk_usage()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn runtime_pull_image(
    kind: RuntimeKind,
    reference: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .pull_image(&reference)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn runtime_load_image(
    kind: RuntimeKind,
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .load_image(&path)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn runtime_prune(
    kind: RuntimeKind,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend.prune().await.map_err(|error| error.to_string())
}

#[tauri::command]
fn redact_log_secrets(line: String) -> String {
    let mut output = String::with_capacity(line.len());
    let mut rest = line.as_str();
    while let Some(index) = rest.find("map1.") {
        output.push_str(&rest[..index]);
        output.push_str("map1.<redacted>");
        let token = &rest[index..];
        let end = token
            .find(|character: char| {
                character.is_whitespace() || character == '"' || character == '\''
            })
            .unwrap_or(token.len());
        rest = &token[end..];
    }
    output.push_str(rest);
    output
}

#[tauri::command]
pub async fn runtime_logs(
    kind: RuntimeKind,
    id: String,
    tail: usize,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .logs(&id, tail)
        .await
        .map(|lines| lines.into_iter().map(redact_log_secrets).collect())
        .map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// 本地 Agent 注册表：MART 只保存“怎么启动”的配置，不保存凭据。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub mode: String,
    pub core: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub environment_id: String,
}

fn agent_profiles_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("agents.json"))
        .map_err(|error| format!("无法定位应用数据目录：{error}"))
}

fn load_agent_map(app: &AppHandle) -> Result<HashMap<String, AgentProfile>, String> {
    let path = agent_profiles_path(app)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|error| format!("Agent 配置格式错误：{error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(format!("读取 Agent 配置失败：{error}")),
    }
}

fn save_agent_map(app: &AppHandle, profiles: &HashMap<String, AgentProfile>) -> Result<(), String> {
    let path = agent_profiles_path(app)?;
    let Some(dir) = path.parent() else {
        return Err("Agent 配置路径没有父目录".into());
    };
    std::fs::create_dir_all(dir).map_err(|error| format!("创建应用数据目录失败：{error}"))?;
    let temp = dir.join(format!(".agents-{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(profiles)
        .map_err(|error| format!("序列化 Agent 配置失败：{error}"))?;
    std::fs::write(&temp, text).map_err(|error| format!("写入 Agent 配置失败：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置 Agent 配置权限失败：{error}"))?;
    }
    std::fs::rename(&temp, &path).map_err(|error| format!("提交 Agent 配置失败：{error}"))
}

const KEYRING_SERVICE: &str = "mixum-art-agent";

fn keyring_entry(agent_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, agent_id)
        .map_err(|error| format!("打开系统钥匙串失败：{error}"))
}

#[tauri::command]
pub fn set_agent_credential(agent_id: String, secret: String) -> Result<(), String> {
    if secret.is_empty() {
        return Err("凭据不能为空".into());
    }
    keyring_entry(&agent_id)?
        .set_password(&secret)
        .map_err(|error| format!("写入系统钥匙串失败：{error}"))
}

#[tauri::command]
pub fn delete_agent_credential(agent_id: String) -> Result<(), String> {
    match keyring_entry(&agent_id)?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("删除系统钥匙串条目失败：{error}")),
    }
}

#[tauri::command]
pub fn has_agent_credential(agent_id: String) -> Result<bool, String> {
    match keyring_entry(&agent_id)?.get_password() {
        Ok(_) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(error) => Err(format!("读取系统钥匙串失败：{error}")),
    }
}

#[tauri::command]
pub fn load_agent_profiles(app: AppHandle) -> Result<Vec<AgentProfile>, String> {
    let mut profiles: Vec<_> = load_agent_map(&app)?.into_values().collect();
    profiles.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(profiles)
}

#[tauri::command]
pub fn save_agent_profile(app: AppHandle, profile: AgentProfile) -> Result<(), String> {
    if profile.name.trim().is_empty() {
        return Err("Agent 名称不能为空".into());
    }
    let mut profiles = load_agent_map(&app)?;
    profiles.insert(profile.id.clone(), profile);
    save_agent_map(&app, &profiles)
}

#[tauri::command]
pub fn remove_agent_profile(app: AppHandle, id: String) -> Result<(), String> {
    let mut profiles = load_agent_map(&app)?;
    profiles.remove(&id);
    save_agent_map(&app, &profiles)
}

// ---------------------------------------------------------------------------
// 环境写操作：当前仅 WSL2 后端实现，其它后端会返回 Unimplemented。
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn start_environment(
    kind: RuntimeKind,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .start_container(&id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn stop_environment(
    kind: RuntimeKind,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .stop_container(&id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn remove_environment(
    kind: RuntimeKind,
    id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .remove_container(&id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn create_environment(
    kind: RuntimeKind,
    spec: CreateSpec,
    state: State<'_, AppState>,
) -> Result<Container, String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    backend
        .create_container(spec)
        .await
        .map_err(|error| error.to_string())
}

// ---------------------------------------------------------------------------
// 环境连接档案：容器 → MAP 地址/令牌/指纹。
//
// 令牌不写 localStorage；落到 app_data_dir/connections.json（Unix 0600）。
// 这里不是钥匙串实现，后续可以替换成系统 keychain，而不改前端命令形状。
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentConnection {
    pub addr: String,
    pub token: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

/// 前端可见的连接信息；token 永远不返回 WebView。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub addr: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

const CONNECTION_KEYRING_SERVICE: &str = "mixum-art-env";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionFileEntry {
    addr: String,
    #[serde(default)]
    fingerprint: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyConnectionEntry {
    addr: String,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    fingerprint: Option<String>,
}

fn connection_keyring_entry(container_id: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(CONNECTION_KEYRING_SERVICE, container_id)
        .map_err(|error| format!("打开环境令牌钥匙串失败：{error}"))
}

fn connections_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("connections.json"))
        .map_err(|error| format!("无法定位应用数据目录：{error}"))
}

fn write_connection_file(
    app: &AppHandle,
    connections: &HashMap<String, EnvironmentConnection>,
) -> Result<(), String> {
    let path = connections_path(app)?;
    let Some(dir) = path.parent() else {
        return Err("连接档案路径没有父目录".into());
    };
    std::fs::create_dir_all(dir).map_err(|error| format!("创建应用数据目录失败：{error}"))?;
    let entries: HashMap<String, ConnectionFileEntry> = connections
        .iter()
        .map(|(id, connection)| {
            (
                id.clone(),
                ConnectionFileEntry {
                    addr: connection.addr.clone(),
                    fingerprint: connection.fingerprint.clone(),
                },
            )
        })
        .collect();
    let temp = dir.join(format!(".connections-{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(&entries)
        .map_err(|error| format!("序列化连接档案失败：{error}"))?;
    std::fs::write(&temp, text).map_err(|error| format!("写入连接档案失败：{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("设置连接档案权限失败：{error}"))?;
    }
    std::fs::rename(&temp, &path).map_err(|error| format!("提交连接档案失败：{error}"))
}

fn load_connection_map(app: &AppHandle) -> Result<HashMap<String, EnvironmentConnection>, String> {
    let path = connections_path(app)?;
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(format!("读取连接档案失败：{error}")),
    };
    let data: HashMap<String, LegacyConnectionEntry> =
        serde_json::from_str(&text).map_err(|error| format!("连接档案格式错误：{error}"))?;
    let mut connections = HashMap::new();
    let mut migrated = false;
    for (id, entry) in data {
        let token = if let Some(token) = entry.token.filter(|token| !token.is_empty()) {
            connection_keyring_entry(&id)?
                .set_password(&token)
                .map_err(|error| format!("迁移环境令牌到钥匙串失败：{error}"))?;
            migrated = true;
            token
        } else {
            match connection_keyring_entry(&id)?.get_password() {
                Ok(token) => token,
                Err(keyring::Error::NoEntry) => {
                    return Err(format!("环境 {id} 的钥匙串令牌不存在"))
                }
                Err(error) => return Err(format!("读取环境令牌钥匙串失败：{error}")),
            }
        };
        connections.insert(
            id,
            EnvironmentConnection {
                addr: entry.addr,
                token,
                fingerprint: entry.fingerprint,
            },
        );
    }
    if migrated {
        write_connection_file(app, &connections)?;
    }
    Ok(connections)
}

fn save_connection_map(
    app: &AppHandle,
    connections: &HashMap<String, EnvironmentConnection>,
) -> Result<(), String> {
    for (id, connection) in connections {
        if connection.token.trim().is_empty() {
            return Err(format!("环境 {id} 的令牌为空"));
        }
        connection_keyring_entry(id)?
            .set_password(&connection.token)
            .map_err(|error| format!("写入环境令牌钥匙串失败：{error}"))?;
    }
    write_connection_file(app, connections)
}

fn connection_for(app: &AppHandle, container_id: &str) -> Result<EnvironmentConnection, String> {
    load_connection_map(app)?
        .remove(container_id)
        .ok_or_else(|| format!("环境 {container_id} 还没有连接档案"))
}

fn free_host_port(used: &mut std::collections::HashSet<u16>) -> Result<u16, String> {
    for _ in 0..32 {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("分配端口: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("读取端口: {error}"))?
            .port();
        if used.insert(port) {
            return Ok(port);
        }
    }
    Err("找不到可用端口".into())
}

async fn wait_agent_token(
    backend: &dyn mixum_env_core::RuntimeBackend,
    id: &str,
) -> Result<String, String> {
    for _ in 0..120 {
        if let Ok(output) = backend
            .exec_capture(id, &["cat", "/var/lib/mixum/admin-token"])
            .await
        {
            let token = output.trim();
            if !token.is_empty() {
                return Ok(token.to_owned());
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err("等待 agentd 令牌超时".into())
}

async fn transfer_node(
    source: &MapClient,
    target: &MapClient,
    node: &str,
    origin: &str,
) -> Result<String, String> {
    let reply = source
        .snapshot_export(node)
        .await
        .map_err(|error| error.to_string())?;
    let file = reply
        .get("file")
        .and_then(Value::as_str)
        .ok_or_else(|| "snap.export 未返回 file".to_string())?
        .to_owned();
    let expected = reply
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let data = source
        .get_file(&format!("transfers/{file}"))
        .await
        .map_err(|error| error.to_string())?;
    let actual: String = Sha256::digest(&data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !expected.is_empty() && actual != expected {
        return Err("迁移文件 sha256 校验失败".into());
    }

    let name = Path::new(&file)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "迁移文件名无效".to_string())?;
    target
        .put_file(name, &data)
        .await
        .map_err(|error| error.to_string())?;
    let reply = target
        .snapshot_import(name, origin)
        .await
        .map_err(|error| error.to_string())?;
    reply
        .get("node")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "snap.import 未返回 node".to_string())
}

async fn snapshot_client(app: &AppHandle, container_id: &str) -> Result<MapClient, String> {
    let connection = connection_for(app, container_id)?;
    MapClient::connect(
        &connection.addr,
        &connection.token,
        connection.fingerprint.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn environment_snapshot_tree(
    app: AppHandle,
    container_id: String,
) -> Result<Value, String> {
    let client = snapshot_client(&app, &container_id).await?;
    client
        .snapshot_tree()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn environment_snapshot_create(
    app: AppHandle,
    container_id: String,
    label: Option<String>,
) -> Result<Value, String> {
    let client = snapshot_client(&app, &container_id).await?;
    client
        .snapshot_create(label.as_deref())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn environment_snapshot_create_kind(
    app: AppHandle,
    container_id: String,
    kind: String,
    label: Option<String>,
) -> Result<Value, String> {
    let client = snapshot_client(&app, &container_id).await?;
    client
        .snapshot_create_kind(&kind, label.as_deref())
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn environment_snapshot_restore(
    app: AppHandle,
    container_id: String,
    node: String,
) -> Result<Value, String> {
    let client = snapshot_client(&app, &container_id).await?;
    client
        .snapshot_restore(&node)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn environment_snapshot_fork(
    app: AppHandle,
    container_id: String,
    node: String,
    branch: String,
) -> Result<Value, String> {
    let client = snapshot_client(&app, &container_id).await?;
    client
        .snapshot_fork(&node, &branch)
        .await
        .map_err(|error| error.to_string())
}

async fn node_gui_sessions(client: &MapClient, node: &str) -> Vec<Value> {
    let Ok(tree) = client.snapshot_tree().await else {
        return Vec::new();
    };
    tree.get("nodes")
        .and_then(Value::as_array)
        .and_then(|nodes| {
            nodes
                .iter()
                .find(|entry| entry.get("id").and_then(Value::as_str) == Some(node))
        })
        .and_then(|entry| entry.get("guiSessions"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn migration_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("mixum/migrations")
}

/// 列出本机缓存里的迁移包（`.mart`）。
#[tauri::command]
pub async fn list_migrations() -> Result<Vec<MigrationFile>, String> {
    let dir = migration_dir();
    let mut out = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
        return Ok(out);
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("mart") {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        out.push(MigrationFile {
            path: path.to_string_lossy().into_owned(),
            size: metadata.len(),
            sha256: String::new(),
            manifest_path: None,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub manifest_path: Option<String>,
}

/// 导出快照节点为 `.mart` 并下载到本机缓存。用户不需要手工拖文件。
#[tauri::command]
pub async fn environment_snapshot_export(
    app: AppHandle,
    container_id: String,
    node: String,
    spec: Option<CreateSpec>,
    image_digest: Option<String>,
    runtime: Option<String>,
    branch: Option<String>,
) -> Result<MigrationFile, String> {
    let client = snapshot_client(&app, &container_id).await?;
    let reply = client
        .snapshot_export(&node)
        .await
        .map_err(|error| error.to_string())?;
    let file = reply
        .get("file")
        .and_then(Value::as_str)
        .ok_or_else(|| "snap.export 未返回 file".to_string())?
        .to_owned();
    let expected = reply
        .get("sha256")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let data = client
        .get_file(&format!("transfers/{file}"))
        .await
        .map_err(|error| error.to_string())?;
    let actual: String = Sha256::digest(&data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if !expected.is_empty() && actual != expected {
        return Err("迁移文件 sha256 校验失败".into());
    }

    let dir = migration_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| format!("创建迁移缓存目录: {error}"))?;
    let name = Path::new(&file)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "迁移文件名无效".to_string())?;
    let path = dir.join(name);
    tokio::fs::write(&path, &data)
        .await
        .map_err(|error| format!("写入迁移文件: {error}"))?;

    let gui_sessions = if spec.is_some() {
        node_gui_sessions(&client, &node).await
    } else {
        Vec::new()
    };
    let manifest_path = if let Some(spec) = spec {
        let manifest = MigrationManifest {
            version: MIGRATION_VERSION,
            source: MigrationSource {
                container_id,
                runtime: runtime.unwrap_or_default(),
                node,
                branch: branch.unwrap_or_default(),
                exported_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0),
            },
            container: spec,
            image_digest,
            mart: MartRef {
                file: name.to_owned(),
                sha256: actual.clone(),
                size: data.len() as u64,
            },
            gui_sessions,
        };
        let stem = name.trim_end_matches(".mart");
        let manifest_path = dir.join(format!("{stem}.migration.json"));
        tokio::fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest)
                .map_err(|error| format!("序列化迁移元数据: {error}"))?,
        )
        .await
        .map_err(|error| format!("写入迁移元数据: {error}"))?;
        Some(manifest_path.to_string_lossy().into_owned())
    } else {
        None
    };

    Ok(MigrationFile {
        path: path.to_string_lossy().into_owned(),
        size: data.len() as u64,
        sha256: actual,
        manifest_path,
    })
}

/// 直接从源环境导出 `.mart` 并迁移到目标环境（不需要落盘/手工拖文件）。
#[tauri::command]
pub async fn environment_migrate(
    app: AppHandle,
    source_container_id: String,
    target_container_id: String,
    node: String,
    origin: Option<String>,
) -> Result<String, String> {
    let source = snapshot_client(&app, &source_container_id).await?;
    let target = snapshot_client(&app, &target_container_id).await?;
    transfer_node(&source, &target, &node, origin.as_deref().unwrap_or("mixumart")).await
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrateNewResult {
    pub target_container_id: String,
    pub target_node: String,
    pub agentd_addr: String,
}

/// 在本机按源容器规格新建一个环境，然后把 `.mart` 迁移过去。
/// 端口会重新分配，避免和源容器冲突；agentd 令牌从容器内读取并写入连接档案。
#[tauri::command]
pub async fn environment_migrate_new(
    app: AppHandle,
    source_container_id: String,
    source_kind: RuntimeKind,
    target_kind: RuntimeKind,
    node: String,
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<MigrateNewResult, String> {
    let source_backend = state
        .registry
        .get(source_kind)
        .ok_or_else(|| format!("没有注册 {source_kind} 后端"))?;
    let inspected = source_backend
        .inspect_container(&source_container_id)
        .await
        .map_err(|error| error.to_string())?;
    let spec = inspected.spec;
    if !spec.ports.iter().any(|port| port.container == 7601) {
        return Err("源容器没有映射 agentd 7601，无法自动迁移".into());
    }

    let mut used = std::collections::HashSet::new();
    let mut agentd_host = None;
    let mut ports = Vec::new();
    for port in &spec.ports {
        let host = free_host_port(&mut used)?;
        if port.container == 7601 {
            agentd_host = Some(host);
        }
        ports.push(PortMapping {
            host,
            container: port.container,
            protocol: port.protocol.clone(),
        });
    }
    let agentd_host = agentd_host.ok_or_else(|| "没有 agentd 端口映射".to_string())?;

    let name = name
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("mixum-{}", node.chars().take(8).collect::<String>()));
    let target_backend = state
        .registry
        .get(target_kind)
        .ok_or_else(|| format!("没有注册 {target_kind} 后端"))?;
    let container = target_backend
        .create_container(CreateSpec {
            name,
            image: spec.image,
            ports,
            mounts: spec.mounts,
            env: spec.env,
            command: Vec::new(),
        })
        .await
        .map_err(|error| error.to_string())?;
    target_backend
        .start_container(&container.id)
        .await
        .map_err(|error| error.to_string())?;
    let token = wait_agent_token(&*target_backend, &container.id).await?;
    let addr = format!("127.0.0.1:{agentd_host}");
    let mut connections = load_connection_map(&app)?;
    connections.insert(
        container.id.clone(),
        EnvironmentConnection {
            addr: addr.clone(),
            token: token.clone(),
            fingerprint: None,
        },
    );
    save_connection_map(&app, &connections)?;

    let source = snapshot_client(&app, &source_container_id).await?;
    let target = MapClient::connect(&addr, &token, None)
        .await
        .map_err(|error| error.to_string())?;
    let target_node = transfer_node(&source, &target, &node, "mixumart").await?;
    Ok(MigrateNewResult {
        target_container_id: container.id,
        target_node,
        agentd_addr: addr,
    })
}

/// 把本机 `.mart` 上传到目标环境并导入为快照节点。
#[tauri::command]
pub async fn environment_snapshot_import(
    app: AppHandle,
    container_id: String,
    path: String,
    origin: Option<String>,
) -> Result<String, String> {
    let data = tokio::fs::read(&path)
        .await
        .map_err(|error| format!("读取迁移文件: {error}"))?;
    let name = Path::new(&path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "迁移文件名无效".to_string())?
        .to_owned();
    let client = snapshot_client(&app, &container_id).await?;
    client
        .put_file(&name, &data)
        .await
        .map_err(|error| error.to_string())?;
    let reply = client
        .snapshot_import(&name, origin.as_deref().unwrap_or("mixumart"))
        .await
        .map_err(|error| error.to_string())?;
    reply
        .get("node")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "snap.import 未返回 node".to_string())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandResult {
    pub task_id: String,
    pub state: String,
    pub lines: Vec<String>,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn environment_run_command(
    app: AppHandle,
    container_id: String,
    command: String,
) -> Result<CommandResult, String> {
    let connection = connection_for(&app, &container_id)?;
    let client = MapClient::connect(
        &connection.addr,
        &connection.token,
        connection.fingerprint.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())?;
    let mut events = client.events();
    client
        .subscribe_task_events()
        .await
        .map_err(|error| error.to_string())?;
    let task_id = client
        .submit_shell_task(&command)
        .await
        .map_err(|error| error.to_string())?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    let mut lines = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(format!("命令执行超时，任务 {task_id} 仍在运行"));
        }
        let event = tokio::time::timeout(remaining, events.recv())
            .await
            .map_err(|_| format!("命令执行超时，任务 {task_id} 仍在运行"))?
            .map_err(|error| format!("事件通道关闭：{error}"))?;
        if event.op == "privilege.request" {
            let _ = app.emit(
                "privilege-request",
                serde_json::json!({
                    "containerId": container_id,
                    "request": event.payload,
                }),
            );
            continue;
        }
        if event.op != "task.event" {
            continue;
        }
        let Some(task_id_value) = event.payload.get("taskId").and_then(Value::as_str) else {
            continue;
        };
        if task_id_value != task_id {
            continue;
        }
        let event_type = event
            .payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let data = event.payload.get("data").cloned().unwrap_or(Value::Null);
        match event_type {
            "log" => {
                if let Some(line) = data.get("line").and_then(Value::as_str) {
                    lines.push(line.to_owned());
                }
            }
            "state" => {
                let state = data
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if state == "completed" {
                    return Ok(CommandResult {
                        task_id,
                        state,
                        lines,
                        error: None,
                    });
                }
                if state == "failed" || state == "canceled" {
                    let error = event
                        .payload
                        .get("data")
                        .and_then(|data| data.get("message"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    return Ok(CommandResult {
                        task_id,
                        state,
                        lines,
                        error,
                    });
                }
            }
            "error" => {
                lines.push(format!(
                    "错误：{}",
                    data.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("任务失败")
                ));
            }
            _ => {}
        }
    }
}

#[tauri::command]
pub fn load_environment_connections(
    app: AppHandle,
) -> Result<HashMap<String, ConnectionInfo>, String> {
    Ok(load_connection_map(&app)?
        .into_iter()
        .map(|(id, connection)| {
            (
                id,
                ConnectionInfo {
                    addr: connection.addr,
                    fingerprint: connection.fingerprint,
                },
            )
        })
        .collect())
}

#[tauri::command]
pub async fn connect_environment(
    app: AppHandle,
    kind: RuntimeKind,
    container_id: String,
    addr: String,
    state: State<'_, AppState>,
) -> Result<ConnectionInfo, String> {
    let backend = state
        .registry
        .get(kind)
        .ok_or_else(|| format!("没有注册 {kind} 后端"))?;
    let token = backend
        .admin_token(&container_id)
        .await
        .map_err(|error| format!("读取容器令牌失败：{error}"))?;
    let client = MapClient::connect(&addr, &token, None)
        .await
        .map_err(|error| format!("连接环境失败：{error}"))?;
    let connection = EnvironmentConnection {
        addr: addr.clone(),
        token,
        fingerprint: Some(client.fingerprint.clone()),
    };
    let mut connections = load_connection_map(&app)?;
    connections.insert(container_id, connection);
    save_connection_map(&app, &connections)?;
    Ok(ConnectionInfo {
        addr,
        fingerprint: Some(client.fingerprint),
    })
}

#[tauri::command]
pub async fn list_environment_surfaces(
    app: AppHandle,
    container_id: String,
) -> Result<Vec<SurfaceInfo>, String> {
    let connection = connection_for(&app, &container_id)?;
    let client = MapClient::connect(
        &connection.addr,
        &connection.token,
        connection.fingerprint.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())?;
    client.list().await.map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_environment_surface(
    app: AppHandle,
    container_id: String,
    surface: u32,
    title: String,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let connection = connection_for(&app, &container_id)?;
    open_surface_window(
        app,
        OpenSurfaceRequest {
            addr: connection.addr,
            token: connection.token,
            fingerprint: connection.fingerprint,
            surface,
            title,
            width,
            height,
        },
    )
}

#[tauri::command]
pub fn save_environment_connection(
    app: AppHandle,
    container_id: String,
    connection: EnvironmentConnection,
) -> Result<(), String> {
    let mut connections = load_connection_map(&app)?;
    connections.insert(container_id, connection);
    save_connection_map(&app, &connections)
}

#[tauri::command]
pub fn remove_environment_connection(app: AppHandle, container_id: String) -> Result<(), String> {
    let mut connections = load_connection_map(&app)?;
    connections.remove(&container_id);
    match connection_keyring_entry(&container_id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(error) => return Err(format!("删除环境令牌失败：{error}")),
    }
    save_connection_map(&app, &connections)
}

// ---------------------------------------------------------------------------
// B 路线：surface 窗口（MAP WSS + Tauri webview canvas）
// ---------------------------------------------------------------------------

use std::time::Duration;

use mixum_env_map::MapClient;
use mixum_env_surface::{InputEvent, SurfaceInfo};
use tauri::{
    webview::PageLoadEvent, AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};
use tokio::sync::{mpsc as tokio_mpsc, oneshot, watch};

/// 扫描一个 MAP 节点的窗口（surface）表。
#[tauri::command]
pub async fn list_surfaces(
    addr: String,
    token: String,
    fingerprint: Option<String>,
) -> Result<Vec<SurfaceInfo>, String> {
    let client = MapClient::connect(&addr, &token, fingerprint.as_deref())
        .await
        .map_err(|error| error.to_string())?;
    client.list().await.map_err(|error| error.to_string())
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSurfaceRequest {
    pub addr: String,
    pub token: String,
    #[serde(default)]
    pub fingerprint: Option<String>,
    pub surface: u32,
    #[serde(default)]
    pub title: String,
    pub width: u32,
    pub height: u32,
}

/// 为一个 surface 打开 Tauri 窗口。窗口加载完成后才连 MAP/attach，避免
/// 首帧比 React 监听器更早到达。
#[tauri::command]
pub fn open_surface_window(app: AppHandle, request: OpenSurfaceRequest) -> Result<(), String> {
    let OpenSurfaceRequest {
        addr,
        token,
        fingerprint,
        surface,
        title,
        width,
        height,
    } = request;
    let label = format!("surface-{surface}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(());
    }

    let addr_json = serde_json::to_string(&addr).map_err(|error| error.to_string())?;
    let init_script =
        format!("window.__MIXUM_SURFACE__ = {{ surface: {surface}, addr: {addr_json} }};");
    let (ready_tx, mut ready_rx) = watch::channel(false);
    let builder = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title(if title.is_empty() {
            "MixumART surface".to_string()
        } else {
            format!("MixumART · {title} · Ctrl/Cmd+Shift+T 接管")
        })
        .inner_size(width.max(320) as f64, height.max(240) as f64)
        // 沉浸式标题栏只属于 MART 主窗口；容器应用窗口保留普通系统标题栏。
        .decorations(true)
        .initialization_script(init_script)
        .on_page_load(move |_window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let _ = ready_tx.send(true);
            }
        });
    #[cfg(target_os = "macos")]
    let builder = builder.title_bar_style(tauri::TitleBarStyle::Visible);
    let window = builder
        .build()
        .map_err(|error| format!("创建窗口失败：{error}"))?;

    #[cfg(target_os = "macos")]
    let native_ptr = if std::env::var("MIXUM_NATIVE_METAL").as_deref() == Ok("0") {
        None
    } else {
        match crate::macos::attach(&window, surface, width, height) {
            Ok(pointer) => Some(pointer),
            Err(error) => {
                eprintln!("native metal overlay 不可用，回退 canvas：{error}");
                None
            }
        }
    };

    let (input_tx, input_rx) = tokio_mpsc::unbounded_channel::<InputEvent>();
    let (close_tx, close_rx) = oneshot::channel::<()>();
    {
        let state = app.state::<AppState>();
        let mut inputs = state
            .surface_inputs
            .lock()
            .map_err(|error| error.to_string())?;
        inputs.insert(surface, input_tx);
        let mut closes = state
            .surface_closes
            .lock()
            .map_err(|error| error.to_string())?;
        closes.insert(surface, close_tx);
        #[cfg(target_os = "macos")]
        if let Some(pointer) = native_ptr {
            state
                .native_surfaces
                .lock()
                .map_err(|error| error.to_string())?
                .insert(surface, pointer);
        }
    }

    let app_for_session = app.clone();
    let label_for_session = label.clone();
    tauri::async_runtime::spawn(async move {
        if ready_rx.wait_for(|ready| *ready).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        eprintln!("surface {surface} MAP session ready");
        let client = match MapClient::connect(&addr, &token, fingerprint.as_deref()).await {
            Ok(client) => client,
            Err(error) => {
                let _ = app_for_session.emit_to(
                    &label_for_session,
                    "surface-disconnected",
                    error.to_string(),
                );
                return;
            }
        };
        let client = Arc::new(client);
        {
            let state = app_for_session.state::<AppState>();
            let mut clients = state.surface_clients.lock().expect("surface clients lock");
            clients.insert(surface, client.clone());
        }
        // 先订阅再 attach：stream.start 触发的全量重放不能丢。
        let mut frames = client.frames();
        if let Err(error) = client.attach(surface).await {
            let _ = app_for_session.emit_to(
                &label_for_session,
                "surface-disconnected",
                error.to_string(),
            );
            return;
        }
        let info = SurfaceInfo {
            id: surface,
            app_id: String::new(),
            title,
            width,
            height,
        };
        let _ = app_for_session.emit_to(&label_for_session, "surface-info", &info);

        let mut input_rx = input_rx;
        let mut close_rx = close_rx;
        let mut first_frame = true;
        loop {
            tokio::select! {
                _ = &mut close_rx => break,
                Some(event) = input_rx.recv() => {
                    if let Err(error) = client.input(surface, &event).await {
                        let _ = app_for_session.emit_to(
                            &label_for_session,
                            "surface-input-error",
                            error.to_string(),
                        );
                    }
                }
                result = frames.recv() => {
                    match result {
                        Ok(frame) if frame.surface == surface => {
                            if first_frame {
                                eprintln!(
                                    "surface {surface} first frame {}x{}",
                                    frame.full_w, frame.full_h
                                );
                                first_frame = false;
                            }
                            #[cfg(target_os = "macos")]
                            crate::macos::push_frame(&app_for_session, surface, &frame);
                            let _ = app_for_session.emit_to(
                                &label_for_session,
                                "surface-frame",
                                &frame,
                            );
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            eprintln!("surface {surface} 帧积压，丢弃 {skipped}");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            let _ = app_for_session.emit_to(
                                &label_for_session,
                                "surface-disconnected",
                                "MAP 连接已关闭",
                            );
                            break;
                        }
                    }
                }
            }
        }
    });

    let app_for_close = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { .. } = event {
            if let Ok(mut inputs) = app_for_close.state::<AppState>().surface_inputs.lock() {
                inputs.remove(&surface);
            }
            if let Ok(mut closes) = app_for_close.state::<AppState>().surface_closes.lock() {
                if let Some(close) = closes.remove(&surface) {
                    let _ = close.send(());
                }
            }
            if let Ok(mut clients) = app_for_close.state::<AppState>().surface_clients.lock() {
                clients.remove(&surface);
            }
            #[cfg(target_os = "macos")]
            crate::macos::detach(&app_for_close, surface);
        }
    });

    Ok(())
}

#[tauri::command]
pub async fn surface_put_file(
    app: AppHandle,
    surface: u32,
    name: String,
    data: Vec<u8>,
) -> Result<String, String> {
    let client = app
        .state::<AppState>()
        .surface_clients
        .lock()
        .map_err(|error| error.to_string())?
        .get(&surface)
        .cloned()
        .ok_or_else(|| "surface 尚未连接".to_string())?;
    client
        .put_file(&name, &data)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn surface_takeover(
    app: AppHandle,
    surface: u32,
    action: String,
) -> Result<String, String> {
    let client = {
        let state = app.state::<AppState>();
        let clients = state
            .surface_clients
            .lock()
            .map_err(|error| error.to_string())?;
        clients.get(&surface).cloned()
    };
    let Some(client) = client else {
        return Err(format!("surface {surface} 没有打开的窗口"));
    };
    client
        .takeover(surface, &action, true)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn open_container_terminal(
    app: AppHandle,
    kind: RuntimeKind,
    container_id: String,
    cols: u16,
    rows: u16,
) -> Result<u32, String> {
    app.state::<AppState>()
        .terminals
        .open(app.clone(), kind, &container_id, cols, rows)
}

#[tauri::command]
pub fn container_terminal_input(app: AppHandle, terminal: u32, data: String) -> Result<(), String> {
    app.state::<AppState>().terminals.input(terminal, &data)
}

#[tauri::command]
pub fn container_terminal_resize(
    app: AppHandle,
    terminal: u32,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    app.state::<AppState>()
        .terminals
        .resize(terminal, cols, rows)
}

#[tauri::command]
pub fn close_container_terminal(app: AppHandle, terminal: u32) -> Result<(), String> {
    app.state::<AppState>().terminals.close(terminal)
}

#[tauri::command]
pub fn surface_input(app: AppHandle, surface: u32, event: InputEvent) -> Result<(), String> {
    let state = app.state::<AppState>();
    let inputs = state
        .surface_inputs
        .lock()
        .map_err(|error| error.to_string())?;
    let sender = inputs
        .get(&surface)
        .ok_or_else(|| format!("surface {surface} 没有打开的窗口"))?;
    sender.send(event).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn remote_clipboard(app: AppHandle, surface: u32) -> Result<String, String> {
    let client = {
        let state = app.state::<AppState>();
        let clients = state
            .surface_clients
            .lock()
            .map_err(|error| error.to_string())?;
        clients.get(&surface).cloned()
    };
    let Some(client) = client else {
        return Err(format!("surface {surface} 没有打开的窗口"));
    };
    client.clipboard().await.map_err(|error| error.to_string())
}


#[tauri::command]
pub async fn environment_privilege_decide(
    app: AppHandle,
    container_id: String,
    id: String,
    approve: bool,
    reason: String,
) -> Result<(), String> {
    if approve {
        authenticate_host_privilege()?;
    }
    let connection = connection_for(&app, &container_id)?;
    let client = MapClient::connect(
        &connection.addr,
        &connection.token,
        connection.fingerprint.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())?;
    client
        .privilege_decision(&id, approve, &reason)
        .await
        .map_err(|error| error.to_string())
}

/// Ask the OS for a human credential before approving an agent's sudo request.
/// macOS uses the native administrator dialog; Linux falls back to polkit.
fn authenticate_host_privilege() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let status = std::process::Command::new("/usr/bin/osascript")
            .args(["-e", "do shell script \"true\" with administrator privileges"])
            .status()
            .map_err(|error| format!("host authentication unavailable: {error}"))?;
        if !status.success() {
            return Err("host authentication cancelled or failed".into());
        }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("pkexec")
            .arg("true")
            .status()
            .map_err(|error| format!("host authentication unavailable: {error}"))?;
        if !status.success() {
            return Err("host authentication cancelled or failed".into());
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err("host authentication is not implemented on this platform; use `mixum privilege approve`".into())
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalView {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub selector: Option<String>,
    pub domains: Vec<String>,
    pub agent_stance: Option<String>,
    pub agent_reason: Option<String>,
    pub created_at_ms: u128,
    pub approved: bool,
    pub action: String,
}

fn approvals_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("无法定位应用数据目录：{error}"))?;
    Ok(base.join("mixum-cli").join("approvals"))
}

#[tauri::command]
pub fn list_approvals(app: AppHandle) -> Result<Vec<ApprovalView>, String> {
    let dir = approvals_dir(&app)?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(data) = std::fs::read(&path) else {
            continue;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&data) else {
            continue;
        };
        if value.get("consumed").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if id.is_empty() {
            continue;
        }
        if let Some(domains) = value.get("domains").and_then(Value::as_array) {
            out.push(ApprovalView {
                kind: "browser".into(),
                id: id.clone(),
                title: value
                    .get("selector")
                    .and_then(Value::as_str)
                    .unwrap_or("browser")
                    .to_owned(),
                selector: value
                    .get("selector")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                domains: domains
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect(),
                agent_stance: None,
                agent_reason: value
                    .get("reason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                created_at_ms: value
                    .get("createdAtMs")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as u128,
                approved: value.get("approved").and_then(Value::as_bool).unwrap_or(false),
                action: format!("mixum browser approve {id}"),
            });
        } else {
            out.push(ApprovalView {
                kind: "root".into(),
                id: id.clone(),
                title: value
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("root container")
                    .to_owned(),
                selector: None,
                domains: Vec::new(),
                agent_stance: value
                    .get("agentStance")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                agent_reason: value
                    .get("agentReason")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                created_at_ms: value
                    .get("createdAtMs")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as u128,
                approved: value.get("approved").and_then(Value::as_bool).unwrap_or(false),
                action: format!("mixum root approve {id}"),
            });
        }
    }
    out.sort_by_key(|item| item.created_at_ms);
    Ok(out)
}

#[tauri::command]
pub fn approve_root_approval(app: AppHandle, id: String) -> Result<(), String> {
    if !id.starts_with("root-") {
        return Err("not a root approval".into());
    }
    authenticate_host_privilege()?;
    let path = approvals_dir(&app)?.join(format!("{id}.json"));
    let data = std::fs::read(&path).map_err(|error| error.to_string())?;
    let mut value: Value = serde_json::from_slice(&data).map_err(|error| error.to_string())?;
    value["approved"] = Value::Bool(true);
    std::fs::write(path, serde_json::to_vec_pretty(&value).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    append_approval_audit(&app, "root_approved", &id, serde_json::json!({}));
    Ok(())
}

#[tauri::command]
pub fn deny_approval(app: AppHandle, id: String) -> Result<(), String> {
    if !id.starts_with("root-") && !id.starts_with("browser-") {
        return Err("invalid approval id".into());
    }
    let path = approvals_dir(&app)?.join(format!("{id}.json"));
    std::fs::remove_file(path).map_err(|error| error.to_string())?;
    append_approval_audit(&app, "approval_denied", &id, serde_json::json!({}));
    Ok(())
}

#[tauri::command]
pub fn approve_browser_approval(app: AppHandle, id: String) -> Result<(), String> {
    if !id.starts_with("browser-") {
        return Err("not a browser approval".into());
    }
    authenticate_host_privilege()?;
    let cli = std::env::var("MIXUM_CLI").unwrap_or_else(|_| "mixum".into());
    let output = std::process::Command::new(&cli)
        .args(["browser", "approve", &id])
        .env("MIXUM_HOST_AUTHENTICATED", "1")
        .output()
        .map_err(|error| format!("run {cli} browser approve: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "browser approve failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let _ = app; // AppHandle kept for future audit wiring.
    Ok(())
}

/// Pending sudo requests across environments with a saved MAP connection.
/// Approval stays in `environment_privilege_decide` (OS authentication again).
#[tauri::command]
pub async fn list_pending_privileges(app: AppHandle) -> Result<Vec<Value>, String> {
    let map = load_connection_map(&app)?;
    let mut out = Vec::new();
    for (container_id, connection) in map {
        let client = match MapClient::connect(
            &connection.addr,
            &connection.token,
            connection.fingerprint.as_deref(),
        )
        .await
        {
            Ok(client) => client,
            Err(_) => continue,
        };
        let Ok(Ok(requests)) =
            tokio::time::timeout(Duration::from_secs(2), client.privilege_list()).await
        else {
            continue;
        };
        for mut request in requests {
            if let Some(object) = request.as_object_mut() {
                object.insert("containerId".into(), Value::String(container_id.clone()));
            }
            out.push(request);
        }
    }
    Ok(out)
}

fn append_approval_audit(app: &AppHandle, action: &str, id: &str, detail: Value) {
    let Ok(base) = app.path().app_data_dir() else {
        return;
    };
    let dir = base.join("mixum-cli").join("audit");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let line = serde_json::json!({
        "action": action,
        "id": id,
        "detail": detail,
        "ts": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default(),
    });
    let path = dir.join("approvals.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
}
