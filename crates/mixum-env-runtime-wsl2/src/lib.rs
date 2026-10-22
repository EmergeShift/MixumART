//! WSL 2 后端（Windows）。
//!
//! mixumOS 节点在 WSL 2 宿主上以「发行版」形式存在：由
//! `wsl --import <name> <dir> <rootfs.tar>` 导入 mixumOS rootfs 生成。
//! 命名约定 `mixum-<name>`，本后端只把符合约定（不区分大小写）的发行版
//! 当作 mixumOS 节点；其余发行版是宿主自己的环境，不在管理范围。
//!
//! 「符合约定」只有一份实现（[`validate_name`]）：列表过滤、写操作保护、
//! 安装目录拼接共用它，避免出现「列得出来但点不动」的名字。
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
const NODE_PREFIX: &str = "mixum-";

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
        ensure_windows()?;
        Ok(list_distributions()
            .await?
            .into_iter()
            .filter(|distro| is_node_name(&distro.name))
            .map(Distro::into_container)
            .collect())
    }

    async fn create_container(&self, spec: CreateSpec) -> Result<Container, EnvError> {
        ensure_windows()?;
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

        let install_dir =
            install_dir(&spec.name).map_err(|reason| invalid("create_container", reason))?;
        std::fs::create_dir_all(&install_dir)?;
        let install_dir_arg = install_dir.to_string_lossy().into_owned();
        let args = ["--import", &spec.name, &install_dir_arg, &spec.image];
        if let Err(err) = run_wsl_checked(&args, CREATE_TIMEOUT).await {
            // 导入失败会留下半成品目录，不清掉的话同名重试会被非空目录挡住。
            let _ = std::fs::remove_dir_all(&install_dir);
            return Err(err);
        }

        // WSL 没有 created 状态：导入完成就是一个 Stopped 的发行版。
        Ok(imported_container(spec.name))
    }

    /// 只把发行版实例拉起来（`/bin/true` 跑完就退）。
    ///
    /// 这是**临时拉起**：WSL 在无进程约 15s 后自动回到 `Stopped`，
    /// `Running` 只表示「当前有会话」。常驻服务（`mixumd` / entrypoint）
    /// 的启动语义等后续契约（§9）。
    async fn start_container(&self, id: &str) -> Result<(), EnvError> {
        ensure_windows()?;
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
        ensure_windows()?;
        check_managed("stop_container", id)?;
        // 契约 §3：已停止视为成功。先看状态，别把「本来就是停的」报成失败。
        match find_distro(id).await? {
            None => return Ok(()),
            Some(distro) if distro.is_stopped() => return Ok(()),
            Some(_) => {}
        }

        let args = ["--terminate", id];
        let (success, stdout, stderr) = run_wsl(&args, WRITE_TIMEOUT).await?;
        if success {
            return Ok(());
        }
        // 竞态兜底：terminate 期间它可能已经自己停了。
        match find_distro(id).await? {
            None => Ok(()),
            Some(distro) if distro.is_stopped() => Ok(()),
            Some(_) => Err(failed(&args, &stdout, &stderr)),
        }
    }

    async fn admin_token(&self, id: &str) -> Result<String, EnvError> {
        ensure_windows()?;
        check_managed("admin_token", id)?;
        let args = ["-d", id, "-e", "cat", "/var/lib/mixum/admin-token"];
        let (success, stdout, stderr) = run_wsl(&args, LIST_TIMEOUT).await?;
        if !success {
            return Err(failed(&args, &stdout, &stderr));
        }
        let token = stdout.trim().to_owned();
        if token.is_empty() {
            return Err(invalid("admin_token", "admin-token 为空"));
        }
        Ok(token)
    }

    async fn logs(&self, id: &str, tail: usize) -> Result<Vec<String>, EnvError> {
        ensure_windows()?;
        check_managed("logs", id)?;
        let tail = tail.clamp(1, 2000);
        let script = format!(
            "journalctl -u mixumd -n {tail} --no-pager 2>/dev/null || \
             tail -n {tail} /var/log/mixumd.log 2>/dev/null || \
             dmesg 2>/dev/null | tail -n {tail}"
        );
        let args = ["-d", id, "-e", "sh", "-lc", &script];
        let (success, stdout, stderr) = run_wsl(&args, LIST_TIMEOUT).await?;
        if !success {
            return Err(failed(&args, &stdout, &stderr));
        }
        Ok(stdout
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    async fn remove_container(&self, id: &str) -> Result<(), EnvError> {
        ensure_windows()?;
        check_managed("remove_container", id)?;
        let existed = distro_exists(id).await?;
        let args = ["--unregister", id];
        let (success, stdout, stderr) = run_wsl(&args, WRITE_TIMEOUT).await?;
        if !success && existed {
            return Err(failed(&args, &stdout, &stderr));
        }
        // 发行版已经没了，顺带清掉安装目录（不是本后端建的目录就不存在，忽略即可）。
        if let Ok(dir) = install_dir(id) {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(())
    }

    /// WSL 2 没有镜像仓库：节点来自本地 rootfs tar，导入之后就不存在「镜像」这一层。
    /// 返回空列表而不是报错——否则设置页会把一个正常的后端显示成异常。
    async fn images(&self) -> Result<Vec<String>, EnvError> {
        ensure_windows()?;
        Ok(Vec::new())
    }

    /// 磁盘占用 = 各节点安装目录内文件大小之和。
    ///
    /// WSL 没有 `system df` 这类命令，vhdx 是唯一能反映真实占用的东西。
    async fn disk_usage(&self) -> Result<String, EnvError> {
        ensure_windows()?;
        let distros = list_distributions().await?;
        let mut nodes = 0usize;
        let mut total = 0u64;
        for distro in distros.iter().filter(|distro| is_node_name(&distro.name)) {
            nodes += 1;
            if let Ok(dir) = install_dir(&distro.name) {
                total += directory_size(&dir);
            }
        }
        Ok(format!(
            "{nodes} 个节点 · {}\n{}",
            human_bytes(total),
            install_root().display()
        ))
    }
}

/// 目录内文件大小之和。WSL 导入后安装目录通常只有 `ext4.vhdx`，不递归。
fn directory_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum()
}

/// 人类可读的字节数，够设置页展示即可。
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn unsupported() -> EnvError {
    EnvError::Unsupported {
        runtime: RuntimeKind::Wsl2,
        reason: "WSL 2 仅在 Windows 宿主可用".to_string(),
    }
}

/// 非 Windows 宿主上一律提前返回 `Unsupported`，不要落到「找不到 wsl.exe」。
fn ensure_windows() -> Result<(), EnvError> {
    if cfg!(target_os = "windows") {
        Ok(())
    } else {
        Err(unsupported())
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

async fn find_distro(name: &str) -> Result<Option<Distro>, EnvError> {
    Ok(list_distributions()
        .await?
        .into_iter()
        .find(|distro| distro.name.eq_ignore_ascii_case(name)))
}

async fn distro_exists(name: &str) -> Result<bool, EnvError> {
    Ok(find_distro(name).await?.is_some())
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

/// mixumOS 节点发行版名的唯一校验入口，列表过滤与写操作共用。
///
/// 除了前缀，后缀还必须是 `[A-Za-z0-9._-]`：这个名字会直接拼进安装目录
/// （`install_root()/<name>`），放开分隔符或 `..` 就能写出安装根目录。
fn validate_name(name: &str) -> Result<(), String> {
    let lower = name.to_ascii_lowercase();
    let Some(suffix) = lower.strip_prefix(NODE_PREFIX) else {
        return Err(format!("发行版名必须以 {NODE_PREFIX} 开头：{name}"));
    };
    if suffix.is_empty() {
        return Err(format!("{NODE_PREFIX} 之后不能为空：{name}"));
    }
    if suffix.contains("..") {
        return Err(format!("发行版名不能包含 `..`：{name}"));
    }
    if suffix.ends_with('.') {
        return Err(format!("发行版名不能以 `.` 结尾：{name}"));
    }
    if let Some(bad) = suffix
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    {
        return Err(format!(
            "发行版名只能包含字母、数字、`.`、`_`、`-`（非法字符 {bad:?}）：{name}"
        ));
    }
    Ok(())
}

fn is_node_name(name: &str) -> bool {
    validate_name(name).is_ok()
}

/// 写操作只允许碰 mixumOS 节点，避免把宿主自己的 Ubuntu / docker-desktop
/// terminate 或 unregister 掉（契约 §9）。
fn check_managed(what: &str, id: &str) -> Result<(), EnvError> {
    validate_name(id).map_err(|reason| invalid(what, format!("{reason}，拒绝操作")))
}

/// `CreateSpec` → `wsl --import` 的可行性检查，纯函数便于单测。
fn validate_spec(spec: &CreateSpec) -> Result<(), EnvError> {
    validate_name(&spec.name).map_err(|reason| invalid("create_container", reason))?;
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

/// 单个发行版的安装目录。名字已过白名单（不含分隔符与 `..`），这里再确认
/// 一次拼接结果没有跳出根目录，作为纵深防御。
fn install_dir(name: &str) -> Result<PathBuf, String> {
    let root = install_root();
    let dir = root.join(name);
    if !dir.starts_with(&root) {
        return Err(format!("安装路径越界：{}", dir.display()));
    }
    Ok(dir)
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
        // 不用 as_chunks::<2>()（要求 Rust 1.88+），也不用 chunks_exact(2)
        // （clippy 会要求换成 as_chunks）：本仓库 MSRV 是 1.77。
        let mut units = Vec::with_capacity(body.len() / 2);
        for pair in body.chunks(2) {
            if let [lo, hi] = pair {
                units.push(u16::from_le_bytes([*lo, *hi]));
            }
        }
        String::from_utf16_lossy(&units).trim().to_string()
    } else {
        String::from_utf8_lossy(body).trim().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Distro {
    name: String,
    state_raw: String,
    version: String,
}

impl Distro {
    fn is_stopped(&self) -> bool {
        self.state_raw.eq_ignore_ascii_case("stopped")
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

/// 解析 `wsl --list --verbose` 表格。
///
/// 以「最后一列是版本号」锚定，不写死 STATE 枚举：WSL 新增状态（例如
/// `Uninstalling`）时这一行仍然能列出来，展示层把未知状态降级成 `Unknown`，
/// 不会整行消失。默认发行版的 `*` 标记剥掉，名字里的空格保留。
fn parse_distributions(text: &str) -> Vec<Distro> {
    let mut distros = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 3 {
            continue;
        }
        let version = tokens[tokens.len() - 1];
        if !version.chars().all(|c| c.is_ascii_digit()) {
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
            state_raw: tokens[tokens.len() - 2].to_string(),
            version: version.to_string(),
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

    /// 需要真实导入的 live 用例靠它拿到 rootfs；没配就返回 `None`。
    fn test_rootfs() -> Option<String> {
        std::env::var("MIXUM_TEST_ROOTFS")
            .ok()
            .filter(|path| Path::new(path).is_file())
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
    fn keeps_rows_with_states_we_do_not_know() {
        // 新增状态不应让整行消失（旧实现会按 KNOWN_STATES 白名单跳过）。
        let text = "  NAME  STATE  VERSION\nmixum-dev  Uninstalling  2\n";
        let distros = parse_distributions(text);
        assert_eq!(distros.len(), 1);
        assert_eq!(distros[0].state_raw, "Uninstalling");
        assert_eq!(
            distros[0].clone().into_container().state,
            ContainerState::Unknown
        );
    }

    #[test]
    fn skips_header_and_unknown_lines() {
        assert!(parse_distributions("  NAME  STATE  VERSION\n").is_empty());
        assert!(parse_distributions("").is_empty());
        assert!(parse_distributions("garbage line without states\n").is_empty());
        // 最后一列不是版本号 → 不是发行版行。
        assert!(parse_distributions("mixum-dev  Running  n/a\n").is_empty());
    }

    #[test]
    fn filters_mixum_nodes_case_insensitively() {
        assert!(is_node_name("mixum-dev"));
        assert!(is_node_name("Mixum-Node"));
        assert!(!is_node_name("Ubuntu-24.04"));
        assert!(!is_node_name("docker-desktop"));
        // 前缀必须带连字符：列表与写操作用同一套规则，不能这里列出来、
        // 那边点不动。
        assert!(!is_node_name("mixum"));
        assert!(!is_node_name("mixumfoo"));
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
    fn recognizes_stopped_state() {
        let distro = |state: &str| Distro {
            name: "mixum-dev".to_string(),
            state_raw: state.to_string(),
            version: "2".to_string(),
        };
        assert!(distro("Stopped").is_stopped());
        assert!(distro("stopped").is_stopped());
        assert!(!distro("Running").is_stopped());
        assert!(!distro("Uninstalling").is_stopped());
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
        // 大小写不敏感，与列表过滤一致。
        assert!(validate_spec(&spec("Mixum-Dev")).is_ok());
        assert!(validate_spec(&spec("mixum-a_b.c-1")).is_ok());
    }

    #[test]
    fn rejects_names_without_mixum_prefix() {
        // mixum / mixumfoo 少了连字符，列表与写操作都必须拒绝。
        for name in ["ubuntu", "mixumdev", "mixum", "mixumfoo", "docker-desktop"] {
            let err = validate_spec(&spec(name)).unwrap_err().to_string();
            assert!(err.contains("mixum-"), "{name} 应被拒绝：{err}");
        }
    }

    #[test]
    fn rejects_path_like_names() {
        // 名字会拼进安装目录，分隔符与 .. 一律拒绝。
        for name in [
            r"mixum-..\evil",
            "mixum-../evil",
            "mixum-a/b",
            r"mixum-a\b",
            "mixum-..",
            "mixum-.",
            "mixum-a b",
            "mixum-",
            "mixum-节点",
        ] {
            assert!(validate_spec(&spec(name)).is_err(), "{name:?} 应被拒绝");
            assert!(!is_node_name(name), "{name:?} 不应算节点");
        }
    }

    #[test]
    fn install_dir_stays_under_root() {
        let dir = install_dir("mixum-dev").expect("合法名字应能拼出目录");
        assert!(dir.starts_with(install_root()));
        assert_eq!(dir.file_name().and_then(|n| n.to_str()), Some("mixum-dev"));
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
        for id in ["Ubuntu-24.04", "docker-desktop", "mixum", "mixumfoo"] {
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

    #[test]
    fn formats_human_bytes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KiB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3.0 MiB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn sums_install_directory_bytes() {
        let dir = std::env::temp_dir().join(format!("mixum-wsl-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ext4.vhdx"), vec![0u8; 4096]).unwrap();
        assert_eq!(directory_size(&dir), 4096);
        // 目录不存在时返回 0，不 panic。
        assert_eq!(directory_size(&dir.join("missing")), 0);
        let _ = std::fs::remove_dir_all(&dir);
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

        // 资源类方法必须给出可用结果，否则设置页会把 WSL 2 显示成异常。
        assert!(
            backend.images().await.unwrap().is_empty(),
            "WSL 2 没有镜像概念"
        );
        let disk = backend.disk_usage().await.unwrap();
        assert!(disk.contains("个节点"), "磁盘摘要：{disk}");
        println!("磁盘占用：{disk}");
    }

    /// 真实幂等冒烟：
    /// - 不存在的名字：`remove` / `stop` 成功，`start` 报「不存在」；
    /// - **存在但已 Stopped** 的名字：`stop` 直接成功（契约 §3）——这一段需要
    ///   真实发行版，故需 `MIXUM_TEST_ROOTFS`，未设置时跳过；
    /// - 宿主自己的发行版：拒绝操作。
    ///
    /// `cargo test -p mixum-env-runtime-wsl2 -- --ignored live_write_idempotency`
    #[tokio::test]
    #[ignore = "需要本机 WSL 与 Windows 宿主（可选 MIXUM_TEST_ROOTFS）"]
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

        // 已存在但已经 Stopped：stop 必须成功，而不是把 terminate 的非零退出
        // 当成失败（单测只覆盖了 is_stopped 纯函数，这里走真实 wsl.exe）。
        // live_write_cycle 也覆盖 import→stop，但这条额外走 list_containers()
        // 解析出的 Stopped 状态，避免只依赖 create_container 的返回值。
        if let Some(tar) = test_rootfs() {
            let name = "mixum-live-idem";
            let backend_cleanup = backend.clone();
            let backend_task = backend.clone();
            let result = tokio::spawn(async move {
                let _ = backend_task.remove_container(name).await;

                let mut spec = spec(name);
                spec.image = tar;
                backend_task.create_container(spec).await.expect("import");
                let listed = backend_task.list_containers().await.unwrap();
                let node = listed
                    .iter()
                    .find(|container| container.name == name)
                    .expect("导入后应能列出");
                assert_eq!(
                    node.state,
                    ContainerState::Stopped,
                    "新导入的节点应当是 Stopped"
                );

                backend_task
                    .stop_container(name)
                    .await
                    .expect("stop 一个已存在且已 Stopped 的节点应成功");
                backend_task
                    .stop_container(name)
                    .await
                    .expect("重复 stop 已 Stopped 的节点也应成功");
                backend_task.remove_container(name).await.expect("remove");
            })
            .await;
            // 断言/中间步骤 panic 时也清掉 distro，避免污染下一次 live 运行。
            let _ = backend_cleanup.remove_container(name).await;
            if let Err(error) = result {
                panic!("live_write_idempotency 子任务失败: {error}");
            }
        } else {
            eprintln!("跳过「存在但已 Stopped」用例：未设置 MIXUM_TEST_ROOTFS");
        }

        // 宿主自己的发行版不在管理范围，必须拒绝（契约 §9）。
        let err = backend
            .remove_container("Ubuntu-24.04")
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Ubuntu-24.04"), "应点名拒绝：{err}");
    }

    /// 真实写入冒烟：会导入并注销一个 `mixum-live-test` 发行版。
    /// 需要设置 `MIXUM_TEST_ROOTFS` 指向一个可用的 rootfs tar（`.tar` / `.tar.gz`），
    /// 否则跳过。
    #[tokio::test]
    #[ignore = "需要 Windows + WSL + MIXUM_TEST_ROOTFS"]
    async fn live_write_cycle() {
        let Some(tar) = test_rootfs() else {
            eprintln!("跳过：未设置 MIXUM_TEST_ROOTFS 或文件不存在");
            return;
        };
        let backend = backend();
        let name = "mixum-live-test";

        // 清掉上次中断留下的残留；失败也无所谓（本来就可能不存在）。
        let _ = backend.remove_container(name).await;

        let mut spec = spec(name);
        spec.image = tar;
        let created = backend.create_container(spec).await.expect("import");
        assert_eq!(created.state, ContainerState::Stopped);

        // 刚导入就是 Stopped：契约 §3 要求「已停止视为成功」。
        backend
            .stop_container(name)
            .await
            .expect("stop 已停止的节点应幂等成功");

        backend.start_container(name).await.expect("start");
        backend.stop_container(name).await.expect("stop");
        backend.remove_container(name).await.expect("remove");

        let listed = backend.list_containers().await.unwrap();
        assert!(!listed.iter().any(|c| c.name == name), "删除后不应再列出");
    }

    /// 真实失败路径：导入一个不是 tar 的文件，确认报错且安装目录被清掉。
    #[tokio::test]
    #[ignore = "需要 Windows + WSL"]
    async fn live_import_failure_cleans_install_dir() {
        let backend = backend();
        let name = "mixum-live-bad";
        let _ = backend.remove_container(name).await;

        let bogus = std::env::temp_dir().join("mixum-bogus-rootfs.tar");
        std::fs::write(&bogus, b"definitely not a tar").unwrap();
        let dir = install_dir(name).unwrap();
        let _ = std::fs::remove_dir_all(&dir);

        let mut spec = spec(name);
        spec.image = bogus.to_string_lossy().into_owned();
        let err = backend
            .create_container(spec)
            .await
            .expect_err("非 tar 文件应导入失败");
        assert!(matches!(err, EnvError::CommandFailed { .. }), "{err:?}");
        assert!(!dir.exists(), "导入失败后应清掉安装目录：{}", dir.display());
        assert!(!distro_exists(name).await.unwrap());

        let _ = std::fs::remove_file(bogus);
    }
}
