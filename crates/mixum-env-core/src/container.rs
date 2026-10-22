//! 容器与创建规格：三个 runtime 共用的一套最小视图。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 容器状态。各 runtime 的原始字符串经 [`ContainerState::from_raw`] 归一。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainerState {
    Created,
    Running,
    Paused,
    Stopped,
    Unknown,
}

impl ContainerState {
    /// 把 Docker / Apple Container 的状态字符串归一。
    ///
    /// 未知值一律落到 [`ContainerState::Unknown`]，不报错：状态字段只用于展示，
    /// 不应因为 runtime 新增了一个状态就让整个列表失败。
    pub fn from_raw(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "created" | "creating" => Self::Created,
            "running" | "up" => Self::Running,
            "paused" => Self::Paused,
            "exited" | "stopped" | "dead" | "removing" => Self::Stopped,
            _ => Self::Unknown,
        }
    }
}

/// 宿主机端口到容器端口的映射。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortMapping {
    pub host: u16,
    pub container: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_protocol() -> String {
    "tcp".to_string()
}

impl PortMapping {
    pub fn tcp(host: u16, container: u16) -> Self {
        Self {
            host,
            container,
            protocol: default_protocol(),
        }
    }
}

/// 绑定挂载。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mount {
    /// `bind` (default) or `volume`.
    #[serde(default = "default_mount_kind")]
    pub kind: String,
    pub source: String,
    pub target: String,
    #[serde(default)]
    pub read_only: bool,
}

fn default_mount_kind() -> String {
    "bind".into()
}

/// 列表用的容器摘要。刻意不含 runtime 私有字段。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Container {
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: ContainerState,
    /// 原始状态文案（如 `Up 19 hours`），仅用于展示。
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub ports: Vec<PortMapping>,
}

/// 创建容器的请求。字段是三个 runtime 的交集，不追求完整覆盖 CLI 能力。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSpec {
    pub name: String,
    pub image: String,
    #[serde(default)]
    pub ports: Vec<PortMapping>,
    #[serde(default)]
    pub mounts: Vec<Mount>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub command: Vec<String>,
}

/// `inspect` 的完整结果：可重建的 spec + 镜像 digest（不塞进 image 引用，
/// 避免部分 runtime 不认 `repo:tag@sha256:...` 的拉取格式）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectedContainer {
    pub spec: CreateSpec,
    #[serde(default)]
    pub image_digest: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_known_states() {
        assert_eq!(ContainerState::from_raw("running"), ContainerState::Running);
        assert_eq!(ContainerState::from_raw("Exited"), ContainerState::Stopped);
        assert_eq!(ContainerState::from_raw("created"), ContainerState::Created);
        assert_eq!(ContainerState::from_raw("paused"), ContainerState::Paused);
        assert_eq!(
            ContainerState::from_raw("whatever"),
            ContainerState::Unknown
        );
    }

    #[test]
    fn serializes_as_camel_case() {
        let container = Container {
            id: "abc".into(),
            name: "map-node".into(),
            image: "mixumos:0.1.0".into(),
            state: ContainerState::Running,
            status: Some("Up 2 hours".into()),
            created_at: Some("2026-10-18T13:02:45Z".into()),
            ports: vec![PortMapping::tcp(17601, 7601)],
        };
        let json = serde_json::to_value(&container).unwrap();
        assert_eq!(json["createdAt"], "2026-10-18T13:02:45Z");
        assert_eq!(
            json["ports"][0],
            serde_json::json!({
                "host": 17601,
                "container": 7601,
                "protocol": "tcp",
            })
        );
        assert_eq!(json["state"], "running");
    }
}
