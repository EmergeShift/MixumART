//! Best-effort release check. It must never block or fail a normal command.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const RELEASES_API: &str = "https://api.github.com/repos/EmergeShift/MixumART/releases/latest";
const CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    checked_at_ms: u128,
    latest: String,
    url: String,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("update-check.json")
}

fn read_cache(data_dir: &Path) -> Option<Cache> {
    let text = fs::read_to_string(cache_path(data_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(data_dir: &Path, cache: &Cache) {
    let _ = fs::create_dir_all(data_dir);
    let path = cache_path(data_dir);
    let temp = data_dir.join(format!(".update-{}.tmp", std::process::id()));
    if let Ok(text) = serde_json::to_string_pretty(cache) {
        if fs::write(&temp, text).is_ok() {
            let _ = fs::rename(temp, path);
        }
    }
}

fn newer(current: &str, latest: &str) -> bool {
    let Ok(current) = semver::Version::parse(current.trim_start_matches('v')) else {
        return false;
    };
    let Ok(latest) = semver::Version::parse(latest.trim_start_matches('v')) else {
        return false;
    };
    latest > current
}

/// Return a newer release, using a 12-hour cache. Network failures are silent.
pub fn check_update(data_dir: &Path, current: &str) -> Option<UpdateInfo> {
    if std::env::var_os("MIXUM_NO_UPDATE_CHECK").is_some() {
        return None;
    }
    let cached = read_cache(data_dir);
    let cache = match cached {
        Some(cache)
            if now_ms().saturating_sub(cache.checked_at_ms) < CHECK_INTERVAL.as_millis() =>
        {
            cache
        }
        _ => {
            let latest = fetch_latest()?;
            let cache = Cache {
                checked_at_ms: now_ms(),
                latest: latest.0,
                url: latest.1,
            };
            write_cache(data_dir, &cache);
            cache
        }
    };
    if newer(current, &cache.latest) {
        Some(UpdateInfo {
            current: current.to_string(),
            latest: cache.latest,
            url: cache.url,
        })
    } else {
        None
    }
}

fn fetch_latest() -> Option<(String, String)> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(3))
        .build();
    let response = agent
        .get(RELEASES_API)
        .set("User-Agent", "mixum-setup-update-check")
        .set("Accept", "application/vnd.github+json")
        .call()
        .ok()?;
    let value: Value = response.into_json().ok()?;
    let tag = value.get("tag_name")?.as_str()?.to_string();
    let url = value
        .get("html_url")
        .and_then(Value::as_str)
        .unwrap_or("https://github.com/EmergeShift/MixumART/releases")
        .to_string();
    Some((tag, url))
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn compares_versions_without_downgrades() {
        assert!(newer("0.1.0", "0.1.1"));
        assert!(newer("v0.1.0", "v0.2.0"));
        assert!(!newer("0.2.0", "0.1.9"));
        assert!(!newer("0.2.0", "0.2.0"));
    }
}
