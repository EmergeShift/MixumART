//! Tauri 命令层：只做「取状态、转错误、交给前端」，不写平台逻辑。
//!
//! 新增命令的顺序：
//! 1. 需要的领域类型先加在 `mixum-env-core`；
//! 2. 命令写在这里；
//! 3. `lib.rs` 的 `generate_handler!` 注册；
//! 4. 前端 `src/api.ts` 包一层带类型的 invoke。

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
// B 路线：surface 窗口（Tauri webview + canvas）
// ---------------------------------------------------------------------------

use std::{
    net::TcpStream,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use mixum_env_display::{send_input, spawn_reader, Event};
use mixum_env_surface::{read_message, InputEvent, SurfaceInfo, MSG_SURFACE_CREATE};
use tauri::{
    webview::PageLoadEvent, AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder,
};

/// 扫描一个 art compositor，返回当前 surface 表。
///
/// compositor 对新连接会重放 surface.create（以及最后一帧全量），所以连接后
/// 读取一小段时间即可拿到完整列表。
#[tauri::command]
pub async fn list_surfaces(addr: String) -> Result<Vec<SurfaceInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || scan_surfaces(&addr))
        .await
        .map_err(|error| error.to_string())?
}

fn scan_surfaces(addr: &str) -> Result<Vec<SurfaceInfo>, String> {
    let mut stream =
        TcpStream::connect(addr).map_err(|error| format!("连接 {addr} 失败：{error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(700)))
        .ok();
    let deadline = Instant::now() + Duration::from_millis(700);
    let mut surfaces = Vec::new();
    while Instant::now() < deadline {
        match read_message(&mut stream) {
            Ok((MSG_SURFACE_CREATE, payload)) => {
                if let Ok(info) = serde_json::from_slice::<SurfaceInfo>(&payload) {
                    if !surfaces
                        .iter()
                        .any(|known: &SurfaceInfo| known.id == info.id)
                    {
                        surfaces.push(info);
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    Ok(surfaces)
}

/// 为一个 surface 打开 Tauri 窗口。窗口加载完成后才开始读帧，避免首帧
/// 比 React 监听器更早到达。
#[tauri::command]
pub fn open_surface_window(
    app: AppHandle,
    addr: String,
    surface: u32,
    title: String,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let label = format!("surface-{surface}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(());
    }

    let stream = TcpStream::connect(&addr).map_err(|error| format!("连接 {addr} 失败：{error}"))?;
    stream.set_nodelay(true).ok();
    let writer = Arc::new(Mutex::new(
        stream
            .try_clone()
            .map_err(|error| format!("复制 socket 失败：{error}"))?,
    ));
    let (input_tx, input_rx) = mpsc::channel::<InputEvent>();
    let writer_for_input = writer.clone();
    std::thread::spawn(move || {
        while let Ok(event) = input_rx.recv() {
            send_input(&writer_for_input, &event);
        }
    });

    let addr_json = serde_json::to_string(&addr).map_err(|error| error.to_string())?;
    let init_script =
        format!("window.__MIXUM_SURFACE__ = {{ surface: {surface}, addr: {addr_json} }};");
    let (ready_tx, ready_rx) = mpsc::channel();
    let window = WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title(if title.is_empty() {
            "MixumART surface".to_string()
        } else {
            format!("MixumART · {title}")
        })
        .inner_size(width.max(320) as f64, height.max(240) as f64)
        .initialization_script(init_script)
        .on_page_load(move |_window, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let _ = ready_tx.send(());
            }
        })
        .build()
        .map_err(|error| format!("创建窗口失败：{error}"))?;

    app.state::<AppState>()
        .surface_inputs
        .lock()
        .map_err(|error| error.to_string())?
        .insert(surface, input_tx);

    let app_for_session = app.clone();
    let label_for_session = label.clone();
    std::thread::spawn(move || {
        let _ = ready_rx.recv_timeout(Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(300));
        eprintln!("surface {surface} session ready");
        let (event_tx, event_rx) = mpsc::channel();
        spawn_reader(stream, event_tx);
        let mut first_frame = true;
        while let Ok(event) = event_rx.recv() {
            match event {
                Event::Surface(info) if info.id == surface => {
                    eprintln!(
                        "surface {surface} info {}x{} ({})",
                        info.width, info.height, info.title
                    );
                    let _ = app_for_session.emit_to(&label_for_session, "surface-info", &info);
                }
                Event::Frame(frame) if frame.surface == surface => {
                    if first_frame {
                        eprintln!(
                            "surface {surface} first frame {}x{}",
                            frame.full_w, frame.full_h
                        );
                        first_frame = false;
                    }
                    let _ = app_for_session.emit_to(&label_for_session, "surface-frame", &frame);
                }
                Event::Destroy(id) if id == surface => {
                    let _ = app_for_session.emit_to(
                        &label_for_session,
                        "surface-destroyed",
                        serde_json::json!({ "surface": id }),
                    );
                    break;
                }
                Event::Disconnected(reason) => {
                    let _ =
                        app_for_session.emit_to(&label_for_session, "surface-disconnected", reason);
                    break;
                }
                _ => {}
            }
        }
    });

    let app_for_close = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { .. } = event {
            if let Ok(mut inputs) = app_for_close.state::<AppState>().surface_inputs.lock() {
                inputs.remove(&surface);
            }
        }
    });

    Ok(())
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

#[cfg(test)]
mod display_tests {
    use super::scan_surfaces;

    /// 需要本机运行中的 art compositor；默认跳过。
    /// `cargo test -p mixum-env-desktop -- --ignored live_scan_surfaces`
    #[test]
    #[ignore = "需要运行中的 art compositor"]
    fn live_scan_surfaces() {
        let surfaces = scan_surfaces("192.168.64.32:7603").expect("scan");
        assert!(!surfaces.is_empty(), "应至少发现一个 surface");
        eprintln!("surfaces: {surfaces:?}");
    }
}
