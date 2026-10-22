//! Tauri 应用壳：装配后端、注册命令、启动窗口。
//!
//! 业务逻辑不写在这里：
//! - 平台细节 → `crates/mixum-env-runtime-*`
//! - 命令实现 → `commands.rs`
//! - 界面 → `apps/desktop/src`

mod commands;
#[cfg(target_os = "macos")]
mod macos;
mod terminal;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use mixum_env_core::{BackendRegistry, RuntimeKind};
use mixum_env_surface::InputEvent;
#[cfg(target_os = "windows")]
use tauri::Manager;

/// 全局状态：后端注册表 + 每个 surface 窗口的输入/关闭通道。
pub struct AppState {
    pub registry: Arc<BackendRegistry>,
    pub surface_inputs: Mutex<HashMap<u32, tokio::sync::mpsc::UnboundedSender<InputEvent>>>,
    pub surface_closes: Mutex<HashMap<u32, tokio::sync::oneshot::Sender<()>>>,
    pub surface_clients: Mutex<HashMap<u32, Arc<mixum_env_map::MapClient>>>,
    pub terminals: crate::terminal::TerminalManager,
    #[cfg(target_os = "macos")]
    pub native_surfaces: Mutex<HashMap<u32, usize>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(build_registry()),
            surface_inputs: Mutex::new(HashMap::new()),
            surface_closes: Mutex::new(HashMap::new()),
            surface_clients: Mutex::new(HashMap::new()),
            terminals: crate::terminal::TerminalManager::default(),
            #[cfg(target_os = "macos")]
            native_surfaces: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// 装配顺序由 [`RuntimeKind::ALL`] 决定，新增 runtime 只需在这里加一个分支。
fn build_registry() -> BackendRegistry {
    let mut registry = BackendRegistry::new();
    for kind in RuntimeKind::ALL {
        registry.register(match kind {
            RuntimeKind::Docker => mixum_env_runtime_docker::backend(),
            RuntimeKind::Wsl2 => mixum_env_runtime_wsl2::backend(),
            RuntimeKind::AppleContainer => mixum_env_runtime_apple::backend(),
        });
    }
    registry
}

pub fn run() {
    augment_path();
    tauri::Builder::default()
        .manage(AppState::new())
        .setup(|app| {
            // Windows 没有 overlay 标题栏：去掉系统装饰，窗口控件由前端标题栏绘制。
            // Linux 保留系统装饰，避免无边框窗口丢失缩放与窗口管理器集成。
            #[cfg(target_os = "windows")]
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_decorations(false);
            }

            // 开发用：MIXUM_ART_AUTOTEST=<addr> + MIXUM_MAP_TOKEN（可选
            // MIXUM_MAP_FP）时自动扫描并打开第一个 surface，方便冒烟。
            if let Ok(addr) = std::env::var("MIXUM_ART_AUTOTEST") {
                if let Ok(token) = std::env::var("MIXUM_MAP_TOKEN") {
                    let fingerprint = std::env::var("MIXUM_MAP_FP").ok();
                    let handle = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        if let Ok(surfaces) = commands::list_surfaces(
                            addr.clone(),
                            token.clone(),
                            fingerprint.clone(),
                        )
                        .await
                        {
                            if let Some(surface) = surfaces.last() {
                                let _ = commands::open_surface_window(
                                    handle,
                                    commands::OpenSurfaceRequest {
                                        addr,
                                        token,
                                        fingerprint,
                                        surface: surface.id,
                                        title: surface.title.clone(),
                                        width: surface.width,
                                        height: surface.height,
                                    },
                                );
                            }
                        }
                    });
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::host_info,
            commands::runtime_status,
            commands::list_containers,
            commands::runtime_images,
            commands::set_agent_credential,
            commands::delete_agent_credential,
            commands::has_agent_credential,
            commands::load_agent_profiles,
            commands::save_agent_profile,
            commands::remove_agent_profile,
            commands::runtime_disk_usage,
            commands::runtime_logs,
            commands::runtime_prune,
            commands::runtime_pull_image,
            commands::runtime_load_image,
            commands::start_environment,
            commands::stop_environment,
            commands::remove_environment,
            commands::create_environment,
            commands::load_environment_connections,
            commands::connect_environment,
            commands::list_environment_surfaces,
            commands::open_environment_surface,
            commands::save_environment_connection,
            commands::remove_environment_connection,
            commands::environment_snapshot_tree,
            commands::environment_snapshot_create,
            commands::environment_snapshot_create_kind,
            commands::environment_container_spec,
            commands::environment_snapshot_restore,
            commands::environment_snapshot_fork,
            commands::environment_snapshot_export,
            commands::list_migrations,
            commands::environment_snapshot_import,
            commands::environment_migrate,
            commands::environment_migrate_new,
            commands::environment_run_command,
            commands::list_surfaces,
            commands::open_surface_window,
            commands::surface_input,
            commands::surface_takeover,
            commands::open_container_terminal,
            commands::container_terminal_input,
            commands::container_terminal_resize,
            commands::close_container_terminal,
            commands::remote_clipboard,
            commands::surface_put_file,
        ])
        .run(tauri::generate_context!())
        .expect("启动 mixum Agent Runtime 失败");
}

/// 从 Finder / 开始菜单启动时，进程拿不到登录 shell 的 PATH，
/// `/opt/homebrew/bin`、`/usr/local/bin` 这类目录会缺失，导致明明装了
/// docker/container 却探测不到。这里补上常见安装前缀。
#[cfg(unix)]
fn augment_path() {
    use std::path::PathBuf;

    let mut paths: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|value| std::env::split_paths(&value).collect())
        .unwrap_or_default();

    for dir in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
        let path = PathBuf::from(dir);
        if !paths.contains(&path) {
            paths.push(path);
        }
    }

    if let Ok(joined) = std::env::join_paths(paths) {
        std::env::set_var("PATH", joined);
    }
}

#[cfg(not(unix))]
fn augment_path() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_every_known_backend_in_order() {
        let registry = build_registry();
        assert_eq!(registry.len(), RuntimeKind::ALL.len());
        for kind in RuntimeKind::ALL {
            assert!(registry.get(kind).is_some(), "{kind} 未注册");
        }
    }
}
