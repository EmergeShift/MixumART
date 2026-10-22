use std::io::{IsTerminal, Read, Write};

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use mixum_env_core::RuntimeKind;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::env;
use crate::output::{CliError, Result};

pub async fn attach(data_dir: &std::path::Path, selector: &str, shell: &str) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(CliError::usage(
            "terminal requires an interactive TTY; use scripts for non-interactive access",
        ));
    }
    let registry = env::registry();
    let resolved = env::find_with(&registry, data_dir, selector).await?;
    if resolved.container.state != mixum_env_core::ContainerState::Running {
        return Err(CliError::conflict(format!(
            "environment {} is not running",
            resolved.container.name
        )));
    }
    let (cols, rows) = terminal_size::terminal_size()
        .map(|(width, height)| (width.0, height.0))
        .unwrap_or((100, 30));
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| CliError::runtime(format!("open PTY failed: {error}")))?;
    let mut command = match resolved.kind {
        RuntimeKind::Docker => {
            let mut command = CommandBuilder::new("docker");
            command.args(["exec", "-it", &resolved.container.id, shell]);
            command
        }
        RuntimeKind::AppleContainer => {
            let mut command = CommandBuilder::new("container");
            command.args(["exec", "-it", &resolved.container.id, shell]);
            command
        }
        RuntimeKind::Wsl2 => {
            let mut command = CommandBuilder::new("wsl.exe");
            command.args(["-d", &resolved.container.id, "-e", shell]);
            command
        }
    };
    command.env("TERM", "xterm-256color");
    let mut child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| CliError::runtime(format!("spawn terminal failed: {error}")))?;
    drop(pair.slave);
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| CliError::runtime(format!("read PTY failed: {error}")))?;
    let mut writer = pair
        .master
        .take_writer()
        .map_err(|error| CliError::runtime(format!("write PTY failed: {error}")))?;

    enable_raw_mode().map_err(|error| CliError::runtime(format!("raw mode failed: {error}")))?;
    let output = std::thread::spawn(move || {
        let mut stdout = std::io::stdout();
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    let _ = stdout.write_all(&buffer[..read]);
                    let _ = stdout.flush();
                }
            }
        }
    });
    let input = std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut buffer = [0u8; 4096];
        loop {
            match stdin.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if writer.write_all(&buffer[..read]).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
            }
        }
    });
    let status = child.wait();
    let _ = disable_raw_mode();
    drop(input);
    let _ = output.join();
    status.map_err(|error| CliError::runtime(format!("terminal failed: {error}")))?;
    Ok(())
}
