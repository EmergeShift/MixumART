//! Shared setup planner for the mixum CLI and MixumART desktop.
//!
//! The setup flow is deliberately a plan/apply API: the CLI and the desktop
//! both render the same plan, and only `apply` writes host configuration.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use mixum_env_core::{BackendRegistry, ContainerState, RuntimeKind, RuntimeStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use toml_edit::{value, Array, DocumentMut, Item, Table};

pub mod update;
pub use update::UpdateInfo;

pub const DEFAULT_IMAGE: &str = "ghcr.io/emergeshift/mixumos:0.1.6";
const MCP_SERVER_KEY: &str = "mixum";

#[derive(Debug, Error)]
pub enum SetupError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("toml: {0}")]
    Toml(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("runtime: {0}")]
    Runtime(String),
}

pub type Result<T> = std::result::Result<T, SetupError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "zh-CN")]
    ZhCn,
    #[serde(rename = "en")]
    En,
}

impl Default for Language {
    fn default() -> Self {
        detect_language()
    }
}

impl Language {
    pub const fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::ZhCn => "中文",
            Self::En => "English",
        }
    }
}

pub fn detect_language() -> Language {
    let value = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if value.starts_with("zh") {
        Language::ZhCn
    } else {
        Language::En
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SetupSettings {
    #[serde(default)]
    pub language: Option<Language>,
    #[serde(default)]
    pub runtime: Option<RuntimeKind>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub environment_id: Option<String>,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub updated_at_ms: u128,
}

impl SetupSettings {
    pub fn language_or_default(&self) -> Language {
        self.language.unwrap_or_default()
    }
}

pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("MIXUM_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("dev.mixum.agentenv")
        .join("mixum-cli")
}

pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("setup.json")
}

pub fn load_settings(data_dir: &Path) -> Result<SetupSettings> {
    match fs::read_to_string(settings_path(data_dir)) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SetupSettings::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn save_settings(data_dir: &Path, settings: &SetupSettings) -> Result<()> {
    fs::create_dir_all(data_dir)?;
    let text = serde_json::to_string_pretty(settings)?;
    write_private_atomic(&settings_path(data_dir), &text)
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn write_private_atomic(path: &Path, text: &str) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(SetupError::Invalid(format!(
            "path has no parent: {}",
            path.display()
        )));
    };
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}-{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("mixum"),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallAction {
    pub command: Vec<String>,
    pub requires_admin: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnerView {
    pub kind: RuntimeKind,
    pub name: String,
    pub supported: bool,
    pub status: RuntimeStatus,
    pub install: Option<InstallAction>,
    pub start: Option<InstallAction>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageChoice {
    pub runtime: RuntimeKind,
    pub reference: String,
    pub local: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentChoice {
    pub runtime: RuntimeKind,
    pub id: String,
    pub name: String,
    pub image: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientView {
    pub id: String,
    pub name: String,
    pub path: String,
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupPlan {
    pub settings: SetupSettings,
    pub runners: Vec<RunnerView>,
    pub images: Vec<ImageChoice>,
    pub environments: Vec<EnvironmentChoice>,
    pub clients: Vec<ClientView>,
    pub launcher: String,
    pub default_image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    pub language: Language,
    pub runtime: RuntimeKind,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub pull_image: bool,
    #[serde(default)]
    pub environment_id: Option<String>,
    #[serde(default)]
    pub clients: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientFailure {
    pub id: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupResult {
    pub settings: SetupSettings,
    pub configured: Vec<String>,
    pub failed: Vec<ClientFailure>,
    pub runner: RuntimeKind,
    pub image: Option<String>,
}

pub fn platform_supports(kind: RuntimeKind) -> bool {
    match kind {
        RuntimeKind::Docker => true,
        RuntimeKind::Wsl2 => cfg!(target_os = "windows"),
        RuntimeKind::AppleContainer => cfg!(target_os = "macos"),
    }
}

pub fn install_action(kind: RuntimeKind) -> Option<InstallAction> {
    if !platform_supports(kind) {
        return None;
    }
    match kind {
        RuntimeKind::Docker => {
            if cfg!(target_os = "macos") {
                Some(InstallAction {
                    command: vec![
                        "brew".into(),
                        "install".into(),
                        "--cask".into(),
                        "docker".into(),
                    ],
                    requires_admin: false,
                    note: "安装后打开 Docker Desktop，等待 daemon 启动。".into(),
                })
            } else if cfg!(target_os = "windows") {
                Some(InstallAction {
                    command: vec![
                        "winget".into(),
                        "install".into(),
                        "--id".into(),
                        "Docker.DockerDesktop".into(),
                        "-e".into(),
                    ],
                    requires_admin: false,
                    note: "安装后打开 Docker Desktop；首次启动可能需要 WSL 2。".into(),
                })
            } else if command_exists("apt-get") {
                Some(InstallAction {
                    command: vec![
                        "sudo".into(),
                        "apt-get".into(),
                        "install".into(),
                        "-y".into(),
                        "docker.io".into(),
                    ],
                    requires_admin: true,
                    note: "安装后运行 `sudo systemctl enable --now docker`。".into(),
                })
            } else if command_exists("dnf") {
                Some(InstallAction {
                    command: vec![
                        "sudo".into(),
                        "dnf".into(),
                        "install".into(),
                        "-y".into(),
                        "docker".into(),
                    ],
                    requires_admin: true,
                    note: "安装后运行 `sudo systemctl enable --now docker`。".into(),
                })
            } else if command_exists("pacman") {
                Some(InstallAction {
                    command: vec![
                        "sudo".into(),
                        "pacman".into(),
                        "-S".into(),
                        "--noconfirm".into(),
                        "docker".into(),
                    ],
                    requires_admin: true,
                    note: "安装后运行 `sudo systemctl enable --now docker`。".into(),
                })
            } else {
                None
            }
        }
        RuntimeKind::AppleContainer => Some(InstallAction {
            command: vec!["brew".into(), "install".into(), "container".into()],
            requires_admin: false,
            note: "安装后运行 `container system start`，按提示安装推荐内核。".into(),
        }),
        RuntimeKind::Wsl2 => Some(InstallAction {
            command: vec!["wsl.exe".into(), "--install".into()],
            requires_admin: true,
            note: "需要管理员权限，完成后通常要重启 Windows。".into(),
        }),
    }
}

pub fn start_action(kind: RuntimeKind) -> Option<InstallAction> {
    if !platform_supports(kind) {
        return None;
    }
    match kind {
        RuntimeKind::Docker => {
            if cfg!(target_os = "macos") {
                if Path::new("/Applications/Docker.app").exists() {
                    Some(InstallAction {
                        command: vec!["open".into(), "-a".into(), "Docker".into()],
                        requires_admin: false,
                        note: "等待 Docker Desktop 的 daemon 启动后重试。".into(),
                    })
                } else {
                    None
                }
            } else if cfg!(target_os = "windows") {
                Some(InstallAction {
                    command: vec![
                        "cmd".into(),
                        "/c".into(),
                        "start".into(),
                        "".into(),
                        "Docker Desktop".into(),
                    ],
                    requires_admin: false,
                    note: "等待 Docker Desktop 的 daemon 启动后重试。".into(),
                })
            } else {
                Some(InstallAction {
                    command: vec![
                        "sudo".into(),
                        "systemctl".into(),
                        "start".into(),
                        "docker".into(),
                    ],
                    requires_admin: true,
                    note: "需要管理员权限。".into(),
                })
            }
        }
        RuntimeKind::AppleContainer => Some(InstallAction {
            command: vec!["container".into(), "system".into(), "start".into()],
            requires_admin: false,
            note: "首次启动可能需要安装推荐内核。".into(),
        }),
        RuntimeKind::Wsl2 => None,
    }
}

fn command_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let candidate = dir.join(name);
                candidate.is_file()
                    || cfg!(target_os = "windows") && dir.join(format!("{name}.exe")).is_file()
            })
        })
        .unwrap_or(false)
}

pub async fn runner_views(registry: &BackendRegistry) -> Vec<RunnerView> {
    let statuses = registry.statuses().await;
    RuntimeKind::ALL
        .iter()
        .map(|kind| {
            let supported = platform_supports(*kind);
            let status = statuses
                .iter()
                .find(|backend| backend.kind == *kind)
                .map(|backend| backend.status.clone())
                .unwrap_or_else(|| RuntimeStatus::Unsupported {
                    reason: "runtime is not registered".into(),
                });
            let install = if supported && matches!(status, RuntimeStatus::NotInstalled { .. }) {
                install_action(*kind)
            } else {
                None
            };
            let start = if supported && matches!(status, RuntimeStatus::NotRunning { .. }) {
                start_action(*kind)
            } else {
                None
            };
            RunnerView {
                kind: *kind,
                name: kind.display_name().to_string(),
                supported,
                status,
                install,
                start,
            }
        })
        .collect()
}

pub async fn start_runner(kind: RuntimeKind) -> Result<String> {
    let action = start_action(kind).ok_or_else(|| {
        SetupError::Unsupported(format!("{} has no start action", kind.display_name()))
    })?;
    run_action(action)
}

pub async fn install_runner(kind: RuntimeKind) -> Result<String> {
    let action = install_action(kind).ok_or_else(|| {
        SetupError::Unsupported(format!(
            "{} cannot be installed on this platform",
            kind.display_name()
        ))
    })?;
    run_action(action)
}

/// Run one already selected install/start action. The desktop elevation bridge
/// uses this for commands that do not need administrator rights.
pub fn run_action(action: InstallAction) -> Result<String> {
    let Some(program) = action.command.first() else {
        return Err(SetupError::Invalid("empty install command".into()));
    };
    let output = Command::new(program)
        .args(&action.command[1..])
        .output()
        .map_err(|error| {
            SetupError::Runtime(format!("cannot run {}: {error}", action.command.join(" ")))
        })?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(SetupError::Runtime(format!(
            "{} exited with {}{}{}",
            action.command.join(" "),
            output.status,
            if stdout.is_empty() {
                String::new()
            } else {
                format!("\n{stdout}")
            },
            if stderr.is_empty() {
                String::new()
            } else {
                format!("\n{stderr}")
            }
        )));
    }
    let mut text = stdout;
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&stderr);
    }
    if !action.note.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&action.note);
    }
    Ok(text)
}

pub async fn plan(
    registry: &BackendRegistry,
    data_dir: &Path,
    launcher: &str,
) -> Result<SetupPlan> {
    let settings = load_settings(data_dir)?;
    let runners = runner_views(registry).await;
    let mut images = Vec::new();
    let mut environments = Vec::new();
    for runner in &runners {
        if !runner.status.is_ready() || !runner.supported {
            continue;
        }
        let Some(backend) = registry.get(runner.kind) else {
            continue;
        };
        if let Ok(refs) = backend.images().await {
            for reference in refs {
                if !images.iter().any(|image: &ImageChoice| {
                    image.runtime == runner.kind && image.reference == reference
                }) {
                    images.push(ImageChoice {
                        runtime: runner.kind,
                        reference,
                        local: true,
                    });
                }
            }
        }
        if let Ok(containers) = backend.list_containers().await {
            for container in containers {
                if container.image.to_ascii_lowercase().contains("mixumos") {
                    environments.push(EnvironmentChoice {
                        runtime: runner.kind,
                        id: container.id,
                        name: container.name,
                        image: container.image,
                        state: match container.state {
                            ContainerState::Running => "running",
                            ContainerState::Stopped => "stopped",
                            ContainerState::Paused => "paused",
                            ContainerState::Created => "created",
                            ContainerState::Unknown => "unknown",
                        }
                        .into(),
                    });
                }
            }
        }
    }
    Ok(SetupPlan {
        settings,
        runners,
        images,
        environments,
        clients: client_views(launcher),
        launcher: launcher.to_string(),
        default_image: DEFAULT_IMAGE.to_string(),
    })
}

pub async fn apply(
    registry: &BackendRegistry,
    data_dir: &Path,
    launcher: &str,
    request: SetupRequest,
) -> Result<SetupResult> {
    if !platform_supports(request.runtime) {
        return Err(SetupError::Unsupported(format!(
            "{} is not available on this platform",
            request.runtime.display_name()
        )));
    }
    let statuses = registry.statuses().await;
    let ready = statuses
        .iter()
        .find(|backend| backend.kind == request.runtime)
        .is_some_and(|backend| backend.status.is_ready());
    if !ready {
        return Err(SetupError::Invalid(format!(
            "{} is not ready; install or start it first",
            request.runtime.display_name()
        )));
    }
    let backend = registry.get(request.runtime).ok_or_else(|| {
        SetupError::Invalid(format!(
            "runtime {} is not registered",
            request.runtime.id()
        ))
    })?;
    let image = request
        .image
        .clone()
        .filter(|value| !value.trim().is_empty());
    if request.pull_image {
        let reference = image
            .as_deref()
            .ok_or_else(|| SetupError::Invalid("an image reference is required to pull".into()))?;
        backend
            .pull_image(reference)
            .await
            .map_err(|error| SetupError::Runtime(format!("cannot pull {reference}: {error}")))?;
    }
    let mut settings = load_settings(data_dir)?;
    settings.language = Some(request.language);
    settings.runtime = Some(request.runtime);
    settings.image = image.clone();
    settings.environment_id = request.environment_id.clone();
    settings.agents = request.clients.clone();
    settings.updated_at_ms = now_ms();
    settings.completed = true;
    save_settings(data_dir, &settings)?;
    write_cli_defaults(data_dir, request.runtime, image.as_deref())?;

    let mut configured = Vec::new();
    let mut failed = Vec::new();
    for id in &request.clients {
        match configure_client(id, launcher) {
            Ok(()) => configured.push(id.clone()),
            Err(error) => failed.push(ClientFailure {
                id: id.clone(),
                error: error.to_string(),
            }),
        }
    }
    Ok(SetupResult {
        settings,
        configured,
        failed,
        runner: request.runtime,
        image,
    })
}

/// Keep the CLI's `mixum env create` defaults in sync with setup.
fn write_cli_defaults(data_dir: &Path, runtime: RuntimeKind, image: Option<&str>) -> Result<()> {
    let path = data_dir.join("config.json");
    let mut root: Value = match fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(&text)?,
        _ => json!({}),
    };
    let object = root
        .as_object_mut()
        .ok_or_else(|| SetupError::Invalid("config.json root is not an object".into()))?;
    object.insert("defaultRuntime".into(), json!(runtime.id()));
    if let Some(image) = image.filter(|value| !value.trim().is_empty()) {
        object.insert("defaultImage".into(), json!(image));
    }
    let text = serde_json::to_string_pretty(&root)?;
    write_private_atomic(&path, &text)
}

// ---------------------------------------------------------------------------
// Agent MCP registration
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum ClientFormat {
    JsonMcp,
    ClaudeCode,
    CodexToml,
    VsCode,
    OpenCode,
}

struct ClientSpec {
    id: &'static str,
    name: &'static str,
    format: ClientFormat,
}

const CLIENTS: &[ClientSpec] = &[
    ClientSpec {
        id: "claude_code",
        name: "Claude Code",
        format: ClientFormat::ClaudeCode,
    },
    ClientSpec {
        id: "claude_desktop",
        name: "Claude Desktop",
        format: ClientFormat::JsonMcp,
    },
    ClientSpec {
        id: "codex",
        name: "Codex",
        format: ClientFormat::CodexToml,
    },
    ClientSpec {
        id: "cursor",
        name: "Cursor",
        format: ClientFormat::JsonMcp,
    },
    ClientSpec {
        id: "vscode",
        name: "VS Code",
        format: ClientFormat::VsCode,
    },
    ClientSpec {
        id: "opencode",
        name: "OpenCode",
        format: ClientFormat::OpenCode,
    },
];

fn spec(id: &str) -> Option<&'static ClientSpec> {
    CLIENTS.iter().find(|client| client.id == id)
}

fn home() -> Option<PathBuf> {
    dirs::home_dir()
}

fn client_path(id: &str) -> Option<PathBuf> {
    let home = home()?;
    let path = match id {
        "claude_code" => home.join(".claude.json"),
        "claude_desktop" => {
            if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Claude/claude_desktop_config.json")
            } else if cfg!(target_os = "windows") {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData/Roaming"))
                    .join("Claude/claude_desktop_config.json")
            } else {
                dirs::config_dir()?.join("Claude/claude_desktop_config.json")
            }
        }
        "codex" => home.join(".codex/config.toml"),
        "cursor" => home.join(".cursor/mcp.json"),
        "windsurf" => home.join(".codeium/windsurf/mcp_config.json"),
        "gemini_cli" => home.join(".gemini/settings.json"),
        "vscode" => {
            if cfg!(target_os = "macos") {
                home.join("Library/Application Support/Code/User/mcp.json")
            } else if cfg!(target_os = "windows") {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData/Roaming"))
                    .join("Code/User/mcp.json")
            } else {
                dirs::config_dir()?.join("Code/User/mcp.json")
            }
        }
        "opencode" => {
            if cfg!(target_os = "macos") {
                dirs::config_dir()?.join("opencode/opencode.json")
            } else if cfg!(target_os = "windows") {
                std::env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData/Roaming"))
                    .join("opencode/opencode.json")
            } else {
                dirs::config_dir()?.join("opencode/opencode.json")
            }
        }
        _ => return None,
    };
    Some(path)
}

fn client_views(launcher: &str) -> Vec<ClientView> {
    CLIENTS
        .iter()
        .map(|client| {
            let path = client_path(client.id);
            let path_string = path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            let (status, detail) = match path.as_ref() {
                None => (
                    "missing".to_string(),
                    Some("cannot determine config path".to_string()),
                ),
                Some(path) if !path.exists() => ("missing".to_string(), None),
                Some(path) => match inspect_client(path, client.format, launcher) {
                    Ok(true) => ("configured".to_string(), None),
                    Ok(false) => (
                        "needs_update".to_string(),
                        Some("mixum entry is missing or points elsewhere".to_string()),
                    ),
                    Err(error) => ("invalid".to_string(), Some(error.to_string())),
                },
            };
            ClientView {
                id: client.id.to_string(),
                name: client.name.to_string(),
                path: path_string,
                status,
                detail,
            }
        })
        .collect()
}

fn inspect_client(path: &Path, format: ClientFormat, launcher: &str) -> Result<bool> {
    let text = fs::read_to_string(path)?;
    match format {
        ClientFormat::CodexToml => {
            let document = text
                .parse::<DocumentMut>()
                .map_err(|error| SetupError::Toml(error.to_string()))?;
            let Some(servers) = document.get("mcp_servers").and_then(Item::as_table) else {
                return Ok(false);
            };
            Ok(servers
                .get(MCP_SERVER_KEY)
                .and_then(Item::as_table)
                .and_then(|entry| entry.get("command"))
                .and_then(Item::as_str)
                .is_some_and(|command| command == launcher))
        }
        ClientFormat::JsonMcp
        | ClientFormat::ClaudeCode
        | ClientFormat::VsCode
        | ClientFormat::OpenCode => {
            let root: Value = serde_json::from_str(&text)?;
            let key = match format {
                ClientFormat::VsCode => "servers",
                ClientFormat::OpenCode => "mcp",
                _ => "mcpServers",
            };
            let Some(entry) = root
                .get(key)
                .and_then(Value::as_object)
                .and_then(|servers| servers.get(MCP_SERVER_KEY))
            else {
                return Ok(false);
            };
            if format == ClientFormat::OpenCode {
                return Ok(entry
                    .get("command")
                    .and_then(Value::as_array)
                    .and_then(|command| command.first())
                    .and_then(Value::as_str)
                    .is_some_and(|command| command == launcher));
            }
            Ok(entry
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|command| command == launcher))
        }
    }
}

pub fn configure_client(id: &str, launcher: &str) -> Result<()> {
    let client =
        spec(id).ok_or_else(|| SetupError::Invalid(format!("unknown agent client: {id}")))?;
    let path = client_path(id)
        .ok_or_else(|| SetupError::Invalid(format!("cannot determine config path for {id}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let original = fs::read_to_string(&path).ok();
    if let Some(text) = &original {
        let backup = PathBuf::from(format!("{}.mixum.backup", path.display()));
        if !backup.exists() {
            fs::write(&backup, text)?;
        }
    }
    let text = match client.format {
        ClientFormat::CodexToml => configure_codex(original.as_deref(), launcher)?,
        _ => configure_json(original.as_deref(), client.format, launcher)?,
    };
    write_private_atomic(&path, &text)?;
    if !inspect_client(&path, client.format, launcher)? {
        return Err(SetupError::Invalid(format!(
            "readback did not contain the mixum entry: {}",
            path.display()
        )));
    }
    Ok(())
}

fn configure_json(original: Option<&str>, format: ClientFormat, launcher: &str) -> Result<String> {
    let mut root: Value = match original {
        Some(text) if !text.trim().is_empty() => serde_json::from_str(text)?,
        _ => json!({}),
    };
    if !root.is_object() {
        return Err(SetupError::Invalid(
            "configuration root is not a JSON object".into(),
        ));
    }
    let key = match format {
        ClientFormat::VsCode => "servers",
        ClientFormat::OpenCode => "mcp",
        _ => "mcpServers",
    };
    let object = root.as_object_mut().expect("checked object");
    let servers = object.entry(key.to_string()).or_insert_with(|| json!({}));
    let servers = servers
        .as_object_mut()
        .ok_or_else(|| SetupError::Invalid(format!("{key} is not an object")))?;
    let entry = match format {
        ClientFormat::VsCode => json!({
            "type": "stdio",
            "command": launcher,
            "args": ["mcp"],
        }),
        ClientFormat::OpenCode => json!({
            "type": "local",
            "command": [launcher, "mcp"],
            "enabled": true,
        }),
        _ => json!({
            "command": launcher,
            "args": ["mcp"],
        }),
    };
    servers.insert(MCP_SERVER_KEY.to_string(), entry);
    Ok(serde_json::to_string_pretty(&root)?)
}

fn configure_codex(original: Option<&str>, launcher: &str) -> Result<String> {
    let mut document = original
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|error| SetupError::Toml(error.to_string()))?;
    let servers = document
        .entry("mcp_servers")
        .or_insert(Item::Table(Table::new()));
    let servers = servers
        .as_table_mut()
        .ok_or_else(|| SetupError::Toml("mcp_servers is not a table".into()))?;
    let mut entry = Table::new();
    entry["command"] = value(launcher);
    let mut args = Array::new();
    args.push("mcp");
    entry["args"] = value(args);
    servers.insert(MCP_SERVER_KEY, Item::Table(entry));
    Ok(document.to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_registration_is_idempotent() {
        let text = configure_json(None, ClientFormat::JsonMcp, "/tmp/mixum").unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["mcpServers"]["mixum"]["command"], "/tmp/mixum");
        let text = configure_json(Some(&text), ClientFormat::JsonMcp, "/tmp/mixum").unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["mcpServers"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn vscode_and_opencode_shapes() {
        let vscode = configure_json(None, ClientFormat::VsCode, "/tmp/mixum").unwrap();
        let root: Value = serde_json::from_str(&vscode).unwrap();
        assert_eq!(root["servers"]["mixum"]["type"], "stdio");
        assert_eq!(root["servers"]["mixum"]["command"], "/tmp/mixum");
        let opencode = configure_json(None, ClientFormat::OpenCode, "/tmp/mixum").unwrap();
        let root: Value = serde_json::from_str(&opencode).unwrap();
        assert_eq!(root["mcp"]["mixum"]["type"], "local");
        assert_eq!(root["mcp"]["mixum"]["command"][0], "/tmp/mixum");
    }

    #[test]
    fn codex_registration_preserves_other_tables() {
        let original = "[model]\nname = \"x\"\n";
        let text = configure_codex(Some(original), "/tmp/mixum").unwrap();
        assert!(text.contains("[model]"));
        assert!(text.contains("mcp_servers.mixum"));
        let document = text.parse::<DocumentMut>().unwrap();
        assert_eq!(
            document["mcp_servers"]["mixum"]["command"].as_str(),
            Some("/tmp/mixum")
        );
    }

    #[test]
    fn platform_impossible_combinations_are_unsupported() {
        if !cfg!(target_os = "windows") {
            assert!(!platform_supports(RuntimeKind::Wsl2));
        }
        if !cfg!(target_os = "macos") {
            assert!(!platform_supports(RuntimeKind::AppleContainer));
        }
    }
}
