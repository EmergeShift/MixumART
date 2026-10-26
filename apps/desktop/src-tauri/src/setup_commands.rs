//! Bridge between the desktop init UI and the shared setup planner.

use std::path::PathBuf;

use mixum_env_core::RuntimeKind;
use mixum_setup::{SetupPlan, SetupRequest, SetupResult, UpdateInfo};
use tauri::{AppHandle, Manager, State};

use crate::AppState;

fn data_dir() -> PathBuf {
    mixum_setup::default_data_dir()
}

fn launcher(app: &AppHandle) -> String {
    if let Ok(resources) = app.path().resource_dir() {
        let bundled = resources.join("bin").join("mixum");
        if bundled.is_file() {
            return bundled.display().to_string();
        }
    }
    std::env::var("MIXUM_CLI").unwrap_or_else(|_| "mixum".to_string())
}

#[tauri::command]
pub async fn setup_plan(app: AppHandle, state: State<'_, AppState>) -> Result<SetupPlan, String> {
    mixum_setup::plan(&state.registry, &data_dir(), &launcher(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn setup_apply(
    app: AppHandle,
    state: State<'_, AppState>,
    request: SetupRequest,
) -> Result<SetupResult, String> {
    mixum_setup::apply(&state.registry, &data_dir(), &launcher(&app), request)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn setup_install_runner(kind: RuntimeKind) -> Result<String, String> {
    mixum_setup::install_runner(kind)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn setup_start_runner(kind: RuntimeKind) -> Result<String, String> {
    mixum_setup::start_runner(kind)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn setup_check_update() -> Result<Option<UpdateInfo>, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    tauri::async_runtime::spawn_blocking(move || {
        mixum_setup::update::check_update(&data_dir(), &current)
    })
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn setup_settings() -> Result<mixum_setup::SetupSettings, String> {
    mixum_setup::load_settings(&data_dir()).map_err(|error| error.to_string())
}
