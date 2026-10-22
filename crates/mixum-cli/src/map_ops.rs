use std::path::Path;
use std::time::{Duration, Instant};

use mixum_env_core::{BackendRegistry, RuntimeKind};
use mixum_env_map::MapClient;
use serde_json::Value;

use crate::env;
use crate::output::{self, CliError, Result};

pub struct MapReady {
    pub fingerprint: String,
    pub windows: usize,
}

pub async fn connect(
    registry: &BackendRegistry,
    kind: RuntimeKind,
    container_id: &str,
    port: u16,
    fingerprint: Option<&str>,
) -> Result<MapClient> {
    if port == 0 {
        return Err(CliError::map(
            "environment has no published MAP port; recreate it with a MAP port",
        ));
    }
    let backend = registry
        .get(kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    let token = backend
        .admin_token(container_id)
        .await
        .map_err(|error| CliError::map(format!("cannot read agentd token: {error}")))?;
    MapClient::connect(&format!("127.0.0.1:{port}"), &token, fingerprint)
        .await
        .map_err(|error| CliError::map(format!("MAP handshake failed: {error}")))
}

pub async fn wait_ready(
    registry: &BackendRegistry,
    _data_dir: &Path,
    kind: RuntimeKind,
    container_id: &str,
    name: &str,
    port: u16,
    timeout: u64,
) -> Result<MapReady> {
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let last_error = match connect(registry, kind, container_id, port, None).await {
            Ok(client) => match client.list().await {
                Ok(windows) => {
                    return Ok(MapReady {
                        fingerprint: client.fingerprint,
                        windows: windows.len(),
                    })
                }
                Err(error) => error.to_string(),
            },
            Err(error) => error.message,
        };
        if Instant::now() >= deadline {
            return Err(CliError::map(format!(
                "MAP for {name} was not ready within {timeout}s: {last_error}"
            )));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn env_client(data_dir: &Path, selector: &str) -> Result<(env::ResolvedEnv, MapClient)> {
    let registry = env::registry();
    let resolved = env::find_with(&registry, data_dir, selector).await?;
    if resolved.container.state != mixum_env_core::ContainerState::Running {
        return Err(CliError::conflict(format!(
            "environment {} is {}; start it first",
            resolved.container.name,
            env::state_label(resolved.container.state)
        )));
    }
    let port = resolved
        .container
        .ports
        .iter()
        .find(|mapping| mapping.container == crate::ports::MAP_PORT)
        .map(|mapping| mapping.host)
        .ok_or_else(|| CliError::map("environment has no published MAP port"))?;
    let client = connect(
        &registry,
        resolved.kind,
        &resolved.container.id,
        port,
        resolved
            .record
            .as_ref()
            .and_then(|record| record.fingerprint.as_deref()),
    )
    .await?;
    Ok((resolved, client))
}

pub async fn windows(data_dir: &Path, selector: &str, timeout: u64, json: bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(timeout.max(1));
    let (resolved, client) = loop {
        match env_client(data_dir, selector).await {
            Ok(value) => break value,
            Err(error) if Instant::now() < deadline && error.code == 6 => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(error) => return Err(error),
        }
    };
    let surfaces = client
        .list()
        .await
        .map_err(|error| CliError::map(format!("display.list failed: {error}")))?;
    if json {
        output::print_json(&serde_json::json!({
            "environment": resolved.container.name,
            "fingerprint": client.fingerprint,
            "windows": surfaces,
        }));
    } else if surfaces.is_empty() {
        println!("{}: no windows", resolved.container.name);
    } else {
        let rows = surfaces
            .iter()
            .map(|surface| {
                vec![
                    surface.id.to_string(),
                    surface.title.clone(),
                    surface.app_id.clone(),
                    format!("{}x{}", surface.width, surface.height),
                    if surface.id >= 1000 {
                        "enhanced"
                    } else {
                        "compat"
                    }
                    .to_string(),
                ]
            })
            .collect::<Vec<_>>();
        output::table(&["ID", "TITLE", "APP", "SIZE", "MODE"], &rows);
    }
    Ok(())
}

pub async fn snapshot_list(data_dir: &Path, selector: &str, json: bool) -> Result<()> {
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let tree = client
        .snapshot_tree()
        .await
        .map_err(|error| CliError::map(format!("snap.tree failed: {error}")))?;
    if json {
        output::print_json(&tree);
    } else {
        print_snapshot_tree(&tree);
    }
    Ok(())
}

pub async fn snapshot_create(
    data_dir: &Path,
    selector: &str,
    label: Option<&str>,
    json: bool,
) -> Result<()> {
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let result = client
        .snapshot_create(label)
        .await
        .map_err(|error| CliError::map(format!("snap.create failed: {error}")))?;
    if json {
        output::print_json(&result);
    } else {
        println!(
            "snapshot created: {}",
            result.get("id").and_then(Value::as_str).unwrap_or("?")
        );
    }
    Ok(())
}

pub async fn snapshot_restore(
    data_dir: &Path,
    selector: &str,
    node: &str,
    yes: bool,
    json: bool,
) -> Result<()> {
    if !yes {
        return Err(CliError::usage(
            "snapshot restore is destructive; pass --yes after verifying the environment",
        ));
    }
    let (_resolved, client) = env_client(data_dir, selector).await?;
    let result = client
        .snapshot_restore(node)
        .await
        .map_err(|error| CliError::map(format!("snap.restore failed: {error}")))?;
    if json {
        output::print_json(&result);
    } else {
        println!("restored snapshot {node}");
    }
    Ok(())
}

fn print_snapshot_tree(value: &Value) {
    let nodes = value
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if nodes.is_empty() {
        println!("no snapshots");
        return;
    }
    let rows = nodes
        .iter()
        .map(|node| {
            vec![
                node.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                node.get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                node.get("branch")
                    .and_then(Value::as_str)
                    .unwrap_or("?")
                    .to_string(),
                node.get("label")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            ]
        })
        .collect::<Vec<_>>();
    output::table(&["ID", "KIND", "BRANCH", "LABEL"], &rows);
}
