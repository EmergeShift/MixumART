//! 运行时后端契约：GUI 只认这一层，不认任何 CLI。
//!
//! 新增一个 runtime = 新增一个实现 [`RuntimeBackend`] 的 crate + 在
//! `RuntimeKind` 里加一个变体 + 在应用装配处注册。除此之外不改任何代码。

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::container::{Container, CreateSpec};
use crate::error::EnvError;

/// 宿主机上可用的容器运行时。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Docker,
    Wsl2,
    AppleContainer,
}

impl RuntimeKind {
    /// 注册顺序 = 界面上环境列表的顺序。
    pub const ALL: [RuntimeKind; 3] = [
        RuntimeKind::Docker,
        RuntimeKind::Wsl2,
        RuntimeKind::AppleContainer,
    ];

    /// 与 serde、前端 `RuntimeKind` 保持一致的稳定标识。
    pub const fn id(self) -> &'static str {
        match self {
            RuntimeKind::Docker => "docker",
            RuntimeKind::Wsl2 => "wsl2",
            RuntimeKind::AppleContainer => "apple_container",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            RuntimeKind::Docker => "Docker",
            RuntimeKind::Wsl2 => "WSL 2",
            RuntimeKind::AppleContainer => "Apple Container",
        }
    }
}

impl std::fmt::Display for RuntimeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.display_name())
    }
}

/// 一次探测的结果。前端按 `state` 做判别联合。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RuntimeStatus {
    /// 可用。`version` 可为空（CLI 不报告版本时）。
    Ready {
        version: Option<String>,
        detail: Option<String>,
    },
    /// 命令行工具不在 PATH 上。
    NotInstalled { hint: String },
    /// 工具在，但 daemon / VM 没起来。
    NotRunning { hint: String },
    /// 平台不支持，或后端尚未接入。
    Unsupported { reason: String },
}

impl RuntimeStatus {
    pub fn ready(version: Option<String>) -> Self {
        Self::Ready {
            version,
            detail: None,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

/// 一个 runtime 后端在界面上的完整状态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendStatus {
    pub kind: RuntimeKind,
    pub name: String,
    pub status: RuntimeStatus,
}

/// 所有 runtime 后端必须实现的契约。
///
/// 实现要求：
/// - `probe` 不能 panic，也不能因为 daemon 未启动而返回 `Err`——那是状态，不是错误；
/// - 所有外部调用走 [`crate::process::run`]，自带超时；
/// - 返回结构里不得出现 runtime 私有字段，扩展先改 core。
#[async_trait]
pub trait RuntimeBackend: Send + Sync {
    fn kind(&self) -> RuntimeKind;

    /// 探测可用性。UI 启动时对每个后端调用一次。
    async fn probe(&self) -> RuntimeStatus;

    /// 列出容器（含已停止的）。只读能力，骨架阶段必须实现。
    async fn list_containers(&self) -> Result<Vec<Container>, EnvError>;

    /// 以下写操作是契约预留：骨架阶段默认返回 Unimplemented，
    /// 由各后端负责人在后续里程碑实现（见 docs/architecture.md 路线图）。
    async fn create_container(&self, _spec: CreateSpec) -> Result<Container, EnvError> {
        Err(EnvError::unimplemented("create_container"))
    }

    async fn start_container(&self, _id: &str) -> Result<(), EnvError> {
        Err(EnvError::unimplemented("start_container"))
    }

    async fn stop_container(&self, _id: &str) -> Result<(), EnvError> {
        Err(EnvError::unimplemented("stop_container"))
    }

    async fn remove_container(&self, _id: &str) -> Result<(), EnvError> {
        Err(EnvError::unimplemented("remove_container"))
    }

    /// 镜像引用列表（Docker/Apple Container 支持；WSL2 无镜像概念）。
    async fn images(&self) -> Result<Vec<String>, EnvError> {
        Err(EnvError::unimplemented("images"))
    }

    /// 运行时磁盘占用的人类可读摘要。
    async fn disk_usage(&self) -> Result<String, EnvError> {
        Err(EnvError::unimplemented("disk_usage"))
    }

    /// 容器最近日志。WSL2 没有统一的容器日志接口，默认不支持。
    async fn logs(&self, _id: &str, _tail: usize) -> Result<Vec<String>, EnvError> {
        Err(EnvError::unimplemented("logs"))
    }

    /// 清理运行时未使用的数据；具体范围由后端决定，调用方必须先确认。
    async fn prune(&self) -> Result<String, EnvError> {
        Err(EnvError::unimplemented("prune"))
    }

    /// 拉取一个镜像引用。WSL2 没有镜像概念，默认不支持。
    async fn pull_image(&self, _reference: &str) -> Result<(), EnvError> {
        Err(EnvError::unimplemented("pull_image"))
    }

    /// 从本地 OCI/tar 归档导入镜像。
    async fn load_image(&self, _path: &str) -> Result<(), EnvError> {
        Err(EnvError::unimplemented("load_image"))
    }

    /// 读取容器里的 agentd admin token，用于 MART 本地自动配对。
    /// 这是本地 exec 特权；token 不会返回给 WebView/日志。
    async fn admin_token(&self, _id: &str) -> Result<String, EnvError> {
        Err(EnvError::unimplemented("admin_token"))
    }
}

/// 后端注册表。应用启动时装配一次，之后只读。
#[derive(Default)]
pub struct BackendRegistry {
    backends: Vec<Arc<dyn RuntimeBackend>>,
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, backend: Arc<dyn RuntimeBackend>) -> &mut Self {
        self.backends.push(backend);
        self
    }

    pub fn get(&self, kind: RuntimeKind) -> Option<Arc<dyn RuntimeBackend>> {
        self.backends.iter().find(|b| b.kind() == kind).cloned()
    }

    pub fn len(&self) -> usize {
        self.backends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }

    /// 逐个探测。数量固定为 3，串行即可；若将来后端变多再并发。
    pub async fn statuses(&self) -> Vec<BackendStatus> {
        let mut statuses = Vec::with_capacity(self.backends.len());
        for backend in &self.backends {
            statuses.push(BackendStatus {
                kind: backend.kind(),
                name: backend.kind().display_name().to_string(),
                status: backend.probe().await,
            });
        }
        statuses
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(RuntimeKind);

    #[async_trait]
    impl RuntimeBackend for Stub {
        fn kind(&self) -> RuntimeKind {
            self.0
        }

        async fn probe(&self) -> RuntimeStatus {
            RuntimeStatus::ready(Some("1.0.0".into()))
        }

        async fn list_containers(&self) -> Result<Vec<Container>, EnvError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn registry_probes_registered_backends() {
        let mut registry = BackendRegistry::new();
        registry.register(Arc::new(Stub(RuntimeKind::Docker)));
        registry.register(Arc::new(Stub(RuntimeKind::Wsl2)));

        let statuses = registry.statuses().await;
        assert_eq!(statuses.len(), 2);
        assert_eq!(statuses[0].kind, RuntimeKind::Docker);
        assert!(statuses[0].status.is_ready());
        assert_eq!(statuses[1].name, "WSL 2");
        assert!(registry.get(RuntimeKind::AppleContainer).is_none());
    }

    #[test]
    fn kind_ids_match_wire_format() {
        assert_eq!(RuntimeKind::AppleContainer.id(), "apple_container");
        assert_eq!(
            serde_json::to_value(RuntimeKind::AppleContainer).unwrap(),
            serde_json::json!("apple_container")
        );
    }
}
