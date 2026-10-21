//! Tauri 命令层：只做「取状态、转错误、交给前端」，不写平台逻辑。
//!
//! 新增命令的顺序：
//! 1. 需要的领域类型先加在 `mixum-env-core`；
//! 2. 命令写在这里；
//! 3. `lib.rs` 的 `generate_handler!` 注册；
//! 4. 前端 `src/api.ts` 包一层带类型的 invoke。

use std::sync::Arc;

use mixum_env_core::{BackendStatus, Container, RuntimeKind};
use serde::Serialize;
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
            let mut clients = state
                .surface_clients
                .lock()
                .expect("surface clients lock");
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
