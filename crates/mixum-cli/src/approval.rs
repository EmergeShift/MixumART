//! Local two-party consent records for root containers.
//!
//! The agent can create a request (with its stance and reason); only a human at
//! an interactive terminal can approve it after host-password authentication.

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::output::{self, CliError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RootRequest {
    pub id: String,
    pub title: String,
    pub image: String,
    /// `agree` or `disagree`; the agent must state why when it agrees.
    pub agent_stance: String,
    pub agent_reason: String,
    pub created_at_ms: u128,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub consumed: bool,
}

pub fn request(
    data_dir: &Path,
    title: &str,
    image: &str,
    stance: &str,
    reason: &str,
    json: bool,
) -> Result<()> {
    if !matches!(stance, "agree" | "disagree") {
        return Err(CliError::usage("stance must be agree or disagree"));
    }
    if stance == "agree" && reason.trim().is_empty() {
        return Err(CliError::usage(
            "agent must explain why it agrees to a root container",
        ));
    }
    if title.trim().is_empty() || image.trim().is_empty() {
        return Err(CliError::usage("title and image are required"));
    }
    let id = format!("root-{}", ulid::Ulid::new());
    let request = RootRequest {
        id: id.clone(),
        title: title.trim().to_owned(),
        image: image.trim().to_owned(),
        agent_stance: stance.to_owned(),
        agent_reason: reason.trim().to_owned(),
        created_at_ms: now_ms(),
        approved: false,
        consumed: false,
    };
    save(data_dir, &request)?;
    audit(
        data_dir,
        "root_request",
        &request.id,
        json!({"title": request.title, "stance": request.agent_stance}),
    );
    if json {
        output::print_json(&json!({
            "id": id,
            "approveCommand": format!("mixum root approve {id}"),
            "request": request,
        }));
    } else {
        println!("root request {id}");
        println!("agent stance: {stance}");
        if !reason.trim().is_empty() {
            println!("agent reason: {}", reason.trim());
        }
        println!("approve as human: mixum root approve {id}");
    }
    Ok(())
}

pub fn approve(data_dir: &Path, id: &str, json: bool) -> Result<()> {
    let mut request = load(data_dir, id)?;
    if request.consumed {
        return Err(CliError::conflict("request already consumed"));
    }
    if request.approved {
        return Err(CliError::conflict("request already approved"));
    }
    eprintln!("ROOT CONTAINER REQUEST {}", request.id);
    eprintln!("title: {}", request.title);
    eprintln!("image: {}", request.image);
    eprintln!("agent stance: {}", request.agent_stance);
    eprintln!("agent reason: {}", request.agent_reason);
    eprintln!("A root container can modify its own system and read its mounted secrets.");
    host_authenticate()?;
    request.approved = true;
    save(data_dir, &request)?;
    audit(
        data_dir,
        "root_approved",
        &request.id,
        json!({"title": request.title}),
    );
    if json {
        output::print_json(&json!({"id": request.id, "approved": true}));
    } else {
        println!("approved root request {}", request.id);
        println!(
            "create with: mixum env create --title \"{}\" --image {} --root-request {}",
            request.title, request.image, request.id
        );
    }
    Ok(())
}

pub fn approved(data_dir: &Path, id: &str) -> Result<RootRequest> {
    let request = load(data_dir, id)?;
    if !request.approved || request.consumed {
        return Err(CliError::conflict(
            "root request is not approved or already used",
        ));
    }
    Ok(request)
}

pub fn consume(data_dir: &Path, id: &str) -> Result<()> {
    let mut request = load(data_dir, id)?;
    request.consumed = true;
    save(data_dir, &request)
}

fn path(data_dir: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(CliError::usage("invalid root request id"));
    }
    Ok(data_dir.join("approvals").join(format!("{id}.json")))
}

fn load(data_dir: &Path, id: &str) -> Result<RootRequest> {
    let path = path(data_dir, id)?;
    let data = std::fs::read(&path)
        .map_err(|error| CliError::io(format!("read root request: {error}")))?;
    serde_json::from_slice(&data)
        .map_err(|error| CliError::io(format!("parse root request: {error}")))
}

fn save(data_dir: &Path, request: &RootRequest) -> Result<()> {
    let path = path(data_dir, &request.id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| CliError::io(format!("create approvals: {error}")))?;
    }
    let data = serde_json::to_vec_pretty(request)
        .map_err(|error| CliError::io(format!("serialize root request: {error}")))?;
    std::fs::write(path, data).map_err(|error| CliError::io(format!("write root request: {error}")))
}

fn audit(data_dir: &Path, action: &str, id: &str, detail: Value) {
    #[cfg(not(unix))]
    {
        let _ = (data_dir, action, id, detail);
        return;
    }
    #[cfg(unix)]
    {
        let dir = data_dir.join("audit");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let path = dir.join("approvals.log");
        let line = serde_json::json!({
            "action": action,
            "id": id,
            "detail": detail,
            "ts": now_ms(),
        });
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
        {
            use std::io::Write;
            let _ = writeln!(file, "{line}");
        }
    }
}

fn host_authenticate() -> Result<()> {
    // MixumART performs the OS prompt first and sets this marker for the
    // single child process; do not expose it as a normal user-facing option.
    if std::env::var("MIXUM_HOST_AUTHENTICATED").ok().as_deref() == Some("1") {
        return Ok(());
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        return Err(CliError::usage(
            "root approval requires an interactive human terminal",
        ));
    }
    let status = std::process::Command::new("sudo")
        .arg("-v")
        .status()
        .map_err(|error| CliError::runtime(format!("host authentication unavailable: {error}")))?;
    if !status.success() {
        return Err(CliError::conflict("host authentication failed"));
    }
    Ok(())
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserRequest {
    pub id: String,
    pub selector: String,
    pub domains: Vec<String>,
    pub reason: String,
    pub created_at_ms: u128,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub consumed: bool,
}

pub fn browser_request(
    data_dir: &Path,
    selector: &str,
    domains: &[String],
    reason: &str,
    json: bool,
) -> Result<()> {
    if domains.is_empty() {
        return Err(CliError::usage("at least one domain is required"));
    }
    let id = format!("browser-{}", ulid::Ulid::new());
    let request = BrowserRequest {
        id: id.clone(),
        selector: selector.to_owned(),
        domains: domains.to_vec(),
        reason: reason.trim().to_owned(),
        created_at_ms: now_ms(),
        approved: false,
        consumed: false,
    };
    save_browser(data_dir, &request)?;
    audit(
        data_dir,
        "browser_request",
        &request.id,
        json!({"domains": request.domains, "selector": request.selector}),
    );
    if json {
        output::print_json(&json!({
            "id": id,
            "approveCommand": format!("mixum browser approve {id}"),
            "request": request,
        }));
    } else {
        println!("browser request {id}");
        println!("domains: {}", domains.join(", "));
        println!("approve as human: mixum browser approve {id}");
    }
    Ok(())
}

pub fn browser_mark_approved(data_dir: &Path, id: &str) -> Result<BrowserRequest> {
    let mut request = load_browser(data_dir, id)?;
    host_authenticate()?;
    request.approved = true;
    save_browser(data_dir, &request)?;
    audit(
        data_dir,
        "browser_approved",
        &request.id,
        json!({"domains": request.domains, "selector": request.selector}),
    );
    Ok(request)
}

pub fn browser_consume(data_dir: &Path, id: &str) -> Result<()> {
    let mut request = load_browser(data_dir, id)?;
    request.consumed = true;
    save_browser(data_dir, &request)
}

fn browser_path(data_dir: &Path, id: &str) -> Result<PathBuf> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(CliError::usage("invalid browser request id"));
    }
    Ok(data_dir.join("approvals").join(format!("{id}.json")))
}

fn load_browser(data_dir: &Path, id: &str) -> Result<BrowserRequest> {
    let path = browser_path(data_dir, id)?;
    let data = std::fs::read(&path)
        .map_err(|error| CliError::io(format!("read browser request: {error}")))?;
    serde_json::from_slice(&data)
        .map_err(|error| CliError::io(format!("parse browser request: {error}")))
}

fn save_browser(data_dir: &Path, request: &BrowserRequest) -> Result<()> {
    let path = browser_path(data_dir, &request.id)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| CliError::io(format!("create approvals: {error}")))?;
    }
    let data = serde_json::to_vec_pretty(request)
        .map_err(|error| CliError::io(format!("serialize browser request: {error}")))?;
    std::fs::write(path, data)
        .map_err(|error| CliError::io(format!("write browser request: {error}")))
}
