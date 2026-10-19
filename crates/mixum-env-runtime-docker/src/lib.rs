//! Docker 后端。
//!
//! 骨架阶段通过 `docker` CLI 实现探测与列举；后续可换成 Engine API
//! （Unix socket / npipe），对外契约不变，替换范围仅限本 crate。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mixum_env_core::container::{Container, ContainerState, PortMapping};
use mixum_env_core::error::EnvError;
use mixum_env_core::process;
use mixum_env_core::runtime::{RuntimeBackend, RuntimeKind, RuntimeStatus};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const DAEMON_HINT: &str = "启动 Docker Desktop 或 colima 后点刷新";

/// 应用装配入口。
pub fn backend() -> Arc<dyn RuntimeBackend> {
    Arc::new(DockerBackend)
}

#[derive(Debug, Default)]
pub struct DockerBackend;

#[async_trait]
impl RuntimeBackend for DockerBackend {
    fn kind(&self) -> RuntimeKind {
        RuntimeKind::Docker
    }

    async fn probe(&self) -> RuntimeStatus {
        let args = ["version", "--format", "{{.Server.Version}}"];
        match process::run("docker", &args, PROBE_TIMEOUT).await {
            Ok(out) if out.success => {
                let version = (!out.stdout.is_empty()).then_some(out.stdout);
                RuntimeStatus::ready(version)
            }
            Ok(out) => RuntimeStatus::NotRunning {
                hint: first_line(&out.stderr).unwrap_or_else(|| DAEMON_HINT.to_string()),
            },
            Err(EnvError::NotInstalled { .. }) => RuntimeStatus::NotInstalled {
                hint: "安装 Docker Desktop 或 Docker Engine：https://docs.docker.com/get-docker/"
                    .to_string(),
            },
            Err(err) => RuntimeStatus::NotRunning {
                hint: err.to_string(),
            },
        }
    }

    async fn list_containers(&self) -> Result<Vec<Container>, EnvError> {
        let args = ["ps", "-a", "--no-trunc", "--format", "{{json .}}"];
        let out = process::run("docker", &args, LIST_TIMEOUT).await?;
        let stdout = out.into_stdout("docker ps")?;
        parse_ps(&stdout)
    }
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

#[derive(Debug, serde::Deserialize)]
struct DockerPsEntry {
    #[serde(rename = "ID")]
    id: String,
    #[serde(rename = "Names")]
    names: String,
    #[serde(rename = "Image")]
    image: String,
    #[serde(rename = "State")]
    state: String,
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "Ports", default)]
    ports: String,
    #[serde(rename = "CreatedAt", default)]
    created_at: String,
}

impl DockerPsEntry {
    fn into_container(self) -> Container {
        Container {
            id: self.id,
            name: self.names,
            image: self.image,
            state: ContainerState::from_raw(&self.state),
            status: non_empty(self.status),
            created_at: non_empty(self.created_at),
            ports: parse_ports(&self.ports),
        }
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

/// `docker ps` 每行一个 JSON 对象；空输出表示没有容器。
fn parse_ps(stdout: &str) -> Result<Vec<Container>, EnvError> {
    let mut containers = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let entry: DockerPsEntry = serde_json::from_str(line)
            .map_err(|err| EnvError::Parse(format!("docker ps 输出无法解析：{err}")))?;
        containers.push(entry.into_container());
    }
    Ok(containers)
}

/// 解析 `docker ps` 的 Ports 列，例如
/// `0.0.0.0:17611->7601/tcp, [::]:17611->7601/tcp`。
/// 未发布的端口（如 `7601/tcp`）会被忽略，同一映射只保留一条。
fn parse_ports(raw: &str) -> Vec<PortMapping> {
    let mut ports: Vec<PortMapping> = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        let Some((host_side, container_side)) = part.split_once("->") else {
            continue;
        };
        let Some(container) = parse_port(container_side) else {
            continue;
        };
        let Some(host) = host_side
            .rsplit(':')
            .next()
            .and_then(|s| s.parse::<u16>().ok())
        else {
            continue;
        };
        let protocol = container_side
            .split_once('/')
            .map(|(_, proto)| proto.trim().to_ascii_lowercase())
            .unwrap_or_else(|| "tcp".to_string());
        let mapping = PortMapping {
            host,
            container,
            protocol,
        };
        if !ports.iter().any(|existing| existing == &mapping) {
            ports.push(mapping);
        }
    }
    ports
}

fn parse_port(side: &str) -> Option<u16> {
    let port = side.trim().split('/').next()?.trim();
    port.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dual_stack_ports() {
        let ports = parse_ports("0.0.0.0:17611->7601/tcp, [::]:17611->7601/tcp");
        assert_eq!(ports, vec![PortMapping::tcp(17611, 7601)]);
    }

    #[test]
    fn ignores_unpublished_ports() {
        assert!(parse_ports("7601/tcp").is_empty());
        assert!(parse_ports("").is_empty());
    }

    #[test]
    fn parses_ps_line() {
        let line = r#"{"Command":"\"/entrypoint.sh\"","CreatedAt":"2026-10-18 20:51:10 +0800 CST","ID":"1341e631b415","Image":"archlinux:mixum-aarch64","Names":"mixum-build-aarch64","Ports":"0.0.0.0:17611->7601/tcp","State":"running","Status":"Up 19 hours"}"#;
        let containers = parse_ps(line).unwrap();
        assert_eq!(containers.len(), 1);
        let container = &containers[0];
        assert_eq!(container.name, "mixum-build-aarch64");
        assert_eq!(container.state, ContainerState::Running);
        assert_eq!(container.ports, vec![PortMapping::tcp(17611, 7601)]);
    }

    /// 真实后端冒烟：需要本机 Docker daemon 可用。
    /// `cargo test -p mixum-env-runtime-docker -- --ignored`
    #[tokio::test]
    #[ignore = "需要本机 Docker daemon"]
    async fn live_probe_and_list() {
        let backend = backend();
        let status = backend.probe().await;
        assert!(status.is_ready(), "docker 不可用：{status:?}");
        let containers = backend.list_containers().await.unwrap();
        println!("docker 容器数：{}", containers.len());
    }
}
