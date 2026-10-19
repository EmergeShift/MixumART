//! Apple Container 后端（macOS 26+，`container` CLI 1.5+）。
//!
//! 参考实测记录：mixumos/docs/apple-container.md。
//! 本 crate 只负责把 CLI 的输出翻译成 core 类型；镜像导入、内核设置等
//! 在后续里程碑补，不在这里预埋。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mixum_env_core::container::{Container, ContainerState, PortMapping};
use mixum_env_core::error::EnvError;
use mixum_env_core::process;
use mixum_env_core::runtime::{RuntimeBackend, RuntimeKind, RuntimeStatus};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const INSTALL_HINT: &str = "brew install container，然后执行 container system start";
const START_HINT: &str = "执行 container system start 启动容器服务";

/// 应用装配入口。
pub fn backend() -> Arc<dyn RuntimeBackend> {
    Arc::new(AppleContainerBackend)
}

#[derive(Debug, Default)]
pub struct AppleContainerBackend;

#[async_trait]
impl RuntimeBackend for AppleContainerBackend {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::AppleContainer
    }

    async fn probe(&self) -> RuntimeStatus {
        let args = ["system", "status", "--format", "json"];
        match process::run("container", &args, PROBE_TIMEOUT).await {
            Ok(out) if out.success => match serde_json::from_str::<SystemStatus>(&out.stdout) {
                Ok(status) if status.is_running() => {
                    RuntimeStatus::ready(status.client.and_then(|c| c.version))
                }
                Ok(_) => RuntimeStatus::NotRunning {
                    hint: START_HINT.to_string(),
                },
                Err(err) => RuntimeStatus::NotRunning {
                    hint: format!("无法解析 container system status：{err}"),
                },
            },
            Ok(out) => RuntimeStatus::NotRunning {
                hint: first_line(&out.stderr).unwrap_or_else(|| START_HINT.to_string()),
            },
            Err(EnvError::NotInstalled { .. }) => RuntimeStatus::NotInstalled {
                hint: INSTALL_HINT.to_string(),
            },
            Err(err) => RuntimeStatus::NotRunning {
                hint: err.to_string(),
            },
        }
    }

    async fn list_containers(&self) -> Result<Vec<Container>, EnvError> {
        let args = ["list", "--all", "--format", "json"];
        let out = process::run("container", &args, LIST_TIMEOUT).await?;
        let stdout = out.into_stdout("container list --all")?;
        parse_list(&stdout)
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

#[derive(Debug, serde::Deserialize)]
struct SystemStatus {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    client: Option<ClientInfo>,
}

impl SystemStatus {
    fn is_running(&self) -> bool {
        self.status.as_deref() == Some("running")
    }
}

#[derive(Debug, serde::Deserialize)]
struct ClientInfo {
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleEntry {
    id: String,
    configuration: AppleConfiguration,
    #[serde(default)]
    status: Option<AppleStatus>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleConfiguration {
    #[serde(default)]
    id: String,
    image: AppleImage,
    #[serde(default)]
    creation_date: Option<String>,
    #[serde(default)]
    published_ports: Vec<ApplePort>,
}

#[derive(Debug, serde::Deserialize)]
struct AppleImage {
    reference: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppleStatus {
    #[serde(default)]
    state: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApplePort {
    container_port: u16,
    host_port: u16,
    #[serde(default = "default_protocol")]
    proto: String,
}

fn default_protocol() -> String {
    "tcp".to_string()
}

fn parse_list(stdout: &str) -> Result<Vec<Container>, EnvError> {
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    let entries: Vec<AppleEntry> = serde_json::from_str(stdout)
        .map_err(|err| EnvError::Parse(format!("container list 输出无法解析：{err}")))?;
    Ok(entries
        .into_iter()
        .map(AppleEntry::into_container)
        .collect())
}

impl AppleEntry {
    fn into_container(self) -> Container {
        let name = if self.configuration.id.is_empty() {
            self.id.clone()
        } else {
            self.configuration.id.clone()
        };
        let state = self
            .status
            .as_ref()
            .map(|status| ContainerState::from_raw(&status.state))
            .unwrap_or(ContainerState::Unknown);
        let ports = self
            .configuration
            .published_ports
            .into_iter()
            .map(|port| PortMapping {
                host: port.host_port,
                container: port.container_port,
                protocol: if port.proto.is_empty() {
                    default_protocol()
                } else {
                    port.proto
                },
            })
            .collect();

        Container {
            id: self.id,
            name,
            image: self.configuration.image.reference,
            state,
            status: None,
            created_at: self.configuration.creation_date,
            ports,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {
        "id": "map-node",
        "configuration": {
          "id": "map-node",
          "creationDate": "2026-10-18T13:02:44Z",
          "image": { "reference": "127.0.0.1:15000/mixumos:fix1" },
          "publishedPorts": [
            {"containerPort": 7601, "count": 1, "hostAddress": "127.0.0.1", "hostPort": 17601, "proto": "tcp"}
          ]
        },
        "status": {"state": "running", "startedDate": "2026-10-18T13:02:45Z"}
      },
      {
        "id": "8ad8436a-0378",
        "configuration": {
          "id": "8ad8436a-0378",
          "creationDate": "2026-10-18T10:45:25Z",
          "image": { "reference": "docker.io/library/alpine:3.20" },
          "publishedPorts": []
        },
        "status": {"state": "stopped"}
      }
    ]"#;

    #[test]
    fn parses_running_and_stopped() {
        let containers = parse_list(SAMPLE).unwrap();
        assert_eq!(containers.len(), 2);

        assert_eq!(containers[0].name, "map-node");
        assert_eq!(containers[0].state, ContainerState::Running);
        assert_eq!(containers[0].ports, vec![PortMapping::tcp(17601, 7601)]);

        assert_eq!(containers[1].state, ContainerState::Stopped);
        assert!(containers[1].ports.is_empty());
    }

    #[test]
    fn empty_output_is_empty_list() {
        assert!(parse_list("").unwrap().is_empty());
        assert!(parse_list("[]").unwrap().is_empty());
    }

    #[test]
    fn running_status_shape() {
        let status: SystemStatus =
            serde_json::from_str(r#"{"status":"running","client":{"version":"1.5.0"}}"#).unwrap();
        assert!(status.is_running());
        assert_eq!(status.client.unwrap().version.as_deref(), Some("1.5.0"));
    }

    /// 真实后端冒烟：需要本机 Apple Container 服务可用。
    /// `cargo test -p mixum-env-runtime-apple -- --ignored`
    #[tokio::test]
    #[ignore = "需要本机 Apple Container 服务"]
    async fn live_probe_and_list() {
        let backend = backend();
        let status = backend.probe().await;
        assert!(status.is_ready(), "container 服务不可用：{status:?}");
        let containers = backend.list_containers().await.unwrap();
        println!("Apple Container 容器数：{}", containers.len());
        assert!(!containers.is_empty(), "本机应有已创建的容器");
    }
}
