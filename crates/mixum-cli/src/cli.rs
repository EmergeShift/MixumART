use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use mixum_env_core::RuntimeKind;

#[derive(Debug, Parser)]
#[command(
    name = "mixum",
    version,
    about = "MixumART local environment control plane",
    long_about = "Create, inspect, monitor and attach to MixumOS environments through Docker, Apple Container or WSL 2. All commands support --json for agents."
)]
pub struct Cli {
    /// Machine-readable JSON output.
    #[arg(long, global = true)]
    pub json: bool,
    /// Override the state directory (defaults to the MixumART app data directory).
    #[arg(long, global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Check runtimes, data directory and port range.
    Doctor,
    /// Inspect container runtimes.
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommand,
    },
    /// Manage environments.
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
    /// Manage shared base environments (母体环境).
    Base {
        #[command(subcommand)]
        command: BaseCommand,
    },
    /// Check and inspect host ports.
    Port {
        #[command(subcommand)]
        command: PortCommand,
    },
    /// Read or update CLI defaults.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Review host privilege requests from agents.
    Privilege {
        #[command(subcommand)]
        command: PrivilegeCommand,
    },
    /// Run the stdio MCP server for agents.
    Mcp,
    /// Open the installed MixumART desktop app.
    Desktop,
    /// Print version and build information.
    Version,
}

#[derive(Debug, Subcommand)]
pub enum RuntimeCommand {
    /// List runtime availability.
    List,
}

#[derive(Debug, Subcommand)]
pub enum EnvCommand {
    /// List managed MixumOS environments. Use --all for every runtime container.
    List {
        #[arg(long)]
        all: bool,
    },
    /// Create a MixumOS environment.
    Create(CreateArgs),
    /// Recreate an env on a new base while keeping its named work/home volumes.
    Rebase {
        selector: String,
        #[arg(long)]
        base: String,
    },
    /// Show one environment.
    Show { selector: String },
    /// Start an environment.
    Start { selector: String },
    /// Stop an environment.
    Stop { selector: String },
    /// Remove an environment. Refuses volumes/data deletion unless --force.
    Remove {
        selector: String,
        #[arg(long)]
        force: bool,
    },
    /// List MAP windows in a running environment.
    Windows {
        selector: String,
        #[arg(long, default_value_t = 30)]
        timeout: u64,
    },
    /// Open a read-only MAP window in the local browser.
    Open {
        selector: String,
        /// Window id; omitted when the environment has exactly one window.
        #[arg(long)]
        window: Option<u32>,
        /// Print the URL but do not launch a browser.
        #[arg(long)]
        no_open: bool,
        /// Bind port; 0 picks an ephemeral port.
        #[arg(long, default_value_t = 0)]
        port: u16,
    },
    /// Read recent container logs.
    Logs {
        selector: String,
        #[arg(long, default_value_t = 200)]
        tail: usize,
    },
    /// Attach an interactive PTY to a running environment.
    Terminal {
        selector: String,
        #[arg(long, default_value = "/bin/sh")]
        shell: String,
    },
    /// Snapshot operations through MAP.
    Snapshot {
        #[command(subcommand)]
        command: SnapshotCommand,
    },
    /// One-shot window screenshot (PNG).
    Screenshot {
        selector: String,
        #[arg(long)]
        window: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Inject a pointer click into a window.
    Click {
        selector: String,
        #[arg(long)]
        window: u32,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long, default_value = "left")]
        button: String,
    },
    /// Move the pointer inside a window.
    Move {
        selector: String,
        #[arg(long)]
        window: u32,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
    },
    /// Commit composed text into a window (IME-safe).
    Type {
        selector: String,
        #[arg(long)]
        window: u32,
        text: String,
    },
    /// Send one keysym (e.g. Return, ctrl+a) to a window.
    Key {
        selector: String,
        #[arg(long)]
        window: u32,
        keysym: String,
        #[arg(long, value_delimiter = ',')]
        mods: Vec<String>,
    },
    /// Scroll a window at a point.
    Scroll {
        selector: String,
        #[arg(long)]
        window: u32,
        #[arg(long)]
        x: f64,
        #[arg(long)]
        y: f64,
        #[arg(long, default_value = "down")]
        direction: String,
    },
    /// Claim a window for agent/user focus.
    Focus {
        selector: String,
        #[arg(long)]
        window: u32,
    },
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCommand {
    /// List snapshots.
    List { selector: String },
    /// Create a snapshot (disk / memory / hybrid).
    Create {
        selector: String,
        #[arg(long)]
        label: Option<String>,
        #[arg(long, default_value = "disk")]
        kind: String,
    },
    /// Fork a snapshot into a new branch.
    Fork {
        selector: String,
        node: String,
        #[arg(long)]
        branch: String,
    },
    /// Export a snapshot as a `.mart` bundle to the host.
    Export {
        selector: String,
        node: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Import a `.mart` bundle into an environment.
    Import {
        selector: String,
        #[arg(long)]
        file: PathBuf,
        #[arg(long, default_value = "cli")]
        origin: String,
    },
    /// Restore a snapshot in place.
    Restore {
        selector: String,
        node: String,
        /// Confirm the destructive restore.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum BaseCommand {
    /// Create a root builder container for a base environment.
    Create {
        name: String,
        #[arg(long)]
        image: Option<String>,
        #[arg(long, value_enum)]
        runtime: Option<RuntimeArg>,
    },
    /// Open a root shell in the base builder (human only).
    Shell { name: String },
    /// Commit the builder rootfs into an immutable image version.
    Commit {
        name: String,
        #[arg(long)]
        tag: Option<String>,
    },
    /// List base environments.
    List,
}

#[derive(Debug, Subcommand)]
pub enum PrivilegeCommand {
    /// Watch pending sudo requests for one environment.
    Watch { selector: String },
    /// Approve one request after host-password authentication.
    Approve {
        selector: String,
        id: String,
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// Deny one request.
    Deny {
        selector: String,
        id: String,
        #[arg(long, default_value = "")]
        reason: String,
    },
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Human title recorded in the local registry.
    #[arg(long)]
    pub title: String,
    /// Agent collaboration mode recorded in the local registry.
    #[arg(long, value_enum, default_value_t = Method::Subagent)]
    pub method: Method,
    /// Environment profile. mixumos requires an agentd-capable image.
    #[arg(long, value_enum, default_value_t = Profile::Mixumos)]
    pub profile: Profile,
    /// Image reference. Omit for mixumos to use the only local mixumos image.
    #[arg(long)]
    pub image: Option<String>,
    /// Base environment spec (`name` or `name:tag`).
    #[arg(long)]
    pub base: Option<String>,
    /// Container name. Generated when omitted.
    #[arg(long)]
    pub name: Option<String>,
    /// Host port for MAP 7601. 0 allocates from 17600-17999.
    #[arg(long, default_value_t = 0)]
    pub port: u16,
    /// Runtime. Required when more than one runtime is ready.
    #[arg(long, value_enum)]
    pub runtime: Option<RuntimeArg>,
    /// Seconds to wait for MAP after start. 0 skips the wait.
    #[arg(long, default_value_t = 30)]
    pub wait_map: u64,
    /// Create but do not start the container.
    #[arg(long)]
    pub no_start: bool,
    /// Create a root container. Human-only: requires typing ROOT after the risk notice.
    #[arg(long)]
    pub root: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Method {
    Subagent,
    AsTool,
    Isolated,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Subagent => "subagent",
            Self::AsTool => "as-tool",
            Self::Isolated => "isolated",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Profile {
    Mixumos,
    Generic,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RuntimeArg {
    Docker,
    Wsl2,
    Apple,
}

impl RuntimeArg {
    pub const fn kind(self) -> RuntimeKind {
        match self {
            Self::Docker => RuntimeKind::Docker,
            Self::Wsl2 => RuntimeKind::Wsl2,
            Self::Apple => RuntimeKind::AppleContainer,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print current defaults.
    Show,
    /// Set a default (default-runtime, default-image).
    Set { key: String, value: String },
    /// Remove a default.
    Unset { key: String },
}

#[derive(Debug, Subcommand)]
pub enum PortCommand {
    /// Check whether 127.0.0.1:PORT can be bound.
    Check { port: u16 },
    /// List host ports currently claimed by runtime containers.
    List,
}
