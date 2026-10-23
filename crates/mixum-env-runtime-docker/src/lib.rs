//! Docker 后端。
//!
//! 骨架阶段通过 `docker` CLI 实现探测与列举；后续可换成 Engine API
//! （Unix socket / npipe），对外契约不变，替换范围仅限本 crate。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mixum_env_core::container::{
    Container, ContainerState, CreateSpec, InspectedContainer, Mount, PortMapping,
};
use mixum_env_core::error::EnvError;
use mixum_env_core::process;
use mixum_env_core::runtime::{RuntimeBackend, RuntimeKind, RuntimeStatus};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const CREATE_TIMEOUT: Duration = Duration::from_secs(600);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
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

    async fn create_container(&self, spec: CreateSpec) -> Result<Container, EnvError> {
        let mut args: Vec<String> = vec!["create".into(), "--name".into(), spec.name.clone()];
        for port in &spec.ports {
            args.push("-p".into());
            args.push(format!(
                "127.0.0.1:{}:{}/{}",
                port.host, port.container, port.protocol
            ));
        }
        for mount in &spec.mounts {
            args.push("--mount".into());
            let mount_type = if mount.kind == "volume" {
                "volume"
            } else {
                "bind"
            };
            let mut value = format!(
                "type={mount_type},source={},target={}",
                mount.source, mount.target
            );
            if mount.read_only {
                value.push_str(",readonly");
            }
            args.push(value);
        }
        for (key, value) in &spec.env {
            args.push("-e".into());
            args.push(format!("{key}={value}"));
        }
        for cap in &spec.cap_add {
            args.push("--cap-add".into());
            args.push(cap.clone());
        }
        args.push(spec.image);
        args.extend(spec.command);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = process::run("docker", &refs, CREATE_TIMEOUT).await?;
        let id = out.into_stdout("docker create")?;
        let containers = self.list_containers().await?;
        containers
            .into_iter()
            .find(|container| container.id == id || container.name == spec.name)
            .ok_or_else(|| EnvError::Parse(format!("docker create 后找不到容器 {id}")))
    }

    async fn start_container(&self, id: &str) -> Result<(), EnvError> {
        let out = process::run("docker", &["start", id], WRITE_TIMEOUT).await?;
        out.into_stdout("docker start").map(|_| ())
    }

    async fn exec_capture(&self, id: &str, command: &[&str]) -> Result<String, EnvError> {
        let mut args: Vec<&str> = vec!["exec", id];
        args.extend_from_slice(command);
        let out = process::run("docker", &args, WRITE_TIMEOUT).await?;
        out.into_stdout("docker exec")
    }

    async fn inspect_container(&self, id: &str) -> Result<InspectedContainer, EnvError> {
        let out = process::run("docker", &["inspect", id], LIST_TIMEOUT).await?;
        if !out.success {
            return Err(EnvError::CommandFailed {
                command: format!("docker inspect {id}"),
                stderr: out.stderr,
            });
        }
        parse_inspect(&out.stdout)
    }

    async fn stop_container(&self, id: &str) -> Result<(), EnvError> {
        let out = process::run("docker", &["stop", id], WRITE_TIMEOUT).await?;
        out.into_stdout("docker stop").map(|_| ())
    }

    async fn remove_container(&self, id: &str) -> Result<(), EnvError> {
        let out = process::run("docker", &["rm", "-f", id], WRITE_TIMEOUT).await?;
        out.into_stdout("docker rm").map(|_| ())
    }

    async fn images(&self) -> Result<Vec<String>, EnvError> {
        let out = process::run(
            "docker",
            &["images", "--format", "{{.Repository}}:{{.Tag}}"],
            LIST_TIMEOUT,
        )
        .await?;
        let stdout = out.into_stdout("docker images")?;
        Ok(stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.contains(":<none>"))
            .map(ToOwned::to_owned)
            .collect())
    }

    async fn disk_usage(&self) -> Result<String, EnvError> {
        let out = process::run("docker", &["system", "df"], LIST_TIMEOUT).await?;
        out.into_stdout("docker system df")
    }

    async fn pull_image(&self, reference: &str) -> Result<(), EnvError> {
        let out = process::run("docker", &["pull", reference], CREATE_TIMEOUT).await?;
        out.into_stdout("docker pull").map(|_| ())
    }

    async fn admin_token(&self, id: &str) -> Result<String, EnvError> {
        let out = process::run(
            "docker",
            &["exec", id, "cat", "/var/lib/mixum/admin-token"],
            LIST_TIMEOUT,
        )
        .await?;
        let token = out.into_stdout("docker exec admin-token")?;
        if token.is_empty() {
            return Err(EnvError::Parse("admin-token 为空".into()));
        }
        Ok(token)
    }

    async fn load_image(&self, path: &str) -> Result<(), EnvError> {
        let out = process::run("docker", &["load", "-i", path], CREATE_TIMEOUT).await?;
        out.into_stdout("docker load").map(|_| ())
    }

    async fn prune(&self) -> Result<String, EnvError> {
        let out = process::run("docker", &["system", "prune", "-f"], WRITE_TIMEOUT).await?;
        out.into_stdout("docker system prune")
    }

    async fn logs(&self, id: &str, tail: usize) -> Result<Vec<String>, EnvError> {
        let tail = tail.to_string();
        let out = process::run("docker", &["logs", "--tail", &tail, id], LIST_TIMEOUT).await?;
        if !out.success {
            return Err(EnvError::CommandFailed {
                command: format!("docker logs {id}"),
                stderr: out.stderr,
            });
        }
        let mut lines: Vec<String> = out
            .stdout
            .lines()
            .chain(out.stderr.lines())
            .map(str::to_owned)
            .collect();
        lines.truncate(tail.parse().unwrap_or(usize::MAX));
        Ok(lines)
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

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerInspect {
    name: String,
    image: String,
    config: DockerConfig,
    host_config: DockerHostConfig,
    #[serde(default)]
    mounts: Vec<DockerMount>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerConfig {
    image: String,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    cmd: Option<Vec<String>>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerHostConfig {
    #[serde(default)]
    port_bindings: std::collections::BTreeMap<String, Option<Vec<DockerPortBinding>>>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerPortBinding {
    #[serde(default)]
    host_port: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DockerMount {
    #[serde(default, rename = "Type")]
    mount_type: Option<String>,
    #[serde(default)]
    name: Option<String>,
    source: String,
    destination: String,
    #[serde(rename = "RW")]
    rw: bool,
}

fn parse_inspect(raw: &str) -> Result<InspectedContainer, EnvError> {
    let entries: Vec<DockerInspect> = serde_json::from_str(raw)?;
    let entry = entries
        .into_iter()
        .next()
        .ok_or_else(|| EnvError::Parse("docker inspect 返回空数组".into()))?;
    let mut ports = Vec::new();
    for (key, bindings) in &entry.host_config.port_bindings {
        let Some((container, protocol)) = key.split_once('/') else {
            continue;
        };
        let Ok(container) = container.parse::<u16>() else {
            continue;
        };
        let Some(binding) = bindings.as_ref().and_then(|list| list.first()) else {
            continue;
        };
        let Ok(host) = binding.host_port.parse::<u16>() else {
            continue;
        };
        ports.push(PortMapping {
            host,
            container,
            protocol: protocol.to_owned(),
        });
    }
    let spec = CreateSpec {
        name: entry.name.trim_start_matches('/').to_owned(),
        image: entry.config.image,
        ports,
        mounts: entry
            .mounts
            .iter()
            .map(|mount| {
                let kind = mount.mount_type.clone().unwrap_or_else(|| "bind".into());
                // Docker inspect reports a volume's host path in Source; the
                // volume name is Name. Passing the path to --mount type=volume
                // fails with invalid characters during rebase/clone.
                let source = if kind == "volume" {
                    mount.name.clone().unwrap_or_else(|| mount.source.clone())
                } else {
                    mount.source.clone()
                };
                Mount {
                    kind,
                    source,
                    target: mount.destination.clone(),
                    read_only: !mount.rw,
                }
            })
            .collect(),
        env: entry
            .config
            .env
            .iter()
            .filter_map(|line| {
                line.split_once('=')
                    .map(|(key, value)| (key.to_owned(), value.to_owned()))
            })
            .collect(),
        command: entry.config.cmd.unwrap_or_default(),
        cap_add: Vec::new(),
    };
    let image_digest = entry.image.starts_with("sha256:").then_some(entry.image);
    Ok(InspectedContainer { spec, image_digest })
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

    #[test]
    fn parses_inspect() {
        let raw = r#"[{"Name":"/demo","Image":"sha256:abc",
            "Config":{"Image":"alpine:3.20","Env":["A=1"],"Cmd":["sleep","1"]},
            "HostConfig":{"PortBindings":{"7601/tcp":[{"HostPort":"17601"}]}},
            "Mounts":[{"Type":"volume","Name":"demo-work","Source":"/var/lib/docker/volumes/demo-work/_data","Destination":"/work","RW":true},{"Source":"/tmp","Destination":"/mnt","RW":false}]}]"#;
        let inspected = parse_inspect(raw).unwrap();
        let spec = inspected.spec;
        assert_eq!(spec.name, "demo");
        assert_eq!(spec.image, "alpine:3.20");
        assert_eq!(spec.ports[0].container, 7601);
        assert_eq!(spec.ports[0].host, 17601);
        assert_eq!(spec.env.get("A").map(String::as_str), Some("1"));
        assert_eq!(spec.command, vec!["sleep", "1"]);
        assert_eq!(spec.mounts[0].source, "demo-work");
        assert_eq!(spec.mounts[0].target, "/work");
        assert!(!spec.mounts[0].read_only);
        assert!(spec.mounts[1].read_only);
        assert_eq!(inspected.image_digest.as_deref(), Some("sha256:abc"));
    }

    #[tokio::test]
    #[ignore = "需要本机 Docker 与 mixum-docker-test 容器"]
    async fn live_inspect() {
        let inspected = backend()
            .inspect_container("mixum-docker-test")
            .await
            .unwrap();
        println!("{inspected:?}");
        assert_eq!(inspected.spec.name, "mixum-docker-test");
        assert_eq!(inspected.spec.ports[0].container, 7601);
        assert_eq!(inspected.spec.mounts[0].target, "/mnt");
        assert_eq!(inspected.spec.env.get("A").map(String::as_str), Some("1"));
    }
}
