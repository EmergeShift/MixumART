//! Bridge between the desktop init UI and the shared setup planner.

use std::path::PathBuf;
use std::process::Command;

use mixum_env_core::RuntimeKind;
use mixum_setup::{InstallAction, SetupPlan, SetupRequest, SetupResult, UpdateInfo};
use tauri::{AppHandle, Manager, State};

use crate::AppState;

fn data_dir() -> PathBuf {
    mixum_setup::default_data_dir()
}

fn launcher(app: &AppHandle) -> String {
    if let Ok(resources) = app.path().resource_dir() {
        let name = if cfg!(target_os = "windows") {
            "mixum.exe"
        } else {
            "mixum"
        };
        let bundled = resources.join("bin").join(name);
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
    let action = mixum_setup::install_action(kind).ok_or_else(|| {
        format!(
            "{} cannot be installed automatically on this platform",
            kind.display_name()
        )
    })?;
    run_desktop_action(action).await
}

#[tauri::command]
pub async fn setup_start_runner(kind: RuntimeKind) -> Result<String, String> {
    let action = mixum_setup::start_action(kind).ok_or_else(|| {
        format!(
            "{} has no automatic start action on this platform",
            kind.display_name()
        )
    })?;
    run_desktop_action(action).await
}

async fn run_desktop_action(action: InstallAction) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || execute_action(action))
        .await
        .map_err(|error| format!("administrator action task failed: {error}"))?
}

fn execute_action(action: InstallAction) -> Result<String, String> {
    if action.requires_admin {
        run_elevated(&action)
    } else {
        mixum_setup::run_action(action).map_err(|error| error.to_string())
    }
}

#[cfg(any(target_os = "windows", all(unix, not(target_os = "macos"))))]
fn strip_sudo(command: &[String]) -> Vec<String> {
    if command.first().is_some_and(|value| value == "sudo") {
        command[1..].to_vec()
    } else {
        command.to_vec()
    }
}

#[cfg(target_os = "macos")]
fn shell_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn apple_script_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(target_os = "windows")]
fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn output_text(output: std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

fn cancelled(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    text.contains("cancel")
        || text.contains("user canceled")
        || text.contains("operation was canceled")
        || text.contains("已取消")
}

#[cfg(target_os = "macos")]
fn run_elevated(action: &InstallAction) -> Result<String, String> {
    let command = action
        .command
        .iter()
        .map(|value| shell_quote(value))
        .collect::<Vec<_>>()
        .join(" ");
    let script = format!(
        "do shell script {} with prompt {} with administrator privileges",
        apple_script_quote(&command),
        apple_script_quote("mixum 需要管理员权限来安装或启动运行时")
    );
    let output = Command::new("osascript")
        .args(["-e", &script])
        .output()
        .map_err(|error| format!("cannot open the macOS administrator prompt: {error}"))?;
    if !output.status.success() {
        let text = output_text(output);
        return if cancelled(&text) {
            Err("已取消管理员授权。".to_string())
        } else {
            Err(format!("管理员操作失败：{text}"))
        };
    }
    let text = output_text(output);
    Ok(if text.is_empty() {
        action.note.clone()
    } else {
        format!("{text}\n{}", action.note)
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn run_elevated(action: &InstallAction) -> Result<String, String> {
    let args = strip_sudo(&action.command);
    let Some(program) = args.first() else {
        return Err("安装命令为空。".to_string());
    };
    let output = Command::new("pkexec")
        .args([program])
        .args(&args[1..])
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "未找到 pkexec。请安装 polkit，或在终端手动运行：\n{}",
                    action.command.join(" ")
                )
            } else {
                format!("无法打开系统授权窗口：{error}")
            }
        })?;
    if !output.status.success() {
        let text = output_text(output);
        return if cancelled(&text) {
            Err("已取消管理员授权。".to_string())
        } else {
            Err(format!("管理员操作失败：{text}"))
        };
    }
    let text = output_text(output);
    Ok(if text.is_empty() {
        action.note.clone()
    } else {
        format!("{text}\n{}", action.note)
    })
}

#[cfg(target_os = "windows")]
fn run_elevated(action: &InstallAction) -> Result<String, String> {
    let args = strip_sudo(&action.command);
    let Some(program) = args.first() else {
        return Err("安装命令为空。".to_string());
    };
    let arguments = args[1..]
        .iter()
        .map(|value| powershell_quote(value))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$p = Start-Process -FilePath {} -ArgumentList @({}) -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        powershell_quote(program),
        arguments
    );
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|error| format!("cannot open the Windows UAC prompt: {error}"))?;
    if !output.status.success() {
        let text = output_text(output);
        return if cancelled(&text) {
            Err("已取消管理员授权。".to_string())
        } else {
            Err(format!("管理员操作失败：{text}"))
        };
    }
    let text = output_text(output);
    Ok(if text.is_empty() {
        action.note.clone()
    } else {
        format!("{text}\n{}", action.note)
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
fn run_elevated(_action: &InstallAction) -> Result<String, String> {
    Err("当前平台没有可用的 GUI 管理员授权方式。".to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn quotes_shell_and_applescript() {
        assert_eq!(shell_quote("plain"), "'plain'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(apple_script_quote("a\\\"b"), "\"a\\\\\\\"b\"");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn strips_sudo_before_pkexec() {
        assert_eq!(
            strip_sudo(&["sudo".into(), "apt-get".into(), "install".into()]),
            vec!["apt-get", "install"]
        );
        assert_eq!(strip_sudo(&["true".into()]), vec!["true"]);
    }

    #[test]
    fn recognizes_cancellation() {
        assert!(cancelled("User canceled."));
        assert!(cancelled("已取消"));
        assert!(!cancelled("permission denied"));
    }
}
