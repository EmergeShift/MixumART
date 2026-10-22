mod cli;
mod env;
mod map_ops;
mod mcp;
mod output;
mod ports;
mod state;
mod terminal;

use clap::Parser;

use cli::{Cli, Command, EnvCommand, PortCommand, RuntimeCommand, SnapshotCommand};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let json = cli.json;
    let result = run(cli).await;
    if let Err(error) = result {
        if json {
            output::print_json(&serde_json::json!({
                "error": { "code": error.code, "message": error.message }
            }));
        }
        eprintln!("mixum: {}", error.message);
        std::process::exit(error.code);
    }
}

fn launch_desktop(json: bool) -> std::result::Result<(), output::CliError> {
    let status = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
            .args(["-b", "dev.mixum.agentenv"])
            .status()
    } else {
        let app = std::env::var_os("MIXUM_DESKTOP_APP").ok_or_else(|| {
            output::CliError::usage(
                "set MIXUM_DESKTOP_APP to the mixum desktop executable on this platform",
            )
        })?;
        std::process::Command::new(app).status()
    }
    .map_err(|error| output::CliError::runtime(format!("cannot launch desktop app: {error}")))?;
    if !status.success() {
        return Err(output::CliError::runtime(format!(
            "desktop app exited with {status}"
        )));
    }
    if json {
        output::print_json(&serde_json::json!({"launched": true, "app": "dev.mixum.agentenv"}));
    } else {
        println!("opened mixum ART");
    }
    Ok(())
}

async fn run(cli: Cli) -> Result<(), output::CliError> {
    let data_dir = state::resolve_data_dir(cli.data_dir.as_deref())?;
    match cli.command {
        Command::Doctor => env::doctor(&data_dir, cli.json).await,
        Command::Runtime { command } => match command {
            RuntimeCommand::List => env::runtime_list(cli.json).await,
        },
        Command::Env { command } => match command {
            EnvCommand::List { all } => env::list(&data_dir, all, cli.json).await,
            EnvCommand::Create(args) => env::create(&data_dir, args, cli.json).await,
            EnvCommand::Show { selector } => env::show(&data_dir, &selector, cli.json).await,
            EnvCommand::Start { selector } => {
                env::change_state(&data_dir, &selector, env::StateChange::Start, cli.json).await
            }
            EnvCommand::Stop { selector } => {
                env::change_state(&data_dir, &selector, env::StateChange::Stop, cli.json).await
            }
            EnvCommand::Remove { selector, force } => {
                env::remove(&data_dir, &selector, force, cli.json).await
            }
            EnvCommand::Windows { selector, timeout } => {
                map_ops::windows(&data_dir, &selector, timeout, cli.json).await
            }
            EnvCommand::Logs { selector, tail } => {
                env::logs(&data_dir, &selector, tail, cli.json).await
            }
            EnvCommand::Terminal { selector, shell } => {
                terminal::attach(&data_dir, &selector, &shell).await
            }
            EnvCommand::Snapshot { command } => match command {
                SnapshotCommand::List { selector } => {
                    map_ops::snapshot_list(&data_dir, &selector, cli.json).await
                }
                SnapshotCommand::Create { selector, label } => {
                    map_ops::snapshot_create(&data_dir, &selector, label.as_deref(), cli.json).await
                }
                SnapshotCommand::Restore {
                    selector,
                    node,
                    yes,
                } => map_ops::snapshot_restore(&data_dir, &selector, &node, yes, cli.json).await,
            },
        },
        Command::Port { command } => match command {
            PortCommand::Check { port } => ports::check_command(port, cli.json).await,
            PortCommand::List => env::port_list(cli.json).await,
        },
        Command::Mcp => mcp::serve(),
        Command::Desktop => launch_desktop(cli.json),
        Command::Version => {
            if cli.json {
                output::print_json(&serde_json::json!({
                    "name": "mixum",
                    "version": env!("CARGO_PKG_VERSION"),
                }));
            } else {
                println!("mixum {}", env!("CARGO_PKG_VERSION"));
            }
            Ok(())
        }
    }
}
