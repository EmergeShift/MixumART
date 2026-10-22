use std::fs;
use std::io::Write;
use std::path::Path;

use mixum_env_core::RuntimeKind;
use serde::{Deserialize, Serialize};

use crate::output::{self, CliError, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub default_runtime: Option<RuntimeKind>,
    #[serde(default)]
    pub default_image: Option<String>,
}

pub fn path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("config.json")
}

pub fn load(data_dir: &Path) -> Result<Config> {
    match fs::read_to_string(path(data_dir)) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|error| CliError::io(format!("config.json is corrupt: {error}"))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(error) => Err(CliError::io(format!("cannot read config.json: {error}"))),
    }
}

pub fn save(data_dir: &Path, config: &Config) -> Result<()> {
    fs::create_dir_all(data_dir)
        .map_err(|error| CliError::io(format!("cannot create data dir: {error}")))?;
    let target = path(data_dir);
    let temp = data_dir.join(format!(".config-{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(config)
        .map_err(|error| CliError::io(format!("cannot serialize config: {error}")))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temp)
        .map_err(|error| CliError::io(format!("cannot write config: {error}")))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| CliError::io(format!("cannot chmod config: {error}")))?;
    }
    file.write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
        .map_err(|error| CliError::io(format!("cannot write config: {error}")))?;
    drop(file);
    fs::rename(&temp, &target)
        .map_err(|error| CliError::io(format!("cannot commit config: {error}")))
}

pub fn show(data_dir: &Path, json: bool) -> Result<()> {
    let config = load(data_dir)?;
    if json {
        output::print_json(&config);
    } else {
        println!(
            "defaultRuntime: {}",
            config
                .default_runtime
                .map(|runtime| runtime.id())
                .unwrap_or("-")
        );
        println!(
            "defaultImage:   {}",
            config.default_image.as_deref().unwrap_or("-")
        );
    }
    Ok(())
}

pub fn set(data_dir: &Path, key: &str, value: &str, json: bool) -> Result<()> {
    let mut config = load(data_dir)?;
    match key {
        "default-runtime" => {
            config.default_runtime = Some(match value {
                "docker" => RuntimeKind::Docker,
                "wsl2" => RuntimeKind::Wsl2,
                "apple" | "apple_container" => RuntimeKind::AppleContainer,
                _ => {
                    return Err(CliError::usage(
                        "default-runtime must be docker, wsl2 or apple",
                    ))
                }
            });
        }
        "default-image" => {
            if value.trim().is_empty() {
                return Err(CliError::usage("default-image cannot be empty"));
            }
            config.default_image = Some(value.to_string());
        }
        _ => {
            return Err(CliError::usage(
                "config keys: default-runtime, default-image",
            ))
        }
    }
    save(data_dir, &config)?;
    if json {
        output::print_json(&config);
    } else {
        println!("updated {key} = {value}");
    }
    Ok(())
}

pub fn unset(data_dir: &Path, key: &str, json: bool) -> Result<()> {
    let mut config = load(data_dir)?;
    match key {
        "default-runtime" => config.default_runtime = None,
        "default-image" => config.default_image = None,
        _ => {
            return Err(CliError::usage(
                "config keys: default-runtime, default-image",
            ))
        }
    }
    save(data_dir, &config)?;
    if json {
        output::print_json(&config);
    } else {
        println!("unset {key}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            default_runtime: Some(RuntimeKind::AppleContainer),
            default_image: Some("mixumos:test".into()),
        };
        save(dir.path(), &config).unwrap();
        assert_eq!(
            load(dir.path()).unwrap().default_image.as_deref(),
            Some("mixumos:test")
        );
    }
}
