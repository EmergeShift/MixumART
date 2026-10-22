use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::output::{CliError, Result};

const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvRecord {
    pub id: String,
    pub name: String,
    pub title: String,
    pub method: String,
    pub runtime: String,
    pub image: String,
    /// 提交母体环境使用的 base spec（`name:tag`）；普通环境为空。
    #[serde(default)]
    pub base: Option<String>,
    /// Explicitly approved root container.
    #[serde(default)]
    pub root: bool,
    pub port: u16,
    pub created_at_ms: u128,
    #[serde(default)]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub version: u32,
    #[serde(default)]
    pub envs: Vec<EnvRecord>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            version: STATE_VERSION,
            envs: Vec::new(),
        }
    }
}

impl State {
    pub fn find(&self, id: &str) -> Option<&EnvRecord> {
        self.envs.iter().find(|record| record.id == id)
    }

    pub fn upsert(&mut self, record: EnvRecord) {
        if let Some(existing) = self.envs.iter_mut().find(|item| item.id == record.id) {
            *existing = record;
        } else {
            self.envs.push(record);
        }
    }

    pub fn remove(&mut self, id: &str) {
        self.envs.retain(|record| record.id != id);
    }
}

pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join("state.json"),
        }
    }

    pub fn load(&self) -> Result<State> {
        match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|error| {
                CliError::io(format!(
                    "state file {} is corrupt: {error}; move it away to reset",
                    self.path.display()
                ))
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
            Err(error) => Err(CliError::io(format!(
                "cannot read state file {}: {error}",
                self.path.display()
            ))),
        }
    }

    pub fn save(&self, state: &State) -> Result<()> {
        let Some(parent) = self.path.parent() else {
            return Err(CliError::io("state path has no parent directory"));
        };
        fs::create_dir_all(parent).map_err(|error| {
            CliError::io(format!(
                "cannot create data directory {}: {error}",
                parent.display()
            ))
        })?;
        let temp = parent.join(format!(".state-{}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(state)
            .map_err(|error| CliError::io(format!("cannot serialize state: {error}")))?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temp)
            .map_err(|error| CliError::io(format!("cannot write state: {error}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| CliError::io(format!("cannot chmod state: {error}")))?;
        }
        file.write_all(text.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|error| CliError::io(format!("cannot write state: {error}")))?;
        drop(file);
        fs::rename(&temp, &self.path).map_err(|error| {
            CliError::io(format!(
                "cannot commit state {}: {error}",
                self.path.display()
            ))
        })
    }
}

pub struct StateLock {
    file: File,
}

impl StateLock {
    pub fn acquire(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir).map_err(|error| {
            CliError::io(format!(
                "cannot create data directory {}: {error}",
                data_dir.display()
            ))
        })?;
        let path = data_dir.join("state.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| CliError::io(format!("cannot open state lock: {error}")))?;
        file.lock_exclusive()
            .map_err(|error| CliError::io(format!("cannot lock state: {error}")))?;
        Ok(Self { file })
    }
}

impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub fn resolve_data_dir(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(dir) = explicit {
        return Ok(dir.to_path_buf());
    }
    if let Some(dir) = std::env::var_os("MIXUM_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base =
        dirs::data_dir().ok_or_else(|| CliError::io("cannot determine user data directory"))?;
    Ok(base.join("dev.mixum.agentenv").join("mixum-cli"))
}

pub fn mount_volume(source: &str, target: &str) -> mixum_env_core::Mount {
    mixum_env_core::Mount {
        kind: "volume".into(),
        source: source.into(),
        target: target.into(),
        read_only: false,
    }
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trip_and_lock() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        let mut state = State::default();
        state.upsert(EnvRecord {
            id: "abc".into(),
            name: "mixum-test".into(),
            title: "test".into(),
            method: "subagent".into(),
            runtime: "docker".into(),
            image: "mixumos:test".into(),
            base: None,
            root: false,
            port: 17601,
            created_at_ms: 1,
            fingerprint: None,
        });
        store.save(&state).unwrap();
        assert_eq!(store.load().unwrap().envs.len(), 1);
        let lock = StateLock::acquire(dir.path()).unwrap();
        drop(lock);
    }
}
