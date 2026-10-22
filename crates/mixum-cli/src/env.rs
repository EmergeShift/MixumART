use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use mixum_env_core::{
    BackendRegistry, BackendStatus, Container, ContainerState, CreateSpec, PortMapping,
    RuntimeBackend, RuntimeKind, RuntimeStatus,
};
use serde::Serialize;
use ulid::Ulid;

use crate::cli::{CreateArgs, Profile};
use crate::map_ops;
use crate::output::{self, CliError, Result};
use crate::ports;
use crate::state::{EnvRecord, State, StateLock, StateStore};

#[derive(Debug, Clone)]
pub struct ResolvedEnv {
    pub kind: RuntimeKind,
    pub container: Container,
    pub record: Option<EnvRecord>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvView {
    pub id: String,
    pub name: String,
    pub title: String,
    pub runtime: RuntimeKind,
    pub image: String,
    pub state: String,
    pub method: Option<String>,
    pub managed: bool,
    pub map_port: Option<u16>,
    pub map_addr: Option<String>,
    pub fingerprint: Option<String>,
    pub created_at_ms: Option<u128>,
}

pub fn registry() -> BackendRegistry {
    let mut registry = BackendRegistry::new();
    registry.register(mixum_env_runtime_docker::backend());
    registry.register(mixum_env_runtime_wsl2::backend());
    registry.register(mixum_env_runtime_apple::backend());
    registry
}

fn platform_default(ready: &[BackendStatus]) -> Option<RuntimeKind> {
    let preferred = if cfg!(target_os = "macos") {
        [RuntimeKind::AppleContainer, RuntimeKind::Docker]
    } else if cfg!(target_os = "windows") {
        [RuntimeKind::Wsl2, RuntimeKind::Docker]
    } else {
        [RuntimeKind::Docker, RuntimeKind::Wsl2]
    };
    preferred
        .iter()
        .find(|kind| ready.iter().any(|backend| backend.kind == **kind))
        .copied()
}

async fn select_backend(
    registry: &BackendRegistry,
    requested: Option<RuntimeKind>,
) -> Result<(RuntimeKind, Arc<dyn RuntimeBackend>)> {
    let statuses = registry.statuses().await;
    let ready = statuses
        .iter()
        .filter(|backend| backend.status.is_ready())
        .cloned()
        .collect::<Vec<_>>();
    let kind = if let Some(kind) = requested {
        let status = statuses.iter().find(|backend| backend.kind == kind);
        match status {
            Some(backend) if backend.status.is_ready() => kind,
            Some(backend) => {
                return Err(CliError::runtime(format!(
                    "{} is not ready: {}",
                    backend.name,
                    status_hint(&backend.status)
                )))
            }
            None => {
                return Err(CliError::runtime(format!(
                    "runtime {} is not registered on this platform",
                    kind.id()
                )))
            }
        }
    } else if ready.len() == 1 {
        ready[0].kind
    } else if ready.is_empty() {
        return Err(CliError::runtime(
            "no container runtime is ready; run `mixum doctor`",
        ));
    } else {
        platform_default(&ready).ok_or_else(|| {
            CliError::runtime("multiple runtimes are ready; pass --runtime explicitly")
        })?
    };
    let backend = registry
        .get(kind)
        .ok_or_else(|| CliError::runtime(format!("runtime {} is not registered", kind.id())))?;
    Ok((kind, backend))
}

pub fn status_hint(status: &RuntimeStatus) -> String {
    match status {
        RuntimeStatus::Ready { version, detail } => {
            let mut text = "ready".to_string();
            if let Some(version) = version {
                text.push_str(&format!(" ({version})"));
            }
            if let Some(detail) = detail {
                text.push_str(&format!(" — {detail}"));
            }
            text
        }
        RuntimeStatus::NotInstalled { hint }
        | RuntimeStatus::NotRunning { hint }
        | RuntimeStatus::Unsupported { reason: hint } => hint.clone(),
    }
}

pub fn state_label(state: ContainerState) -> &'static str {
    match state {
        ContainerState::Created => "created",
        ContainerState::Running => "running",
        ContainerState::Paused => "paused",
        ContainerState::Stopped => "stopped",
        ContainerState::Unknown => "unknown",
    }
}

fn map_port(container: &Container) -> Option<u16> {
    container
        .ports
        .iter()
        .find(|mapping| mapping.container == ports::MAP_PORT)
        .map(|mapping| mapping.host)
}

pub fn view(kind: RuntimeKind, container: &Container, record: Option<&EnvRecord>) -> EnvView {
    let map_port = map_port(container);
    EnvView {
        id: container.id.clone(),
        name: container.name.clone(),
        title: record
            .map(|record| record.title.clone())
            .unwrap_or_else(|| container.name.clone()),
        runtime: kind,
        image: container.image.clone(),
        state: state_label(container.state).to_string(),
        method: record.map(|record| record.method.clone()),
        managed: record.is_some(),
        map_port,
        map_addr: map_port.map(|port| format!("127.0.0.1:{port}")),
        fingerprint: record.and_then(|record| record.fingerprint.clone()),
        created_at_ms: record.map(|record| record.created_at_ms),
    }
}

fn record_view(record: &EnvRecord) -> EnvView {
    EnvView {
        id: record.id.clone(),
        name: record.name.clone(),
        title: record.title.clone(),
        runtime: parse_kind(&record.runtime),
        image: record.image.clone(),
        state: "missing".to_string(),
        method: Some(record.method.clone()),
        managed: true,
        map_port: Some(record.port),
        map_addr: Some(format!("127.0.0.1:{}", record.port)),
        fingerprint: record.fingerprint.clone(),
        created_at_ms: Some(record.created_at_ms),
    }
}

fn parse_kind(value: &str) -> RuntimeKind {
    match value {
        "wsl2" => RuntimeKind::Wsl2,
        "apple_container" => RuntimeKind::AppleContainer,
        _ => RuntimeKind::Docker,
    }
}

async fn all_containers(registry: &BackendRegistry) -> Result<Vec<(RuntimeKind, Container)>> {
    let mut result = Vec::new();
    let mut failures = Vec::new();
    for kind in RuntimeKind::ALL {
        let Some(backend) = registry.get(kind) else {
            continue;
        };
        if !backend.probe().await.is_ready() {
            continue;
        }
        match backend.list_containers().await {
            Ok(containers) => {
                result.extend(containers.into_iter().map(|container| (kind, container)));
            }
            Err(error) => failures.push(format!("{}: {error}", kind.display_name())),
        }
    }
    if !failures.is_empty() {
        return Err(CliError::runtime(format!(
            "cannot list all runtimes: {}",
            failures.join("; ")
        )));
    }
    Ok(result)
}

fn resolve_in(
    entries: &[(RuntimeKind, Container)],
    state: &State,
    selector: &str,
) -> Result<ResolvedEnv> {
    let mut matches = Vec::new();
    for (kind, container) in entries {
        let record = state.find(&container.id);
        let title_matches = record.is_some_and(|record| record.title == selector);
        let exact = container.id == selector || container.name == selector || title_matches;
        let prefix = selector.len() >= 4
            && (container.id.starts_with(selector) || container.name.starts_with(selector));
        if exact || prefix {
            matches.push(ResolvedEnv {
                kind: *kind,
                container: container.clone(),
                record: record.cloned(),
            });
        }
    }
    if matches.is_empty() {
        return Err(CliError::not_found(format!(
            "environment {selector:?} was not found; run `mixum env list`"
        )));
    }
    if matches.len() > 1 {
        let names = matches
            .iter()
            .map(|item| item.container.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(CliError::conflict(format!(
            "selector {selector:?} is ambiguous: {names}"
        )));
    }
    Ok(matches.remove(0))
}

pub async fn find_with(
    registry: &BackendRegistry,
    data_dir: &Path,
    selector: &str,
) -> Result<ResolvedEnv> {
    let entries = all_containers(registry).await?;
    let state = StateStore::new(data_dir).load()?;
    resolve_in(&entries, &state, selector)
}

pub async fn runtime_list(json: bool) -> Result<()> {
    let registry = registry();
    let statuses = registry.statuses().await;
    if json {
        output::print_json(&statuses);
    } else {
        let rows = statuses
            .iter()
            .map(|backend| {
                vec![
                    backend.kind.id().to_string(),
                    backend.name.clone(),
                    status_hint(&backend.status),
                ]
            })
            .collect::<Vec<_>>();
        output::table(&["ID", "RUNTIME", "STATUS"], &rows);
    }
    Ok(())
}

pub async fn list(data_dir: &Path, json: bool) -> Result<()> {
    let registry = registry();
    let entries = all_containers(&registry).await?;
    let state = StateStore::new(data_dir).load()?;
    let mut views = entries
        .iter()
        .map(|(kind, container)| view(*kind, container, state.find(&container.id)))
        .collect::<Vec<_>>();
    for record in &state.envs {
        if !entries
            .iter()
            .any(|(_, container)| container.id == record.id)
        {
            views.push(record_view(record));
        }
    }
    views.sort_by(|left, right| left.name.cmp(&right.name));
    if json {
        output::print_json(&views);
        return Ok(());
    }
    let rows = views
        .iter()
        .map(|view| {
            vec![
                view.name.clone(),
                view.runtime.id().to_string(),
                view.state.clone(),
                view.title.clone(),
                view.method.clone().unwrap_or_else(|| "-".to_string()),
                view.map_addr.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect::<Vec<_>>();
    output::table(
        &["NAME", "RUNTIME", "STATE", "TITLE", "METHOD", "MAP"],
        &rows,
    );
    Ok(())
}

fn slug(value: &str) -> String {
    let mut result = String::new();
    let mut last_dash = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !result.is_empty() {
            result.push('-');
            last_dash = true;
        }
    }
    let trimmed = result.trim_matches('-');
    if trimmed.is_empty() {
        "env".to_string()
    } else {
        trimmed.chars().take(48).collect()
    }
}

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn image_candidates(images: &[String]) -> Vec<String> {
    let mut candidates = images
        .iter()
        .filter(|image| {
            let lower = image.to_ascii_lowercase();
            lower.contains("mixumos") && !lower.contains("<none>")
        })
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.dedup();
    candidates
}

async fn choose_image(
    backend: &Arc<dyn RuntimeBackend>,
    profile: Profile,
    image: Option<&str>,
) -> Result<String> {
    let images = backend
        .images()
        .await
        .map_err(|error| CliError::runtime(format!("cannot list images: {error}")))?;
    if let Some(image) = image {
        if profile == Profile::Mixumos && !image.to_ascii_lowercase().contains("mixumos") {
            return Err(CliError::usage(
                "profile mixumos requires an image whose reference contains 'mixumos'; use --profile generic for other images",
            ));
        }
        if !images.iter().any(|item| item == image) {
            return Err(CliError::not_found(format!(
                "image {image:?} is not present locally; pull it first or choose one of: {}",
                images.join(", ")
            )));
        }
        return Ok(image.to_string());
    }
    if profile == Profile::Generic {
        return Err(CliError::usage(
            "--image is required with --profile generic",
        ));
    }
    let mut candidates = image_candidates(&images);
    match candidates.len() {
        0 => Err(CliError::not_found(
            "no local image contains 'mixumos'; pull one or pass --image",
        )),
        1 => Ok(candidates.remove(0)),
        _ => Err(CliError::usage(format!(
            "multiple mixumos images are present; pass --image: {}",
            candidates.join(", ")
        ))),
    }
}

pub async fn create(data_dir: &Path, args: CreateArgs, json: bool) -> Result<()> {
    let title = args.title.trim();
    if title.is_empty() {
        return Err(CliError::usage("--title cannot be empty"));
    }
    if title.chars().count() > 120 {
        return Err(CliError::usage("--title is limited to 120 characters"));
    }
    let _lock = StateLock::acquire(data_dir)?;
    let store = StateStore::new(data_dir);
    let state = store.load()?;
    let registry = registry();
    let (kind, backend) =
        select_backend(&registry, args.runtime.map(|runtime| runtime.kind())).await?;
    let image = choose_image(&backend, args.profile, args.image.as_deref()).await?;
    let containers = backend.list_containers().await.map_err(|error| {
        CliError::runtime(format!("cannot list {} containers: {error}", kind.id()))
    })?;
    let name = match args.name {
        Some(name) => {
            if !valid_name(&name) {
                return Err(CliError::usage(
                    "--name must be 1-63 lowercase letters, digits or hyphens and start with a letter",
                ));
            }
            if containers.iter().any(|container| container.name == name)
                || state.envs.iter().any(|record| record.name == name)
            {
                return Err(CliError::conflict(format!(
                    "container name {name:?} already exists"
                )));
            }
            name
        }
        None => {
            let slug = slug(title);
            let suffix = Ulid::new().to_string().to_ascii_lowercase();
            format!("mixum-{slug}-{}", &suffix[..6])
        }
    };
    let all_used = ports::used_host_ports(
        &all_containers(&registry)
            .await?
            .iter()
            .map(|(_, container)| container.ports.clone())
            .collect::<Vec<_>>(),
    );
    let port = ports::allocate(args.port, &all_used)?;
    let mut env = BTreeMap::new();
    env.insert("MIXUM_METHOD".to_string(), args.method.as_str().to_string());
    env.insert("MIXUM_TITLE".to_string(), title.to_string());
    let spec = CreateSpec {
        name: name.clone(),
        image: image.clone(),
        ports: vec![PortMapping::tcp(port, ports::MAP_PORT)],
        mounts: Vec::new(),
        env,
        command: Vec::new(),
    };
    let started = !args.no_start;
    let mut container = backend
        .create_container(spec)
        .await
        .map_err(|error| CliError::runtime(format!("create failed: {error}")))?;
    let mut record = EnvRecord {
        id: container.id.clone(),
        name: name.clone(),
        title: title.to_string(),
        method: args.method.as_str().to_string(),
        runtime: kind.id().to_string(),
        image: image.clone(),
        port,
        created_at_ms: crate::state::now_ms(),
        fingerprint: None,
    };
    let mut state = store.load()?;
    state.upsert(record.clone());
    store.save(&state)?;
    if !started {
        let view = view(kind, &container, Some(&record));
        if json {
            output::print_json(&serde_json::json!({ "environment": view, "mapReady": false }));
        } else {
            println!(
                "created {} ({}) image={image} port={port}",
                name, container.id
            );
            println!("not started; run `mixum env start {}`", name);
        }
        return Ok(());
    }
    backend
        .start_container(&container.id)
        .await
        .map_err(|error| {
            CliError::runtime(format!(
                "environment {} was created but start failed: {error}",
                container.id
            ))
        })?;
    if let Ok(current) = backend.list_containers().await {
        if let Some(current) = current.into_iter().find(|item| item.id == container.id) {
            container = current;
        }
    }
    let wait_map = if args.profile == Profile::Generic {
        0
    } else {
        args.wait_map
    };
    if wait_map > 0 {
        match map_ops::wait_ready(
            &registry,
            data_dir,
            kind,
            &container.id,
            &name,
            port,
            wait_map,
        )
        .await
        {
            Ok(map) => {
                record.fingerprint = Some(map.fingerprint.clone());
                let mut state = store.load()?;
                state.upsert(record.clone());
                store.save(&state)?;
                let view = view(kind, &container, Some(&record));
                if json {
                    output::print_json(&serde_json::json!({
                        "environment": view,
                        "mapReady": true,
                        "fingerprint": map.fingerprint,
                        "windows": map.windows,
                    }));
                } else {
                    println!("created {} ({})", name, container.id);
                    println!("runtime: {}  image: {}", kind.id(), image);
                    println!("map: 127.0.0.1:{port}  tls: {}", map.fingerprint);
                    println!("windows: {}", map.windows);
                }
                return Ok(());
            }
            Err(error) => {
                return Err(CliError::map(format!(
                    "environment {name} ({}) was created, but MAP did not become ready: {}",
                    container.id, error.message
                )))
            }
        }
    }
    let view = view(kind, &container, Some(&record));
    if json {
        output::print_json(&serde_json::json!({ "environment": view, "mapReady": false }));
    } else {
        println!("created {} ({}) port={port}", name, container.id);
        println!("MAP wait disabled; run `mixum env windows {name}` once agentd is ready");
    }
    Ok(())
}

pub async fn show(data_dir: &Path, selector: &str, json: bool) -> Result<()> {
    let registry = registry();
    let resolved = find_with(&registry, data_dir, selector).await?;
    let state = StateStore::new(data_dir).load()?;
    let record = resolved
        .record
        .clone()
        .or_else(|| state.find(&resolved.container.id).cloned());
    let mut view = view(resolved.kind, &resolved.container, record.as_ref());
    let mut map_error = None;
    if resolved.container.state == ContainerState::Running {
        match map_ops::connect(
            &registry,
            resolved.kind,
            &resolved.container.id,
            view.map_port.unwrap_or(0),
            record
                .as_ref()
                .and_then(|record| record.fingerprint.as_deref()),
        )
        .await
        {
            Ok(client) => {
                view.fingerprint = Some(client.fingerprint.clone());
                match client.list().await {
                    Ok(surfaces) => {
                        if json {
                            output::print_json(&serde_json::json!({
                                "environment": view,
                                "windows": surfaces,
                            }));
                        } else {
                            println!("{}  {}", view.name, view.id);
                            println!("title:   {}", view.title);
                            println!("runtime: {}  image: {}", view.runtime.id(), view.image);
                            println!(
                                "state:   {}  map: {}",
                                view.state,
                                view.map_addr.clone().unwrap_or_default()
                            );
                            println!("tls:     {}", client.fingerprint);
                            println!("windows: {}", surfaces.len());
                            for surface in surfaces {
                                println!(
                                    "  {}  {}  {}x{}",
                                    surface.id, surface.title, surface.width, surface.height
                                );
                            }
                        }
                        return Ok(());
                    }
                    Err(error) => map_error = Some(error.to_string()),
                }
            }
            Err(error) => map_error = Some(error.to_string()),
        }
    }
    if json {
        output::print_json(&serde_json::json!({ "environment": view, "mapError": map_error }));
    } else {
        println!("{}  {}", view.name, view.id);
        println!("title:   {}", view.title);
        println!("runtime: {}  image: {}", view.runtime.id(), view.image);
        println!(
            "state:   {}  map: {}",
            view.state,
            view.map_addr.clone().unwrap_or_default()
        );
        if let Some(error) = map_error {
            println!("map:     {error}");
        }
    }
    Ok(())
}

pub enum StateChange {
    Start,
    Stop,
}

pub async fn change_state(
    data_dir: &Path,
    selector: &str,
    change: StateChange,
    json: bool,
) -> Result<()> {
    let registry = registry();
    let resolved = find_with(&registry, data_dir, selector).await?;
    let backend = registry
        .get(resolved.kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    match change {
        StateChange::Start => {
            backend
                .start_container(&resolved.container.id)
                .await
                .map_err(|error| CliError::runtime(format!("start failed: {error}")))?;
        }
        StateChange::Stop => {
            backend
                .stop_container(&resolved.container.id)
                .await
                .map_err(|error| CliError::runtime(format!("stop failed: {error}")))?;
        }
    }
    if json {
        output::print_json(&serde_json::json!({
            "id": resolved.container.id,
            "name": resolved.container.name,
            "state": match change { StateChange::Start => "running", StateChange::Stop => "stopped" },
        }));
    } else {
        println!(
            "{} {}",
            match change {
                StateChange::Start => "started",
                StateChange::Stop => "stopped",
            },
            resolved.container.name
        );
    }
    Ok(())
}

pub async fn remove(data_dir: &Path, selector: &str, force: bool, json: bool) -> Result<()> {
    let registry = registry();
    let resolved = find_with(&registry, data_dir, selector).await?;
    if resolved.container.state == ContainerState::Running && !force {
        return Err(CliError::conflict(
            "environment is running; stop it first or pass --force",
        ));
    }
    let backend = registry
        .get(resolved.kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    if resolved.container.state == ContainerState::Running {
        backend
            .stop_container(&resolved.container.id)
            .await
            .map_err(|error| CliError::runtime(format!("stop before remove failed: {error}")))?;
    }
    backend
        .remove_container(&resolved.container.id)
        .await
        .map_err(|error| CliError::runtime(format!("remove failed: {error}")))?;
    let _lock = StateLock::acquire(data_dir)?;
    let store = StateStore::new(data_dir);
    let mut state = store.load()?;
    state.remove(&resolved.container.id);
    store.save(&state)?;
    if json {
        output::print_json(&serde_json::json!({
            "removed": resolved.container.id,
            "name": resolved.container.name,
        }));
    } else {
        println!("removed {}", resolved.container.name);
    }
    Ok(())
}

fn redact_secrets(line: String) -> String {
    let mut output = String::with_capacity(line.len());
    let mut rest = line.as_str();
    while let Some(index) = rest.find("map1.") {
        output.push_str(&rest[..index]);
        output.push_str("map1.<redacted>");
        let token = &rest[index..];
        let end = token
            .find(|character: char| {
                character.is_whitespace() || character == '"' || character == '\''
            })
            .unwrap_or(token.len());
        rest = &token[end..];
    }
    output.push_str(rest);
    output
}

pub async fn logs(data_dir: &Path, selector: &str, tail: usize, json: bool) -> Result<()> {
    let registry = registry();
    let resolved = find_with(&registry, data_dir, selector).await?;
    let backend = registry
        .get(resolved.kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    let lines = backend
        .logs(&resolved.container.id, tail.clamp(1, 2000))
        .await
        .map_err(|error| CliError::runtime(format!("logs failed: {error}")))?;
    let lines = lines.into_iter().map(redact_secrets).collect::<Vec<_>>();
    if json {
        output::print_json(&lines);
    } else {
        for line in lines {
            println!("{line}");
        }
    }
    Ok(())
}

pub async fn port_list(json: bool) -> Result<()> {
    let registry = registry();
    let entries = all_containers(&registry).await?;
    let mut rows = entries
        .iter()
        .flat_map(|(kind, container)| {
            container.ports.iter().map(move |mapping| {
                (
                    mapping.host,
                    mapping.container,
                    container.name.clone(),
                    kind.id().to_string(),
                )
            })
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.0);
    if json {
        let values = rows
            .iter()
            .map(|(host, container, name, runtime)| {
                serde_json::json!({
                    "hostPort": host,
                    "containerPort": container,
                    "environment": name,
                    "runtime": runtime,
                })
            })
            .collect::<Vec<_>>();
        output::print_json(&values);
    } else {
        let rows = rows
            .into_iter()
            .map(|(host, container, name, runtime)| {
                vec![host.to_string(), container.to_string(), name, runtime]
            })
            .collect::<Vec<_>>();
        output::table(&["HOST", "CONTAINER", "ENV", "RUNTIME"], &rows);
    }
    Ok(())
}

pub async fn doctor(data_dir: &Path, json: bool) -> Result<()> {
    let registry = registry();
    let statuses = registry.statuses().await;
    let state = StateStore::new(data_dir).load();
    let first_free = (ports::PORT_START..=ports::PORT_END)
        .find(|port| ports::port_is_free(*port).unwrap_or(false));
    let data_ok = std::fs::create_dir_all(data_dir).is_ok();
    if json {
        output::print_json(&serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "dataDir": data_dir,
            "dataDirWritable": data_ok,
            "state": match &state { Ok(state) => serde_json::json!({"envs": state.envs.len()}), Err(error) => serde_json::json!({"error": error.message}) },
            "portRange": { "start": ports::PORT_START, "end": ports::PORT_END, "firstFree": first_free },
            "runtimes": statuses,
        }));
    } else {
        println!("mixum {}", env!("CARGO_PKG_VERSION"));
        println!("data:  {}", data_dir.display());
        println!(
            "state: {}",
            match state {
                Ok(state) => format!("{} environments", state.envs.len()),
                Err(error) => format!("error: {}", error.message),
            }
        );
        println!(
            "ports: {}-{} (first free: {})",
            ports::PORT_START,
            ports::PORT_END,
            first_free
                .map(|p| p.to_string())
                .unwrap_or_else(|| "none".into())
        );
        for backend in statuses {
            println!(
                "{}: {} — {}",
                backend.kind.id(),
                backend.name,
                status_hint(&backend.status)
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_map_tokens() {
        let line = "READY token=map1.header.payload more";
        let redacted = redact_secrets(line.to_string());
        assert!(redacted.contains("map1.<redacted>"));
        assert!(!redacted.contains("payload"));
    }

    #[test]
    fn slug_is_stable_and_safe() {
        assert_eq!(slug("Fix Login!"), "fix-login");
        assert_eq!(slug("中文"), "env");
        assert!(valid_name("mixum-test-1"));
        assert!(!valid_name("Fix"));
        assert!(!valid_name("-bad"));
    }

    #[test]
    fn filters_mixumos_images() {
        let images = vec![
            "alpine:3.20".to_string(),
            "127.0.0.1:15000/mixumos:0.1.4".to_string(),
            "mixumos:arm64".to_string(),
            "<none>:<none>".to_string(),
        ];
        let candidates = image_candidates(&images);
        assert_eq!(candidates.len(), 2);
        assert!(candidates.iter().all(|image| image.contains("mixumos")));
    }
}
