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
