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
        || text.contains("取消")
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

/// Windows 提权脚本回传的两个标记。
///
/// PowerShell 5.1 在 stdout 被重定向时按控制台代码页输出（zh-CN 是 GBK），
/// 直接 `from_utf8_lossy` 会得到乱码，`cancelled()` 也就匹配不到“已取消”，
/// 取消会被当成成功。因此脚本只回传 ASCII 标记，错误文本用
/// base64(UTF-8) 包裹，绕开代码页——比强设 `[Console]::OutputEncoding` 更可靠。
#[cfg(target_os = "windows")]
const ELEVATION_CANCELLED: &str = "MIXUM_ELEVATION_CANCELLED";
#[cfg(target_os = "windows")]
const ELEVATION_FAILED: &str = "MIXUM_ELEVATION_FAILED:";

#[cfg(target_os = "windows")]
#[derive(Debug, PartialEq, Eq)]
enum ElevationOutcome {
    Cancelled,
    Failed(String),
}

#[cfg(target_os = "windows")]
fn elevation_outcome(text: &str) -> Option<ElevationOutcome> {
    use base64::Engine as _;
    for line in text.lines() {
        let line = line.trim();
        if line == ELEVATION_CANCELLED {
            return Some(ElevationOutcome::Cancelled);
        }
        if let Some(payload) = line.strip_prefix(ELEVATION_FAILED) {
            let detail = base64::engine::general_purpose::STANDARD
                .decode(payload.trim())
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .unwrap_or_else(|| "提权脚本返回了无法解析的错误详情。".to_string());
            return Some(ElevationOutcome::Failed(detail));
        }
    }
    None
}

/// `Start-Process` 失败是非终止错误：不设 `-ErrorAction` 时 `$p` 保持 `$null`，
/// 而 `exit $null.ExitCode` 实际退出 0——这就是“取消被当成成功”的根因。
/// 脚本因此改成：`$ErrorActionPreference='Stop'` + try/catch，显式判空，
/// 失败一律退非零。
///
/// 取消的识别：PowerShell 5.1 抛的是**没有 `InnerException` 的裸
/// `InvalidOperationException`**（HResult 0x80131509），1223 只出现在消息文本里，
/// 所以先扫异常链上的 `Win32Exception.NativeErrorCode`（未来版本可能暴露），
/// 再退回“与系统本地化的 ERROR_CANCELLED 文案比对”——不硬编码中英文。
#[cfg(target_os = "windows")]
const ELEVATION_SCRIPT: &str = r#"$ErrorActionPreference = 'Stop'
try {
  $p = Start-Process -FilePath __MIXUM_PROGRAM__ -ArgumentList @(__MIXUM_ARGUMENTS__) -Verb RunAs -Wait -PassThru
} catch {
  $native = 0
  $cause = $_.Exception
  while ($null -ne $cause) {
    if ($cause -is [System.ComponentModel.Win32Exception]) { $native = $cause.NativeErrorCode; break }
    $cause = $cause.InnerException
  }
  $cancelText = (New-Object System.ComponentModel.Win32Exception 1223).Message
  $message = [string]$_.Exception.Message
  if ($native -eq 1223 -or ($cancelText -and $message.Contains($cancelText))) {
    Write-Output 'MIXUM_ELEVATION_CANCELLED'
    exit 1223
  }
  $detail = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($message))
  Write-Output ('MIXUM_ELEVATION_FAILED:' + $detail)
  exit 1
}
if ($null -eq $p) {
  $detail = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('Start-Process returned no process'))
  Write-Output ('MIXUM_ELEVATION_FAILED:' + $detail)
  exit 1
}
exit $p.ExitCode"#;

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
    let script = ELEVATION_SCRIPT
        .replace("__MIXUM_PROGRAM__", &powershell_quote(program))
        .replace("__MIXUM_ARGUMENTS__", &arguments);
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|error| format!("cannot open the Windows UAC prompt: {error}"))?;
    let succeeded = output.status.success();
    let code = output.status.code();
    let text = output_text(output);
    if !succeeded {
        // 提权进程自己失败时拿不到它的 stderr（ShellExecute 不转发句柄），
        // 所以这里只能报退出码并给出可手动执行的命令，不展示可能乱码的文本。
        return Err(elevation_error(&text, code, &action.command.join(" ")));
    }
    Ok(if text.is_empty() {
        action.note.clone()
    } else {
        format!("{text}\n{}", action.note)
    })
}

#[cfg(target_os = "windows")]
fn elevation_error(text: &str, code: Option<i32>, manual: &str) -> String {
    match elevation_outcome(text) {
        Some(ElevationOutcome::Cancelled) => "已取消管理员授权。".to_string(),
        // 兜底：脚本已把异常原文按 UTF-8 解回来了，万一某个 locale 下
        // `Win32Exception(1223).Message` 与实际文案不一致，这里仍能识别取消。
        Some(ElevationOutcome::Failed(detail)) if cancelled(&detail) => {
            "已取消管理员授权。".to_string()
        }
        Some(ElevationOutcome::Failed(detail)) => format!("管理员操作失败：{detail}"),
        None => match code {
            Some(code) => format!(
                "管理员操作失败：提权进程退出码 {code}。可在管理员 PowerShell 中手动运行：\n{manual}"
            ),
            None => format!(
                "管理员操作失败：提权进程被终止。可在管理员 PowerShell 中手动运行：\n{manual}"
            ),
        },
    }
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

    #[cfg(target_os = "windows")]
    #[test]
    fn parses_elevation_markers() {
        use base64::Engine as _;

        assert_eq!(
            elevation_outcome("MIXUM_ELEVATION_CANCELLED"),
            Some(ElevationOutcome::Cancelled)
        );
        assert_eq!(
            elevation_outcome("  MIXUM_ELEVATION_CANCELLED  \r\n"),
            Some(ElevationOutcome::Cancelled)
        );
        // 判定只依赖 ASCII 标记：前面的 GBK→UTF-8 乱码不影响结果。
        assert_eq!(
            elevation_outcome("锟斤拷\nMIXUM_ELEVATION_CANCELLED"),
            Some(ElevationOutcome::Cancelled)
        );
        let detail = base64::engine::general_purpose::STANDARD.encode("拒绝访问。".as_bytes());
        assert_eq!(
            elevation_outcome(&format!("{ELEVATION_FAILED}{detail}")),
            Some(ElevationOutcome::Failed("拒绝访问。".to_string()))
        );
        assert!(matches!(
            elevation_outcome(&format!("{ELEVATION_FAILED}not-base64!!")),
            Some(ElevationOutcome::Failed(_))
        ));
        assert_eq!(elevation_outcome(""), None);
        assert_eq!(elevation_outcome("some unrelated error"), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn elevation_script_guards_start_process() {
        // 根因回归：取消时 $p 为空 → 旧脚本 `exit $p.ExitCode` 退出 0（假成功）。
        let script = ELEVATION_SCRIPT
            .replace("__MIXUM_PROGRAM__", &powershell_quote("wsl.exe"))
            .replace("__MIXUM_ARGUMENTS__", &powershell_quote("--install"));
        assert!(script.contains("$ErrorActionPreference = 'Stop'"));
        assert!(script.contains("try {"));
        assert!(script.contains("catch {"));
        assert!(script.contains("if ($null -eq $p)"));
        assert!(script.contains("$native -eq 1223"));
        // 真实异常是裸 InvalidOperationException（无 InnerException），1223 只在文案里，
        // 因此必须保留“系统本地化文案”这条判定路径。
        assert!(script.contains("Win32Exception 1223"));
        assert!(script.contains("$message.Contains($cancelText)"));
        assert!(script.contains("-FilePath 'wsl.exe' -ArgumentList @('--install')"));
        assert!(script.trim_end().ends_with("exit $p.ExitCode"));
        // 占位符必须全部替换掉，否则 PowerShell 会把 `__MIXUM_*__` 当成命令。
        assert!(!script.contains("__MIXUM_"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn maps_elevation_failure_to_message() {
        use base64::Engine as _;

        let manual = "wsl.exe --install";
        // 脚本标记 -> 取消
        assert_eq!(
            elevation_error("MIXUM_ELEVATION_CANCELLED", Some(1223), manual),
            "已取消管理员授权。"
        );
        // 兜底：标记是 FAILED，但解回来的原文就是用户取消
        let detail = base64::engine::general_purpose::STANDARD
            .encode("This command cannot be run due to the error: The operation was canceled by the user.");
        assert_eq!(
            elevation_error(&format!("MIXUM_ELEVATION_FAILED:{detail}"), Some(1), manual),
            "已取消管理员授权。"
        );
        // 真正的失败不能被吞成取消
        let denied = base64::engine::general_purpose::STANDARD.encode("拒绝访问。".as_bytes());
        assert_eq!(
            elevation_error(&format!("MIXUM_ELEVATION_FAILED:{denied}"), Some(1), manual),
            "管理员操作失败：拒绝访问。"
        );
        // 提权进程自己退非零（安装脚本失败）：不报假成功，给出退出码和手动命令
        let exit7 = elevation_error("", Some(7), manual);
        assert!(exit7.contains("退出码 7"), "{exit7}");
        assert!(exit7.contains(manual), "{exit7}");
        let killed = elevation_error("", None, manual);
        assert!(killed.contains("被终止"), "{killed}");
    }
}
