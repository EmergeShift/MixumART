//! WSL 2 后端（Windows）。
//!
//! mixumOS 节点在 WSL 2 宿主上以「发行版」形式存在：由
//! `wsl --import <name> <dir> <rootfs.tar>` 导入 mixumOS rootfs 生成。
//! 命名约定 `mixum-<name>`，本后端只把匹配约定（不区分大小写）的发行版
//! 当作 mixumOS 节点列出；其余发行版是宿主自己的环境，不在管理范围。
//!
//! wsl.exe 的控制台输出是 UTF-16LE（不随系统区域与代码页变化），统一经
//! [`decode_console`] 解码；外部调用走 core 的 [`process::run_raw`]。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mixum_env_core::container::{Container, ContainerState};
use mixum_env_core::error::EnvError;
use mixum_env_core::process;
use mixum_env_core::runtime::{RuntimeBackend, RuntimeKind, RuntimeStatus};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const WSL: &str = "wsl.exe";

/// mixumOS 节点发行版的命名前缀（不区分大小写）。
const DISTRO_PREFIX: &str = "mixum";

/// `wsl --list --verbose` 的 STATE 列枚举，不随界面语言变化。
const KNOWN_STATES: [&str; 4] = ["Running", "Stopped", "Installing", "Converting"];

/// 应用装配入口。
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
        let (success, stdout, stderr) = match run_wsl(&["--status"], PROBE_TIMEOUT).await {
            Ok(parts) => parts,
            Err(EnvError::NotInstalled { .. }) => {
                return RuntimeStatus::NotInstalled {
                    hint: "以管理员运行 `wsl --install` 启用 WSL".to_string(),
                }
            }
            Err(err) => {
                return RuntimeStatus::NotRunning {
                    hint: err.to_string(),
                }
            }
        };
        if no_distributions_installed(&stdout) || no_distributions_installed(&stderr) {
            // 功能本身可用，只是还没有任何发行版——可以导入 mixumOS 节点。
            return RuntimeStatus::Ready {
                version: best_effort_version().await,
                detail: Some("WSL 可用，尚未安装任何发行版".to_string()),
            };
        }
        if !success {
            return RuntimeStatus::NotRunning {
                hint: first_line(&stdout)
                    .or_else(|| first_line(&stderr))
                    .unwrap_or_else(|| "WSL 功能未启用".to_string()),
            };
        }
        RuntimeStatus::Ready {
            version: best_effort_version().await,
            detail: None,
        }
    }

    async fn list_containers(&self) -> Result<Vec<Container>, EnvError> {
        let (success, stdout, stderr) = run_wsl(&["--list", "--verbose"], LIST_TIMEOUT).await?;
        if !success {
            return Err(EnvError::CommandFailed {
                command: "wsl.exe --list --verbose".to_string(),
                stderr: first_line(&stderr)
                    .unwrap_or_else(|| first_line(&stdout).unwrap_or_default()),
            });
        }
        Ok(parse_distributions(&stdout)
            .into_iter()
            .filter(|distro| distro.is_mixum_node())
            .map(|distro| distro.into_container())
            .collect())
    }
}

/// 执行 wsl.exe 并把 UTF-16LE/UTF-8 输出统一解码成文本。
async fn run_wsl(args: &[&str], timeout: Duration) -> Result<(bool, String, String), EnvError> {
    let raw = process::run_raw(WSL, args, timeout).await?;
    let stdout = decode_console(&raw.stdout);
    let stderr = decode_console(&raw.stderr);
    Ok((raw.success, stdout, stderr))
}

/// `wsl --version` 只在新版 WSL 存在；失败不影响 probe 结果。
async fn best_effort_version() -> Option<String> {
    let (success, stdout, _) = run_wsl(&["--version"], PROBE_TIMEOUT).await.ok()?;
    if !success {
        return None;
    }
    let value = first_line(&stdout)?.split_once(':')?.1.trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// wsl.exe 装了但一个发行版都没有时的提示文案（中英文两种已知形态）。
fn no_distributions_installed(text: &str) -> bool {
    text.contains("没有已安装的分发")
        || text
            .to_ascii_lowercase()
            .contains("no installed distributions")
}

fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

/// wsl.exe 的输出编码不随系统代码页变化：默认 UTF-16LE（ASCII 部分呈
/// NUL 交错，行尾 \r\x00\n\x00 必然产生 0x00），新版支持 WSL_UTF8=1
/// 但旧版忽略。按字节启发式：BOM 或含 0x00 → UTF-16LE，否则按 UTF-8。
fn decode_console(raw: &[u8]) -> String {
    let body = raw.strip_prefix([0xFF, 0xFE].as_slice()).unwrap_or(raw);
    if body.contains(&0u8) {
        let units: Vec<u16> = body
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units).trim().to_string()
    } else {
        String::from_utf8_lossy(body).trim().to_string()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Distro {
    name: String,
    state_raw: String,
    version: String,
}

impl Distro {
    fn is_mixum_node(&self) -> bool {
        self.name.to_ascii_lowercase().starts_with(DISTRO_PREFIX)
    }

    fn into_container(self) -> Container {
        Container {
            id: self.name.clone(),
            name: self.name,
            // WSL 发行版没有镜像概念，填运行时名占位（语义见 runtime-contract.md §6）。
            image: "wsl2".to_string(),
            state: ContainerState::from_raw(&self.state_raw),
            status: Some(format!("WSL{} {}", self.version, self.state_raw)),
            created_at: None,
            ports: vec![],
        }
    }
}

/// 解析 `wsl --list --verbose` 表格。STATE 列是不随区域变化的英文枚举，
/// 以它为锚点取行，避免依赖列宽与表头；默认发行版的 `*` 标记剥掉。
fn parse_distributions(text: &str) -> Vec<Distro> {
    let mut distros = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 3 {
            continue;
        }
        let state_raw = tokens[tokens.len() - 2];
        if !KNOWN_STATES.contains(&state_raw) {
            continue;
        }
        let mut name_tokens = &tokens[..tokens.len() - 2];
        if name_tokens.first() == Some(&"*") {
            name_tokens = &name_tokens[1..];
        }
        if name_tokens.is_empty() {
            continue;
        }
        distros.push(Distro {
            name: name_tokens.join(" "),
            state_raw: state_raw.to_string(),
            version: tokens[tokens.len() - 1].to_string(),
        });
    }
    distros
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    #[test]
    fn decodes_utf16le_console_output() {
        let raw = utf16le(
            "  NAME            STATE           VERSION\r\n* Ubuntu-24.04    Stopped         2\r\n",
        );
        assert_eq!(
            decode_console(&raw),
            "NAME            STATE           VERSION\r\n* Ubuntu-24.04    Stopped         2"
        );
    }

    #[test]
    fn decodes_utf16le_cjk_without_nul_interference() {
        let raw = utf16le("默认分发: Ubuntu-24.04\r\n默认版本: 2");
        assert!(decode_console(&raw).starts_with("默认分发: Ubuntu-24.04"));
    }

    #[test]
    fn decodes_utf8_passthrough() {
        assert_eq!(
            decode_console(b"WSL version: 3.0.3.0\r\n"),
            "WSL version: 3.0.3.0"
        );
    }

    #[test]
    fn strips_utf16le_bom() {
        let mut raw = vec![0xFF, 0xFE];
        raw.extend(utf16le("hello"));
        assert_eq!(decode_console(&raw), "hello");
    }

    #[test]
    fn parses_real_list_output() {
        let text =
            "  NAME            STATE           VERSION\n* Ubuntu-24.04    Stopped         2\n";
        let distros = parse_distributions(text);
        assert_eq!(
            distros,
            vec![Distro {
                name: "Ubuntu-24.04".to_string(),
                state_raw: "Stopped".to_string(),
                version: "2".to_string(),
            }]
        );
    }

    #[test]
    fn parses_names_with_spaces_and_star() {
        let text =
            "  NAME      STATE     VERSION\n* My Distro  Running   2\nmixum-build  Running   2\n";
        let distros = parse_distributions(text);
        assert_eq!(distros.len(), 2);
        assert_eq!(distros[0].name, "My Distro");
        assert_eq!(distros[1].name, "mixum-build");
        assert_eq!(distros[1].state_raw, "Running");
    }

    #[test]
    fn skips_header_and_unknown_lines() {
        assert!(parse_distributions("  NAME  STATE  VERSION\n").is_empty());
        assert!(parse_distributions("").is_empty());
        assert!(parse_distributions("garbage line without states\n").is_empty());
    }

    #[test]
    fn filters_mixum_nodes_case_insensitively() {
        let node = |name: &str| Distro {
            name: name.to_string(),
            state_raw: "Running".to_string(),
            version: "2".to_string(),
        };
        assert!(node("mixum-dev").is_mixum_node());
        assert!(node("Mixum-Node").is_mixum_node());
        assert!(!node("Ubuntu-24.04").is_mixum_node());
        assert!(!node("docker-desktop").is_mixum_node());
    }

    #[test]
    fn maps_distro_to_container() {
        let container = Distro {
            name: "mixum-dev".to_string(),
            state_raw: "Stopped".to_string(),
            version: "2".to_string(),
        }
        .into_container();
        assert_eq!(container.id, "mixum-dev");
        assert_eq!(container.name, "mixum-dev");
        assert_eq!(container.state, ContainerState::Stopped);
        assert_eq!(container.status.as_deref(), Some("WSL2 Stopped"));
        assert_eq!(container.ports, vec![]);
        assert_eq!(container.created_at, None);
    }

    #[test]
    fn detects_no_distributions_hint() {
        assert!(no_distributions_installed(
            "适用于 Linux 的 Windows 子系统没有已安装的分发版。"
        ));
        assert!(no_distributions_installed(
            "There are no installed distributions."
        ));
        assert!(!no_distributions_installed("默认分发: Ubuntu-24.04"));
    }

    /// 真实后端冒烟：需要 Windows 宿主与已启用的 WSL。
    /// `cargo test -p mixum-env-runtime-wsl2 -- --ignored`
    #[tokio::test]
    #[ignore = "需要本机 WSL 与 Windows 宿主"]
    async fn live_probe_and_list() {
        let backend = backend();
        let status = backend.probe().await;
        assert!(status.is_ready(), "wsl 不可用：{status:?}");
        let containers = backend.list_containers().await.unwrap();
        println!("mixumOS 节点（mixum-* 发行版）数：{}", containers.len());
    }
}
