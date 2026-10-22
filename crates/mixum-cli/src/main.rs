mod base;
mod browser;
mod chrome;
mod cli;
mod config;
mod env;
mod map_ops;
mod mcp;
mod output;
mod ports;
mod state;
mod terminal;

use clap::Parser;

use cli::{
    BaseCommand, BrowserCommand, Cli, Command, ConfigCommand, EnvCommand, PortCommand,
    PrivilegeCommand, RuntimeCommand, SnapshotCommand,
};

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
            EnvCommand::Rebase { selector, base } => {
                env::rebase(&data_dir, &selector, &base, cli.json).await
            }
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
            EnvCommand::Open {
                selector,
                window,
                no_open,
                port,
            } => browser::open(&data_dir, &selector, window, port, no_open, cli.json).await,
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
                SnapshotCommand::Create {
                    selector,
                    label,
                    kind,
                } => {
                    map_ops::snapshot_create(
                        &data_dir,
                        &selector,
                        label.as_deref(),
                        &kind,
                        cli.json,
                    )
                    .await
                }
                SnapshotCommand::Fork {
                    selector,
                    node,
                    branch,
                } => map_ops::snapshot_fork(&data_dir, &selector, &node, &branch, cli.json).await,
                SnapshotCommand::Export {
                    selector,
                    node,
                    out,
                } => map_ops::snapshot_export(&data_dir, &selector, &node, &out, cli.json).await,
                SnapshotCommand::Import {
                    selector,
                    file,
                    origin,
                } => map_ops::snapshot_import(&data_dir, &selector, &file, &origin, cli.json).await,
                SnapshotCommand::Restore {
                    selector,
                    node,
                    yes,
                } => map_ops::snapshot_restore(&data_dir, &selector, &node, yes, cli.json).await,
            },
            EnvCommand::Screenshot {
                selector,
                window,
                out,
            } => map_ops::screenshot(&data_dir, &selector, window, out.as_deref(), cli.json).await,
            EnvCommand::Click {
                selector,
                window,
                x,
                y,
                button,
            } => map_ops::click(&data_dir, &selector, window, x, y, &button, cli.json).await,
            EnvCommand::Move {
                selector,
                window,
                x,
                y,
            } => map_ops::move_pointer(&data_dir, &selector, window, x, y, cli.json).await,
            EnvCommand::Type {
                selector,
                window,
                text,
            } => map_ops::type_text(&data_dir, &selector, window, &text, cli.json).await,
            EnvCommand::Key {
                selector,
                window,
                keysym,
                mods,
            } => map_ops::key(&data_dir, &selector, window, &keysym, &mods, cli.json).await,
            EnvCommand::Scroll {
                selector,
                window,
                x,
                y,
                direction,
            } => map_ops::scroll(&data_dir, &selector, window, x, y, &direction, cli.json).await,
            EnvCommand::Focus { selector, window } => {
                map_ops::focus(&data_dir, &selector, window, cli.json).await
            }
        },
        Command::Browser { command } => match command {
            BrowserCommand::Install => chrome::install(cli.json),
            BrowserCommand::Export { domains } => chrome::export(&domains, cli.json),
            BrowserCommand::Share { selector, domains } => {
                chrome::share(&data_dir, &selector, &domains, cli.json).await
            }
        },
        Command::Base { command } => match command {
            BaseCommand::Create {
                name,
                image,
                runtime,
            } => {
                base::create(
                    &data_dir,
                    &name,
                    image.as_deref(),
                    runtime.map(|runtime| runtime.kind()),
                    cli.json,
                )
                .await
            }
            BaseCommand::Shell { name } => base::shell(&data_dir, &name).await,
            BaseCommand::Commit { name, tag } => {
                base::commit(&data_dir, &name, tag.as_deref(), cli.json).await
            }
            BaseCommand::List => base::list(&data_dir, cli.json).await,
        },
        Command::Port { command } => match command {
            PortCommand::Check { port } => ports::check_command(port, cli.json).await,
            PortCommand::List => env::port_list(cli.json).await,
        },
        Command::Privilege { command } => match command {
            PrivilegeCommand::Watch { selector } => {
                map_ops::privilege_watch(&data_dir, &selector, cli.json).await
            }
            PrivilegeCommand::Approve {
                selector,
                id,
                reason,
            } => {
                map_ops::privilege_decide(&data_dir, &selector, &id, true, &reason, cli.json).await
            }
            PrivilegeCommand::Deny {
                selector,
                id,
                reason,
            } => {
                map_ops::privilege_decide(&data_dir, &selector, &id, false, &reason, cli.json).await
            }
        },
        Command::Mcp => mcp::serve(),
        Command::Desktop => launch_desktop(cli.json),
        Command::Config { command } => match command {
            ConfigCommand::Show => config::show(&data_dir, cli.json),
            ConfigCommand::Set { key, value } => config::set(&data_dir, &key, &value, cli.json),
            ConfigCommand::Unset { key } => config::unset(&data_dir, &key, cli.json),
        },
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
