//! 统一错误类型。命令层的 `Result<T, String>` 由 Tauri 适配层转换。

use thiserror::Error;

use crate::runtime::RuntimeKind;

#[derive(Debug, Error)]
pub enum EnvError {
    #[error("{runtime} 在当前平台不可用：{reason}")]
    Unsupported {
        runtime: RuntimeKind,
        reason: String,
    },

    #[error("未找到 {program}，可能尚未安装：{hint}")]
    NotInstalled { program: String, hint: String },

    #[error("命令超时（{seconds}s）：{command}")]
    Timeout { command: String, seconds: u64 },

    #[error("命令执行失败：{command}\n{stderr}")]
    CommandFailed { command: String, stderr: String },

    #[error("无法解析 runtime 输出：{0}")]
    Parse(String),

    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),

    #[error("能力尚未实现：{0}")]
    Unimplemented(&'static str),
}

impl EnvError {
    /// 后端骨架阶段的占位返回值：契约已定，实现待补。
    pub fn unimplemented(what: &'static str) -> Self {
        Self::Unimplemented(what)
    }
}

impl From<serde_json::Error> for EnvError {
    fn from(err: serde_json::Error) -> Self {
        Self::Parse(err.to_string())
    }
}
