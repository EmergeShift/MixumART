//! Chrome bridge host-facing commands: install native host, export a lease,
//! and hand that lease to a container browser controller.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::map_ops;
use crate::output::{self, CliError, Result};

const EXTENSION_ID: &str = "ahimdgpdmhhbhkokcablggjjidkdlnhl";

pub fn install(json: bool) -> Result<()> {
    let host = browser_host_path()?;
    if !host.exists() {
        return Err(CliError::runtime(format!(
            "native host binary not found: {} (build/install mixum-browser-host first)",
            host.display()
        )));
    }
    let manifest_dir = native_manifest_dir()?;
    std::fs::create_dir_all(&manifest_dir)
        .map_err(|error| CliError::io(format!("create native host dir: {error}")))?;
    let manifest = manifest_dir.join("dev.mixum.browser.json");
    let body = json!({
        "name": "dev.mixum.browser",
        "description": "Mixum browser bridge native host",
        "path": host,
        "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXTENSION_ID}/")],
    });
    std::fs::write(&manifest, serde_json::to_vec_pretty(&body).unwrap())
        .map_err(|error| CliError::io(format!("write native host manifest: {error}")))?;
    let extension_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../extensions/mixum-browser")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("extensions/mixum-browser"));
    if json {
        output::print_json(&json!({
            "nativeHost": manifest,
            "extensionId": EXTENSION_ID,
            "extensionDir": extension_dir,
        }));
    } else {
        println!("native host: {}", manifest.display());
        println!("extension id: {EXTENSION_ID}");
        println!("load unpacked for development: {}", extension_dir.display());
        println!(
            "production install: publish this extension or deploy it via Chrome enterprise policy"
        );
    }
    Ok(())
}

pub fn export(domains: &[String], json: bool) -> Result<()> {
    if domains.is_empty() {
        return Err(CliError::usage(
            "at least one --domain is required; the user must approve those domains",
        ));
    }
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|error| {
        CliError::runtime(format!(
            "Chrome bridge is not running ({}): {error}; open Chrome and the Mixum Browser Bridge extension",
            path.display()
        ))
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(20))).ok();
    let request = json!({"type":"export","domains":domains});
    writeln!(stream, "{request}")
        .map_err(|error| CliError::runtime(format!("send bridge request: {error}")))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| CliError::runtime(format!("read bridge reply: {error}")))?;
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|error| CliError::runtime(format!("parse bridge reply: {error}")))?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(CliError::runtime(
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Chrome bridge rejected the request")
                .to_owned(),
        ));
    }
    let lease = value.get("lease").cloned().unwrap_or(Value::Null);
    if json {
        output::print_json(&lease);
    } else {
        let cookies = lease
            .get("cookies")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let tabs = lease
            .get("tabs")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        println!("{cookies} cookies, {tabs} tabs for {}", domains.join(", "));
    }
    Ok(())
}

pub async fn share(data_dir: &Path, selector: &str, domains: &[String], json: bool) -> Result<()> {
    if domains.is_empty() {
        return Err(CliError::usage("at least one --domain is required"));
    }
    let path = socket_path();
    let mut stream = UnixStream::connect(&path).map_err(|error| {
        CliError::runtime(format!(
            "Chrome bridge is not running ({}): {error}; run `mixum browser install` and open Chrome",
            path.display()
        ))
    })?;
    stream.set_read_timeout(Some(Duration::from_secs(20))).ok();
    let request = json!({"type":"export","domains":domains});
    writeln!(stream, "{request}")
        .map_err(|error| CliError::runtime(format!("send bridge request: {error}")))?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|error| CliError::runtime(format!("read bridge reply: {error}")))?;
    let value: Value = serde_json::from_str(line.trim())
        .map_err(|error| CliError::runtime(format!("parse bridge reply: {error}")))?;
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(CliError::runtime(
            value
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Chrome bridge rejected the request")
                .to_owned(),
        ));
    }
    let lease = value.get("lease").cloned().unwrap_or(Value::Null);
    let (_resolved, client) = map_ops::env_client(data_dir, selector).await?;
    let body = serde_json::to_vec(&lease)
        .map_err(|error| CliError::io(format!("serialize browser lease: {error}")))?;
    let remote = client
        .put_file("browser-lease.json", &body)
        .await
        .map_err(|error| CliError::map(format!("file.put browser lease: {error}")))?;
    let container_path = format!("/var/lib/mixum/work/{remote}");
    let registry = crate::env::registry();
    let resolved = crate::env::find_with(&registry, data_dir, selector).await?;
    let backend = registry
        .get(resolved.kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    let output_text = backend
        .exec_capture(
            &resolved.container.id,
            &["mixum-browser", "apply", &container_path],
        )
        .await
        .map_err(|error| {
            CliError::runtime(format!(
                "container browser controller failed: {error}; install Chromium and mixum-browser in the base first"
            ))
        })?;
    let _ = backend
        .exec_capture(&resolved.container.id, &["rm", "-f", &container_path])
        .await;
    if json {
        output::print_json(&json!({
            "environment": resolved.container.name,
            "domains": domains,
            "controller": output_text.trim(),
            "lease": {
                "cookies": lease.get("cookies").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
                "tabs": lease.get("tabs").and_then(Value::as_array).map(Vec::len).unwrap_or(0),
            }
        }));
    } else {
        println!("shared browser lease with {}", resolved.container.name);
        if !output_text.trim().is_empty() {
            println!("{}", output_text.trim());
        }
    }
    Ok(())
}

fn browser_host_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("MIXUM_BROWSER_HOST") {
        return Ok(PathBuf::from(path));
    }
    let current = std::env::current_exe()
        .map_err(|error| CliError::runtime(format!("current executable: {error}")))?;
    let sibling = current.with_file_name("mixum-browser-host");
    if sibling.exists() {
        return Ok(sibling);
    }
    Ok(PathBuf::from("/usr/local/bin/mixum-browser-host"))
}

fn socket_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MIXUM_BROWSER_SOCKET") {
        return PathBuf::from(path);
    }
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("dev.mixum.agentenv")
        .join("mixum-cli")
        .join("browser.sock")
}

fn native_manifest_dir() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home =
            dirs::home_dir().ok_or_else(|| CliError::runtime("cannot locate home directory"))?;
        return Ok(home
            .join("Library")
            .join("Application Support")
            .join("Google")
            .join("Chrome")
            .join("NativeMessagingHosts"));
    }
    #[cfg(target_os = "linux")]
    {
        let home =
            dirs::home_dir().ok_or_else(|| CliError::runtime("cannot locate home directory"))?;
        return Ok(home
            .join(".config")
            .join("google-chrome")
            .join("NativeMessagingHosts"));
    }
    #[allow(unreachable_code)]
    Err(CliError::usage(
        "browser bridge install is currently implemented for macOS and Linux",
    ))
}
