//! 宿主侧持久 PTY：把 runtime CLI 的 exec 包进 pseudo terminal。
//!
//! 不经过 agentd，因此只要求容器正在运行；Docker/Apple 用对应 CLI，
//! WSL2 用 wsl.exe。输出通过 Tauri 事件 `terminal-data` 推给前端。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use mixum_env_core::RuntimeKind;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde_json::json;
use tauri::{AppHandle, Emitter};

pub struct TerminalSession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
}

#[derive(Default)]
pub struct TerminalManager {
    next_id: AtomicU32,
    sessions: Mutex<HashMap<u32, TerminalSession>>,
}

impl TerminalManager {
    pub fn open(
        &self,
        app: AppHandle,
        kind: RuntimeKind,
        container_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<u32, String> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("打开 PTY 失败：{error}"))?;

        let mut command = match kind {
            RuntimeKind::Docker => {
                let mut command = CommandBuilder::new("docker");
                command.args(["exec", "-it", container_id, "/bin/sh"]);
                command
            }
            RuntimeKind::AppleContainer => {
                let mut command = CommandBuilder::new("container");
                command.args(["exec", "-it", container_id, "/bin/sh"]);
                command
            }
            RuntimeKind::Wsl2 => {
                let mut command = CommandBuilder::new("wsl.exe");
                command.args(["-d", container_id, "-e", "/bin/sh"]);
                command
            }
        };
        command.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| format!("启动容器终端失败：{error}"))?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| format!("读取 PTY 失败：{error}"))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| format!("写入 PTY 失败：{error}"))?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let app_for_reader = app.clone();
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
                        let _ = app_for_reader
                            .emit("terminal-data", json!({ "terminal": id, "data": text }));
                    }
                    Err(_) => break,
                }
            }
        });

        self.sessions.lock().expect("terminal sessions").insert(
            id,
            TerminalSession {
                master: pair.master,
                writer,
                child,
            },
        );
        Ok(id)
    }

    pub fn input(&self, id: u32, data: &str) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
        let session = sessions.get_mut(&id).ok_or("终端不存在")?;
        session
            .writer
            .write_all(data.as_bytes())
            .and_then(|_| session.writer.flush())
            .map_err(|error| format!("写入终端失败：{error}"))
    }

    pub fn resize(&self, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        let sessions = self.sessions.lock().map_err(|error| error.to_string())?;
        let session = sessions.get(&id).ok_or("终端不存在")?;
        session
            .master
            .resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| format!("调整终端大小失败：{error}"))
    }

    pub fn close(&self, id: u32) -> Result<(), String> {
        let mut sessions = self.sessions.lock().map_err(|error| error.to_string())?;
        if let Some(mut session) = sessions.remove(&id) {
            let _ = session.child.kill();
        }
        Ok(())
    }
}

impl Drop for TerminalManager {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, mut session) in sessions.drain() {
                let _ = session.child.kill();
            }
        }
    }
}
