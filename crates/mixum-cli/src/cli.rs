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
}

#[derive(Debug, Subcommand)]
pub enum SnapshotCommand {
    /// List snapshots.
    List { selector: String },
    /// Create a disk snapshot.
    Create {
        selector: String,
        #[arg(long)]
        label: Option<String>,
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
