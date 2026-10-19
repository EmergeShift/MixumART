//! Tauri 应用壳：装配后端、注册命令、启动窗口。
//!
//! 业务逻辑不写在这里：
//! - 平台细节 → `crates/mixum-env-runtime-*`
//! - 命令实现 → `commands.rs`
//! - 界面 → `apps/desktop/src`

mod commands;

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use mixum_env_core::{BackendRegistry, RuntimeKind};
use mixum_env_surface::InputEvent;

/// 全局状态：后端注册表 + 每个 surface 窗口的输入/关闭通道。
pub struct AppState {
    pub registry: Arc<BackendRegistry>,
    pub surface_inputs: Mutex<HashMap<u32, tokio::sync::mpsc::UnboundedSender<InputEvent>>>,
    pub surface_closes: Mutex<HashMap<u32, tokio::sync::oneshot::Sender<()>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(build_registry()),
            surface_inputs: Mutex::new(HashMap::new()),
            surface_closes: Mutex::new(HashMap::new()),
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
            commands::list_surfaces,
            commands::open_surface_window,
            commands::surface_input,
        ])
        .run(tauri::generate_context!())
        .expect("启动 mixum Agent Env 失败");
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
