//! Chrome bridge host-facing commands: install native host, export a lease,
//! and hand that lease to a container browser controller.

#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
#[cfg(unix)]
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
    let extension_dir = extension_dir()?;
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

#[cfg(unix)]
pub fn export(domains: &[String], urls: &[String], json: bool) -> Result<()> {
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
    let request = json!({"type":"export","domains":domains,"urls":urls});
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
        if !urls.is_empty() {
            println!("urls: {}", urls.join(", "));
        }
    }
    Ok(())
}

#[cfg(unix)]
pub async fn share(
    data_dir: &Path,
    selector: &str,
    domains: &[String],
    urls: &[String],
    json: bool,
) -> Result<()> {
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
    let request = json!({"type":"export","domains":domains,"urls":urls});
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
    let container_path = if remote.starts_with('/') {
        remote.clone()
    } else {
        format!("/var/lib/mixum/work/{remote}")
    };
    let registry = crate::env::registry();
    let resolved = crate::env::find_with(&registry, data_dir, selector).await?;
    let backend = registry
        .get(resolved.kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    let controller = backend
        .exec_capture(
            &resolved.container.id,
            &["mixum-browser", "apply", &container_path],
        )
        .await;
    // Never leave a cookie lease behind, even when the controller fails.
    let _ = backend
        .exec_capture(&resolved.container.id, &["rm", "-f", &container_path])
        .await;
    let output_text = controller.map_err(|error| {
        CliError::runtime(format!(
            "container browser controller failed: {error}; install Chromium and mixum-browser in the base first"
        ))
    })?;
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

#[cfg(not(unix))]
pub fn export(_domains: &[String], _urls: &[String], _json: bool) -> Result<()> {
    Err(CliError::usage(
        "the Chrome bridge is currently available on macOS and Linux only",
    ))
}

#[cfg(not(unix))]
pub async fn share(
    _data_dir: &Path,
    _selector: &str,
    _domains: &[String],
    _urls: &[String],
    _json: bool,
) -> Result<()> {
    Err(CliError::usage(
        "the Chrome bridge is currently available on macOS and Linux only",
    ))
}

fn extension_dir() -> Result<PathBuf> {
    let candidates = [
        std::env::var_os("MIXUM_EXTENSION_DIR").map(PathBuf::from),
        std::env::current_exe().ok().map(|exe| {
            exe.parent()
                .unwrap_or_else(|| Path::new("."))
                .join("../share/mixum/extensions/mixum-browser")
        }),
        // Portable CLI bundle: bin/mixum + share/mixum/extensions/... or
        // bin/mixum + extensions/... (release tarball layout).
        std::env::current_exe().ok().map(|exe| {
            exe.parent()
                .unwrap_or_else(|| Path::new("."))
                .join("extensions/mixum-browser")
        }),
        std::env::current_exe().ok().map(|exe| {
            exe.parent()
                .unwrap_or_else(|| Path::new("."))
                .join("../extensions/mixum-browser")
        }),
        std::env::current_exe().ok().map(|exe| {
            exe.parent()
                .unwrap_or_else(|| Path::new("."))
                .join("../Resources/extensions/mixum-browser")
        }),
        Some(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/mixum-browser")),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.join("manifest.json").is_file() {
            return candidate
                .canonicalize()
                .map_err(|error| CliError::io(format!("canonicalize extension: {error}")));
        }
    }
    Err(CliError::usage(
        "extension sources not found; set MIXUM_EXTENSION_DIR or install them under share/mixum/extensions",
    ))
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
    if let Some(dir) = std::env::var_os("MIXUM_CHROME_NATIVE_HOST_DIR") {
        return Ok(PathBuf::from(dir));
    }
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

/// Build the Web Store upload artifact. Chrome's `--load-extension` path is
/// dev-only; store review consumes this zip.
pub fn package(out: Option<&str>, json: bool) -> Result<()> {
    let extension = extension_dir()?;
    if !extension.join("manifest.json").is_file() {
        return Err(CliError::usage(format!(
            "extension sources not found at {}",
            extension.display()
        )));
    }
    let out = match out {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from("mixum-browser-extension.zip"),
    };
    // zip runs with current_dir(extension), so resolve the output path against
    // the CLI's working directory before invoking it.
    let out = if out.is_absolute() {
        out
    } else {
        std::env::current_dir()
            .map_err(|error| CliError::io(format!("cwd: {error}")))?
            .join(out)
    };
    if out.exists() {
        return Err(CliError::conflict(format!(
            "refusing to overwrite {}",
            out.display()
        )));
    }
    let output = std::process::Command::new("zip")
        .arg("-q")
        .arg("-r")
        .arg(&out)
        .arg(".")
        .arg("-x")
        .arg("*.tpl")
        .arg("-x")
        .arg("*.zip")
        .current_dir(&extension)
        .output()
        .map_err(|error| CliError::runtime(format!("zip: {error}")))?;
    if !output.status.success() {
        return Err(CliError::runtime(format!(
            "zip failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let bytes = std::fs::metadata(&out)
        .map_err(|error| CliError::io(format!("stat zip: {error}")))?
        .len();
    if json {
        output::print_json(&serde_json::json!({
            "extension": extension,
            "zip": out,
            "bytes": bytes,
            "store": "upload this zip at https://chrome.google.com/webstore/devconsole",
        }));
    } else {
        println!("packaged {}", out.display());
        println!("bytes: {bytes}");
        println!("upload at https://chrome.google.com/webstore/devconsole");
    }
    Ok(())
}

/// Enterprise policy fragment. Consumer installs still require the Web Store
/// listing; this is the no-manual path for managed Chrome.
pub fn policy(out: Option<&str>, format: crate::cli::PolicyFormat, json: bool) -> Result<()> {
    let text = match format {
        crate::cli::PolicyFormat::Json => {
            let body = serde_json::json!({
                "ExtensionSettings": {
                    EXTENSION_ID: {
                        "installation_mode": "force_installed",
                        "update_url": "https://clients2.google.com/service/update2/crx"
                    }
                }
            });
            serde_json::to_string_pretty(&body)
                .map_err(|error| CliError::io(format!("serialize policy: {error}")))?
        }
        crate::cli::PolicyFormat::Plist => format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>ExtensionSettings</key>
  <dict>
    <key>{EXTENSION_ID}</key>
    <dict>
      <key>installation_mode</key>
      <string>force_installed</string>
      <key>update_url</key>
      <string>https://clients2.google.com/service/update2/crx</string>
    </dict>
  </dict>
</dict>
</plist>
"#
        ),
    };
    match out {
        Some(path) => {
            std::fs::write(path, &text)
                .map_err(|error| CliError::io(format!("write policy: {error}")))?;
            if json {
                output::print_json(&serde_json::json!({
                    "path": path,
                    "extensionId": EXTENSION_ID,
                    "format": "chrome-policy-json",
                }));
            } else {
                println!("wrote {path}");
                println!("extension id: {EXTENSION_ID}");
            }
        }
        None => println!("{text}"),
    }
    Ok(())
}

pub fn status(json: bool) -> Result<()> {
    let manifest = native_manifest_dir()
        .ok()
        .map(|dir| dir.join("dev.mixum.browser.json"));
    let host = browser_host_path().unwrap_or_else(|_| PathBuf::from("mixum-browser-host"));
    let socket = socket_path();
    let manifest_ok = manifest.as_ref().is_some_and(|path| path.is_file());
    let host_ok = host.is_file();
    let socket_ok = socket.exists();
    let value = serde_json::json!({
        "extensionId": EXTENSION_ID,
        "nativeHostManifest": manifest,
        "nativeHostManifestPresent": manifest_ok,
        "nativeHostBinary": host,
        "nativeHostBinaryPresent": host_ok,
        "socket": socket,
        "socketPresent": socket_ok,
        "nextStep": if !manifest_ok {
            "run `mixum browser install`"
        } else if !host_ok {
            "install mixum-browser-host next to mixum or set MIXUM_BROWSER_HOST"
        } else if !socket_ok {
            "open Chrome and the Mixum Browser Bridge extension"
        } else {
            "ready"
        },
    });
    if json {
        output::print_json(&value);
    } else {
        println!("extension id: {EXTENSION_ID}");
        println!(
            "native host manifest: {} ({})",
            manifest
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "unresolved".into()),
            if manifest_ok { "present" } else { "missing" }
        );
        println!(
            "native host binary: {} ({})",
            host.display(),
            if host_ok { "present" } else { "missing" }
        );
        println!(
            "socket: {} ({})",
            socket.display(),
            if socket_ok { "present" } else { "missing" }
        );
        println!("next: {}", value["nextStep"].as_str().unwrap_or(""));
    }
    Ok(())
}
