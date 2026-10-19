//! WSL 2 后端（Windows）——**占位实现**。
//!
//! 这个 crate 由 WSL 2 兼容小组并行开发。骨架阶段刻意只保留接口：
//! 已注册到 GUI，能显示「仅 Windows / 接入中」，但没有任何真实调用，
//! 以免两边同时改同一份代码。
//!
//! 接入时需要做的（也只在这个 crate 内）：
//! 1. `probe()`：调用 `wsl.exe --status` / `--list --verbose` 判断发行版与状态；
//! 2. `list_containers()`：读取 mixumOS 发行版中的容器清单（具体形态由该组定）；
//! 3. 写操作：按 [`mixum_env_core::runtime::RuntimeBackend`] 契约实现。
//!
//! 唯一可能需要跨组协调的改动：`RuntimeKind` 增加/调整字段，
//! 或 core 错误类型不够用——那要先改 `mixum-env-core` 的契约文档。

use std::sync::Arc;

use async_trait::async_trait;
use mixum_env_core::container::Container;
use mixum_env_core::error::EnvError;
use mixum_env_core::runtime::{RuntimeBackend, RuntimeKind, RuntimeStatus};

const PENDING: &str = "WSL 2 后端由并行小组接入中（crates/mixum-env-runtime-wsl2）";

/// 应用装配入口。实现替换后调用方无需改动。
pub fn backend() -> Arc<dyn RuntimeBackend> {
    Arc::new(Wsl2Backend)
}

#[derive(Debug, Default)]
pub struct Wsl2Backend;

#[async_trait]
impl RuntimeBackend for Wsl2Backend {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Wsl2
    }

    async fn probe(&self) -> RuntimeStatus {
        if !cfg!(target_os = "windows") {
            return RuntimeStatus::Unsupported {
                reason: "WSL 2 仅在 Windows 宿主可用".to_string(),
            };
        }
        RuntimeStatus::Unsupported {
            reason: PENDING.to_string(),
        }
    }

    async fn list_containers(&self) -> Result<Vec<Container>, EnvError> {
        Err(EnvError::Unsupported {
            runtime: RuntimeKind::Wsl2,
            reason: PENDING.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reports_platform_or_pending() {
        let status = backend().probe().await;
        match status {
            RuntimeStatus::Unsupported { reason } => assert!(!reason.is_empty()),
            other => panic!("占位后端不应返回 {other:?}"),
        }
    }
}
