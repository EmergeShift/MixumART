//! Tauri 命令层：只做「取状态、转错误、交给前端」，不写平台逻辑。
//!
//! 新增命令的顺序：
//! 1. 需要的领域类型先加在 `mixum-env-core`；
//! 2. 命令写在这里；
//! 3. `lib.rs` 的 `generate_handler!` 注册；
//! 4. 前端 `src/api.ts` 包一层带类型的 invoke。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use mixum_env_core::{BackendStatus, Container, CreateSpec, RuntimeKind};
use serde::Serialize;
use serde_json::Value;
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

fn connections_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join("connections.json"))
        .map_err(|error| format!("无法定位应用数据目录：{error}"))
}

fn load_connection_map(app: &AppHandle) -> Result<HashMap<String, EnvironmentConnection>, String> {
    let path = connections_path(app)?;
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            serde_json::from_str(&text).map_err(|error| format!("连接档案格式错误：{error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
        Err(error) => Err(format!("读取连接档案失败：{error}")),
    }
}

fn save_connection_map(
    app: &AppHandle,
    connections: &HashMap<String, EnvironmentConnection>,
) -> Result<(), String> {
    let path = connections_path(app)?;
    let Some(dir) = path.parent() else {
        return Err("连接档案路径没有父目录".into());
    };
    std::fs::create_dir_all(dir).map_err(|error| format!("创建应用数据目录失败：{error}"))?;
    let temp = dir.join(format!(".connections-{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(connections)
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

fn connection_for(app: &AppHandle, container_id: &str) -> Result<EnvironmentConnection, String> {
    load_connection_map(app)?
        .remove(container_id)
        .ok_or_else(|| format!("环境 {container_id} 还没有连接档案"))
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
) -> Result<HashMap<String, EnvironmentConnection>, String> {
    load_connection_map(&app)
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
            format!("MixumART · {title}")
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
