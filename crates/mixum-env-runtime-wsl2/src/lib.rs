//! WSL 2 后端（Windows）。
//!
//! mixumOS 节点在 WSL 2 宿主上以「发行版」形式存在：由
//! `wsl --import <name> <dir> <rootfs.tar>` 导入 mixumOS rootfs 生成。
//! 命名约定 `mixum-<name>`，本后端只把匹配约定（不区分大小写）的发行版
//! 当作 mixumOS 节点列出；其余发行版是宿主自己的环境，不在管理范围。
//!
//! 写操作的字段映射与幂等约定见 `docs/runtime-contract.md` §9：
//! create = import、start = 拉起实例、stop = terminate、remove = unregister。
//!
//! wsl.exe 的控制台输出是 UTF-16LE（不随系统区域与代码页变化），统一经
//! [`decode_console`] 解码；外部调用走 core 的 [`process::run_raw`]。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use mixum_env_core::container::{Container, ContainerState, CreateSpec};
use mixum_env_core::error::EnvError;
use mixum_env_core::process;
use mixum_env_core::runtime::{RuntimeBackend, RuntimeKind, RuntimeStatus};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
/// 导入 rootfs 要落盘出 vhdx，GB 级 tar 按分钟计（契约 §9）。
const CREATE_TIMEOUT: Duration = Duration::from_secs(600);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const WSL: &str = "wsl.exe";

/// mixumOS 节点发行版的命名前缀（不区分大小写）。
const DISTRO_PREFIX: &str = "mixum";

/// `create` 强制带连字符：列表过滤宽松到 `mixum`，创建严格到 `mixum-`，
/// 否则导入出来的节点会叫 `mixumfoo` 这种读不出层级感的名字。
const CREATE_PREFIX: &str = "mixum-";

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
        Ok(list_distributions()
            .await?
            .into_iter()
            .filter(Distro::is_mixum_node)
            .map(Distro::into_container)
            .collect())
    }

    async fn create_container(&self, spec: CreateSpec) -> Result<Container, EnvError> {
        if !cfg!(target_os = "windows") {
            return Err(unsupported());
        }
        validate_spec(&spec)?;
        if !Path::new(&spec.image).is_file() {
            return Err(invalid(
                "create_container",
                format!("rootfs tar 不存在：{}", spec.image),
            ));
        }
        if distro_exists(&spec.name).await? {
            return Err(invalid(
                "create_container",
                format!("发行版 {} 已存在，先 remove 再导入", spec.name),
            ));
        }

        let install_dir = install_root().join(&spec.name);
        std::fs::create_dir_all(&install_dir)?;
        let install_dir = install_dir.to_string_lossy().into_owned();
        let args = ["--import", &spec.name, &install_dir, &spec.image];
        run_wsl_checked(&args, CREATE_TIMEOUT).await?;

        // WSL 没有 created 状态：导入完成就是一个 Stopped 的发行版。
        Ok(imported_container(spec.name))
    }

    async fn start_container(&self, id: &str) -> Result<(), EnvError> {
        check_managed("start_container", id)?;
        let args = ["-d", id, "-e", "/bin/true"];
        let (success, stdout, stderr) = run_wsl(&args, WRITE_TIMEOUT).await?;
        if success {
            return Ok(());
        }
        // 与 stop/remove 不同：点一个不存在的节点是调用方的错，值得报出来。
        if !distro_exists(id).await? {
            return Err(invalid("start_container", format!("发行版 {id} 不存在")));
        }
        Err(failed(&args, &stdout, &stderr))
    }

    async fn stop_container(&self, id: &str) -> Result<(), EnvError> {
        check_managed("stop_container", id)?;
        let args = ["--terminate", id];
        let (success, stdout, stderr) = run_wsl(&args, WRITE_TIMEOUT).await?;
        if success {
            return Ok(());
        }
        // 已经不在列表里 = 已经停了，幂等（契约 §3、§9）。
        if !distro_exists(id).await? {
            return Ok(());
        }
        Err(failed(&args, &stdout, &stderr))
    }

    async fn remove_container(&self, id: &str) -> Result<(), EnvError> {
        check_managed("remove_container", id)?;
        let args = ["--unregister", id];
        let (success, stdout, stderr) = run_wsl(&args, WRITE_TIMEOUT).await?;
        if success {
            return Ok(());
        }
        // 不存在视为成功：重复点击删除不该报错（契约 §3、§9）。
        if !distro_exists(id).await? {
            return Ok(());
        }
        Err(failed(&args, &stdout, &stderr))
    }
}

fn unsupported() -> EnvError {
    EnvError::Unsupported {
        runtime: RuntimeKind::Wsl2,
        reason: "WSL 2 仅在 Windows 宿主可用".to_string(),
    }
}

/// 执行 wsl.exe 并把 UTF-16LE/UTF-8 输出统一解码成文本。
async fn run_wsl(args: &[&str], timeout: Duration) -> Result<(bool, String, String), EnvError> {
    let raw = process::run_raw(WSL, args, timeout).await?;
    let stdout = decode_console(&raw.stdout);
    let stderr = decode_console(&raw.stderr);
    Ok((raw.success, stdout, stderr))
}

/// 执行 wsl.exe 并要求成功；失败转成 [`EnvError::CommandFailed`]。
async fn run_wsl_checked(args: &[&str], timeout: Duration) -> Result<(), EnvError> {
    let (success, stdout, stderr) = run_wsl(args, timeout).await?;
    if success {
        return Ok(());
    }
    Err(failed(args, &stdout, &stderr))
}

/// 全部发行版（不过滤 `mixum-*`）。一个都没有时返回空表而不是错误——
/// 「没有容器」本来就是合法状态（契约 §3）。
async fn list_distributions() -> Result<Vec<Distro>, EnvError> {
    let args = ["--list", "--verbose"];
    let (success, stdout, stderr) = run_wsl(&args, LIST_TIMEOUT).await?;
    if !success {
        if no_distributions_installed(&stdout) || no_distributions_installed(&stderr) {
            return Ok(Vec::new());
        }
        return Err(failed(&args, &stdout, &stderr));
    }
    Ok(parse_distributions(&stdout))
}

async fn distro_exists(name: &str) -> Result<bool, EnvError> {
    Ok(list_distributions()
        .await?
        .iter()
        .any(|distro| distro.name.eq_ignore_ascii_case(name)))
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

/// 命令非零退出时的统一错误形态，stderr 取首行（契约 §5）。
fn failed(args: &[&str], stdout: &str, stderr: &str) -> EnvError {
    EnvError::CommandFailed {
        command: process::display_command(WSL, args),
        stderr: first_line(stderr)
            .or_else(|| first_line(stdout))
            .unwrap_or_default(),
    }
}

/// 参数不合法时的错误形态：契约里没有专门的校验错误类型，借用
/// `CommandFailed` 的展示链路，`command` 写操作名、`stderr` 写原因。
fn invalid(what: &str, reason: impl Into<String>) -> EnvError {
    EnvError::CommandFailed {
        command: what.to_string(),
        stderr: reason.into(),
    }
}

/// 写操作只允许碰 mixumOS 节点，避免把宿主自己的 Ubuntu / docker-desktop
/// terminate 或 unregister 掉（契约 §9）。
fn check_managed(what: &str, id: &str) -> Result<(), EnvError> {
    if is_create_name(id) {
        return Ok(());
    }
    Err(invalid(
        what,
        format!("{id} 不是 mixumOS 节点（需要 {CREATE_PREFIX} 前缀），拒绝操作"),
    ))
}

/// `CreateSpec` → `wsl --import` 的可行性检查，纯函数便于单测。
fn validate_spec(spec: &CreateSpec) -> Result<(), EnvError> {
    if !is_create_name(&spec.name) {
        return Err(invalid(
            "create_container",
            format!("发行版名必须以 {CREATE_PREFIX} 开头：{}", spec.name),
        ));
    }
    let unsupported: Vec<&str> = [
        (!spec.ports.is_empty()).then_some("ports"),
        (!spec.mounts.is_empty()).then_some("mounts"),
        (!spec.env.is_empty()).then_some("env"),
        (!spec.command.is_empty()).then_some("command"),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !unsupported.is_empty() {
        return Err(invalid(
            "create_container",
            format!(
                "WSL 2 导入不支持 {}（见 docs/runtime-contract.md §9）",
                unsupported.join(" / ")
            ),
        ));
    }
    Ok(())
}

fn is_create_name(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with(CREATE_PREFIX)
}

/// 发行版 vhdx 的落盘根目录：`%MIXUM_WSL_INSTALL_ROOT%`，缺省
/// `%LOCALAPPDATA%\mixum\wsl`（C 盘紧张时用环境变量指到别的盘）。
fn install_root() -> PathBuf {
    install_root_from(
        std::env::var("MIXUM_WSL_INSTALL_ROOT").ok(),
        std::env::var("LOCALAPPDATA").ok(),
    )
}

fn install_root_from(root: Option<String>, local_appdata: Option<String>) -> PathBuf {
    if let Some(root) = root.filter(|value| !value.trim().is_empty()) {
        return PathBuf::from(root);
    }
    let base = local_appdata.unwrap_or_else(|| r"C:\Users\Public\AppData\Local".to_string());
    PathBuf::from(base).join("mixum").join("wsl")
}

fn imported_container(name: String) -> Container {
    Container {
        id: name.clone(),
        name,
        // 与列表保持一致：WSL 发行版没有镜像概念，填运行时名占位（契约 §6）。
        image: "wsl2".to_string(),
        state: ContainerState::Stopped,
        status: Some("WSL 导入完成".to_string()),
        created_at: None,
        ports: vec![],
    }
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
    use mixum_env_core::container::PortMapping;

    fn utf16le(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    fn spec(name: &str) -> CreateSpec {
        CreateSpec {
            name: name.to_string(),
            image: r"C:\mixum\dist\rootfs.tar".to_string(),
            ports: vec![],
            mounts: vec![],
            env: Default::default(),
            command: vec![],
        }
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

    #[test]
    fn accepts_valid_create_spec() {
        assert!(validate_spec(&spec("mixum-dev")).is_ok());
        // 列表过滤不区分大小写，创建也放过大小写变体。
        assert!(validate_spec(&spec("Mixum-Dev")).is_ok());
    }

    #[test]
    fn rejects_names_without_mixum_prefix() {
        for name in ["ubuntu", "mixumdev", "mixum", "docker-desktop"] {
            let err = validate_spec(&spec(name)).unwrap_err().to_string();
            assert!(err.contains("mixum-"), "{name} 应被拒绝：{err}");
        }
    }

    #[test]
    fn rejects_fields_wsl_cannot_map() {
        let mut with_ports = spec("mixum-dev");
        with_ports.ports = vec![PortMapping::tcp(17601, 7601)];
        assert!(validate_spec(&with_ports)
            .unwrap_err()
            .to_string()
            .contains("ports"));

        let mut with_env = spec("mixum-dev");
        with_env.env.insert("MIXUM".to_string(), "1".to_string());
        assert!(validate_spec(&with_env)
            .unwrap_err()
            .to_string()
            .contains("env"));
    }

    #[test]
    fn write_ops_refuse_foreign_distros() {
        assert!(check_managed("start_container", "mixum-dev").is_ok());
        for id in ["Ubuntu-24.04", "docker-desktop"] {
            let err = check_managed("remove_container", id)
                .unwrap_err()
                .to_string();
            assert!(err.contains(id), "应点名拒绝 {id}：{err}");
        }
    }

    #[test]
    fn install_root_prefers_env_override() {
        let root = install_root_from(
            Some(r"D:\mixum-wsl".to_string()),
            Some(r"C:\Users\keri\AppData\Local".to_string()),
        );
        assert_eq!(root, PathBuf::from(r"D:\mixum-wsl"));

        let root = install_root_from(None, Some(r"C:\Users\keri\AppData\Local".to_string()));
        assert_eq!(
            root,
            PathBuf::from(r"C:\Users\keri\AppData\Local")
                .join("mixum")
                .join("wsl")
        );

        // 空白的覆盖值当作没设。
        let root = install_root_from(Some("   ".to_string()), None);
        assert!(root.ends_with(Path::new("mixum").join("wsl")));
    }

    #[test]
    fn create_reports_imported_distro_as_stopped() {
        let container = imported_container("mixum-dev".to_string());
        assert_eq!(container.id, "mixum-dev");
        assert_eq!(container.image, "wsl2");
        assert_eq!(container.state, ContainerState::Stopped);
        assert!(container.status.is_some());
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

    /// 真实幂等冒烟：只碰不存在的名字，不改动任何发行版。
    /// `cargo test -p mixum-env-runtime-wsl2 -- --ignored live_write_idempotency`
    #[tokio::test]
    #[ignore = "需要本机 WSL 与 Windows 宿主"]
    async fn live_write_idempotency() {
        let backend = backend();
        let absent = "mixum-live-absent";

        backend
            .remove_container(absent)
            .await
            .expect("remove 不存在的节点应幂等成功");
        backend
            .stop_container(absent)
            .await
            .expect("stop 不存在的节点应幂等成功");
        let err = backend
            .start_container(absent)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("不存在"), "start 不存在应报错：{err}");

        // 宿主自己的发行版不在管理范围，必须拒绝（契约 §9）。
        let err = backend
            .remove_container("Ubuntu-24.04")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Ubuntu-24.04"), "应点名拒绝：{err}");
    }

    /// 真实写入冒烟：会导入并注销一个 `mixum-live-test` 发行版。
    /// 需要设置 `MIXUM_TEST_ROOTFS` 指向一个可用的 rootfs tar，否则跳过。
    ///
    /// ```sh
    /// MIXUM_TEST_ROOTFS=D:\mixum\rootfs.tar \
    ///   cargo test -p mixum-env-runtime-wsl2 -- --ignored live_write_cycle
    /// ```
    #[tokio::test]
    #[ignore = "需要 Windows + WSL + MIXUM_TEST_ROOTFS"]
    async fn live_write_cycle() {
        let tar = match std::env::var("MIXUM_TEST_ROOTFS") {
            Ok(path) if Path::new(&path).is_file() => path,
            _ => {
                eprintln!("跳过：未设置 MIXUM_TEST_ROOTFS 或文件不存在");
                return;
            }
        };
        let backend = backend();
        let name = "mixum-live-test";

        // 清掉上次中断留下的残留；失败也无所谓（本来就可能不存在）。
        let _ = backend.remove_container(name).await;

        let mut spec = spec(name);
        spec.image = tar;
        let created = backend.create_container(spec).await.expect("import");
        assert_eq!(created.state, ContainerState::Stopped);

        backend.start_container(name).await.expect("start");
        backend.stop_container(name).await.expect("stop");
        backend.remove_container(name).await.expect("remove");

        let listed = backend.list_containers().await.unwrap();
        assert!(!listed.iter().any(|c| c.name == name), "删除后不应再列出");
    }
}
