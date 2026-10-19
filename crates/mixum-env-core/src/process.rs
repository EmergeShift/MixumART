//! 外部命令调用的统一入口。
//!
//! 规则（见 `docs/runtime-contract.md`）：
//! - 只走 argv，不经过 shell，避免引号与注入问题；
//! - 一律带超时，避免 daemon 挂起时 UI 卡死；
//! - 找不到可执行文件时返回 [`EnvError::NotInstalled`]，由后端补安装提示。

use std::process::Stdio;
use std::time::Duration;

use crate::error::EnvError;

/// 默认超时：探测与列表这类交互都要在用户可感知的时间内返回。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    /// 失败时转成 [`EnvError::CommandFailed`]，成功时返回 stdout。
    pub fn into_stdout(self, command: impl Into<String>) -> Result<String, EnvError> {
        if self.success {
            Ok(self.stdout)
        } else {
            Err(EnvError::CommandFailed {
                command: command.into(),
                stderr: self.stderr,
            })
        }
    }
}

/// 原始字节输出。wsl.exe 这类控制台工具输出 UTF-16LE，UTF-8 无损转换
/// 会破坏后续解码——由调用方取原始字节自行解码（见 runtime-contract.md §4）。
#[derive(Debug, Clone)]
pub struct RawCommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// 执行命令并收集输出；`program` 不存在时返回 NotInstalled，超时返回 Timeout。
pub async fn run(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<CommandOutput, EnvError> {
    let raw = run_raw(program, args, timeout).await?;
    Ok(CommandOutput {
        success: raw.success,
        stdout: String::from_utf8_lossy(&raw.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&raw.stderr).trim().to_string(),
    })
}

/// 执行命令并收集原始字节输出；超时与缺程序的语义同 [`run`]。
pub async fn run_raw(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<RawCommandOutput, EnvError> {
    let display = display_command(program, args);
    let future = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output();

    let output = match tokio::time::timeout(timeout, future).await {
        Err(_) => {
            return Err(EnvError::Timeout {
                command: display,
                seconds: timeout.as_secs(),
            })
        }
        Ok(Err(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(EnvError::NotInstalled {
                program: program.to_string(),
                hint: "确认已安装，且可执行文件位于 PATH 中".to_string(),
            })
        }
        Ok(Err(err)) => return Err(EnvError::Io(err)),
        Ok(Ok(output)) => output,
    };

    Ok(RawCommandOutput {
        success: output.status.success(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

pub fn display_command(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_missing_binary() {
        let err = run("mixum-env-definitely-missing", &[], Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(matches!(err, EnvError::NotInstalled { .. }));
    }

    #[tokio::test]
    async fn captures_stdout() {
        let (program, args): (&str, Vec<&str>) = if cfg!(windows) {
            ("cmd", vec!["/c", "echo", "hello"])
        } else {
            ("/bin/echo", vec!["hello"])
        };
        let out = run(program, &args, Duration::from_secs(2)).await.unwrap();
        assert!(out.success);
        assert_eq!(out.stdout, "hello");
    }
}
