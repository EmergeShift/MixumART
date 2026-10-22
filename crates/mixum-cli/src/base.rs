//! 母体环境：人类在 root builder 容器里安装环境，commit 成不可变 OCI 镜像。
//!
//! Apple Container 路径不依赖 Docker：rootfs tar + `FROM scratch`/`ADD` 由
//! Apple Container 自己的 builder 构建；Docker 使用同样的 rootfs tar 语义。

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use mixum_env_core::{Container, CreateSpec, RuntimeKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::env;
use crate::output::{self, CliError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct BaseMeta {
    name: String,
    runtime: String,
    builder: String,
    source_image: String,
    versions: Vec<BaseVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BaseVersion {
    pub(crate) tag: String,
    pub(crate) image: String,
    pub(crate) digest: String,
    pub(crate) created_at_ms: u128,
}

pub async fn create(
    data_dir: &Path,
    name: &str,
    image: Option<&str>,
    runtime: Option<RuntimeKind>,
    json: bool,
) -> Result<()> {
    validate_name(name)?;
    let registry = env::registry();
    let kind = choose_runtime(&registry, runtime).await?;
    let backend = registry
        .get(kind)
        .ok_or_else(|| CliError::runtime(format!("runtime {kind} 未注册")))?;
    let image = match image {
        Some(image) => image.to_owned(),
        None => {
            let images = backend
                .images()
                .await
                .map_err(|error| CliError::runtime(format!("list images: {error}")))?;
            let matches: Vec<_> = images
                .iter()
                .filter(|image| image.contains("mixumos"))
                .cloned()
                .collect();
            match matches.as_slice() {
                [only] => only.clone(),
                [] => {
                    return Err(CliError::usage(
                        "no local mixumos image; pass --image explicitly",
                    ))
                }
                _ => {
                    return Err(CliError::usage(format!(
                        "multiple mixumos images; pass --image: {}",
                        matches.join(", ")
                    )))
                }
            }
        }
    };
    let images = backend
        .images()
        .await
        .map_err(|error| CliError::runtime(format!("list images: {error}")))?;
    if !images.iter().any(|item| item == &image) {
        return Err(CliError::usage(format!(
            "image not present locally: {image}"
        )));
    }
    let builder_name = builder_name(name);
    if backend
        .list_containers()
        .await
        .map_err(|error| CliError::runtime(format!("list containers: {error}")))?
        .iter()
        .any(|container| container.name == builder_name)
    {
        return Err(CliError::conflict(format!(
            "base builder already exists: {builder_name}"
        )));
    }
    let spec = CreateSpec {
        name: builder_name.clone(),
        image: image.clone(),
        ports: vec![],
        mounts: vec![],
        env: BTreeMap::new(),
        command: vec!["sleep".into(), "infinity".into()],
    };
    let container = backend
        .create_container(spec)
        .await
        .map_err(|error| CliError::runtime(format!("create base builder: {error}")))?;
    backend
        .start_container(&container.id)
        .await
        .map_err(|error| CliError::runtime(format!("start base builder: {error}")))?;
    let meta = BaseMeta {
        name: name.to_owned(),
        runtime: kind.id().to_owned(),
        builder: container.id.clone(),
        source_image: image,
        versions: vec![],
    };
    save_meta(data_dir, &meta)?;
    let value = serde_json::json!({
        "name": name,
        "runtime": kind.id(),
        "builder": container.id,
        "image": meta.source_image,
    });
    if json {
        output::print_json(&value);
    } else {
        println!(
            "base builder {} created on {}",
            meta.builder,
            kind.display_name()
        );
        println!("run: mixum base shell {name}");
    }
    Ok(())
}

pub async fn shell(data_dir: &Path, name: &str) -> Result<()> {
    let meta = load_meta(data_dir, name)?;
    let kind = runtime_kind(&meta.runtime)?;
    let backend = env::registry()
        .get(kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    let container = find_builder(&backend, &meta).await?;
    if container.state != mixum_env_core::ContainerState::Running {
        return Err(CliError::conflict(format!(
            "base builder {} is not running",
            container.name
        )));
    }
    let (program, args) = match kind {
        RuntimeKind::AppleContainer => (
            "container",
            vec![
                "exec".to_string(),
                "-it".into(),
                container.id.clone(),
                "/bin/bash".into(),
            ],
        ),
        RuntimeKind::Docker => (
            "docker",
            vec![
                "exec".to_string(),
                "-it".into(),
                container.id.clone(),
                "/bin/bash".into(),
            ],
        ),
        RuntimeKind::Wsl2 => {
            return Err(CliError::usage(
                "base builder terminal is not implemented for WSL2 yet",
            ))
        }
    };
    let status = Command::new(program)
        .args(args)
        .status()
        .map_err(|error| CliError::runtime(format!("open base shell: {error}")))?;
    if !status.success() {
        return Err(CliError::runtime(format!(
            "base shell exited with {status}"
        )));
    }
    Ok(())
}

pub async fn commit(data_dir: &Path, name: &str, tag: Option<&str>, json: bool) -> Result<()> {
    let mut meta = load_meta(data_dir, name)?;
    let kind = runtime_kind(&meta.runtime)?;
    let backend = env::registry()
        .get(kind)
        .ok_or_else(|| CliError::runtime("runtime disappeared"))?;
    let container = find_builder(&backend, &meta).await?;
    if container.state != mixum_env_core::ContainerState::Running {
        return Err(CliError::conflict("base builder is not running"));
    }
    let tag = match tag {
        Some(tag) => tag.to_owned(),
        None => format!("v{}", meta.versions.len() + 1),
    };
    if meta.versions.iter().any(|version| version.tag == tag) {
        return Err(CliError::conflict(format!(
            "base version already exists: {tag}"
        )));
    }
    let build_dir = data_dir
        .join("bases")
        .join(name)
        .join(format!("build-{tag}"));
    if build_dir.exists() {
        std::fs::remove_dir_all(&build_dir)
            .map_err(|error| CliError::io(format!("clean build dir: {error}")))?;
    }
    std::fs::create_dir_all(&build_dir)
        .map_err(|error| CliError::io(format!("create build dir: {error}")))?;
    let rootfs_tar = build_dir.join("rootfs.tar");
    export_rootfs(kind, &container, &rootfs_tar)?;
    let config = image_config(kind, &meta.source_image)?;
    write_containerfile(&build_dir, &config)?;
    let image = if kind == RuntimeKind::Wsl2 {
        rootfs_tar.display().to_string()
    } else {
        format!("mixum-base-{name}:{tag}")
    };
    if kind != RuntimeKind::Wsl2 {
        build_image(kind, &build_dir, &image)?;
    }
    let digest = image_digest(kind, &image)?;
    let version = BaseVersion {
        tag: tag.clone(),
        image: image.clone(),
        digest: digest.clone(),
        created_at_ms: now_ms(),
    };
    meta.versions.push(version.clone());
    save_meta(data_dir, &meta)?;
    let value = serde_json::json!({
        "name": name,
        "tag": tag,
        "image": image,
        "digest": digest,
        "sourceImage": meta.source_image,
    });
    if json {
        output::print_json(&value);
    } else {
        println!("committed {name}:{tag} -> {image}");
        println!("digest: {digest}");
    }
    Ok(())
}

pub async fn list(data_dir: &Path, json: bool) -> Result<()> {
    let dir = data_dir.join("bases");
    let mut bases = Vec::new();
    if dir.exists() {
        for entry in std::fs::read_dir(&dir).map_err(|error| CliError::io(error.to_string()))? {
            let path = entry
                .map_err(|error| CliError::io(error.to_string()))?
                .path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Ok(meta) = read_meta(&path) {
                bases.push(meta);
            }
        }
    }
    bases.sort_by(|a, b| a.name.cmp(&b.name));
    if json {
        output::print_json(&bases);
    } else if bases.is_empty() {
        println!("no bases");
    } else {
        for base in bases {
            let latest = base.versions.last();
            println!(
                "{}\t{}\t{}\t{}",
                base.name,
                base.runtime,
                base.versions.len(),
                latest
                    .map(|v| format!("{} ({})", v.image, v.digest))
                    .unwrap_or_else(|| "-".into())
            );
        }
    }
    Ok(())
}

pub fn resolve_image(data_dir: &Path, spec: &str) -> Result<String> {
    resolve_version(data_dir, spec).map(|version| version.image)
}

pub(crate) fn resolve_version(data_dir: &Path, spec: &str) -> Result<BaseVersion> {
    let (name, tag) = match spec.split_once(':') {
        Some((name, tag)) => (name, Some(tag)),
        None => (spec, None),
    };
    let meta = load_meta(data_dir, name)?;
    let version = match tag {
        Some(tag) => meta
            .versions
            .iter()
            .find(|version| version.tag == tag)
            .ok_or_else(|| CliError::usage(format!("base version not found: {spec}")))?,
        None => meta
            .versions
            .last()
            .ok_or_else(|| CliError::usage(format!("base has no committed version: {name}")))?,
    };
    Ok(version.clone())
}

async fn choose_runtime(
    registry: &mixum_env_core::BackendRegistry,
    requested: Option<RuntimeKind>,
) -> Result<RuntimeKind> {
    if let Some(kind) = requested {
        let backend = registry
            .get(kind)
            .ok_or_else(|| CliError::runtime(format!("runtime {kind} not registered")))?;
        if !backend.probe().await.is_ready() {
            return Err(CliError::runtime(format!("runtime {kind} is not ready")));
        }
        return Ok(kind);
    }
    let order = if cfg!(target_os = "macos") {
        [
            RuntimeKind::AppleContainer,
            RuntimeKind::Docker,
            RuntimeKind::Wsl2,
        ]
    } else if cfg!(windows) {
        [
            RuntimeKind::Wsl2,
            RuntimeKind::Docker,
            RuntimeKind::AppleContainer,
        ]
    } else {
        [
            RuntimeKind::Docker,
            RuntimeKind::Wsl2,
            RuntimeKind::AppleContainer,
        ]
    };
    for kind in order {
        if let Some(backend) = registry.get(kind) {
            if backend.probe().await.is_ready() {
                return Ok(kind);
            }
        }
    }
    Err(CliError::runtime("no ready runtime for base environment"))
}

fn runtime_kind(raw: &str) -> Result<RuntimeKind> {
    match raw {
        "apple_container" => Ok(RuntimeKind::AppleContainer),
        "docker" => Ok(RuntimeKind::Docker),
        "wsl2" => Ok(RuntimeKind::Wsl2),
        other => Err(CliError::io(format!(
            "unknown runtime in base metadata: {other}"
        ))),
    }
}

async fn find_builder(
    backend: &std::sync::Arc<dyn mixum_env_core::RuntimeBackend>,
    meta: &BaseMeta,
) -> Result<Container> {
    backend
        .list_containers()
        .await
        .map_err(|error| CliError::runtime(format!("list containers: {error}")))?
        .into_iter()
        .find(|container| container.id == meta.builder || container.name == meta.builder)
        .ok_or_else(|| CliError::runtime("base builder container is missing"))
}

fn export_rootfs(kind: RuntimeKind, container: &Container, dest: &Path) -> Result<()> {
    let (program, exec_args) = match kind {
        RuntimeKind::AppleContainer => (
            "container",
            vec![
                "exec".to_string(),
                container.id.clone(),
                "sh".into(),
                "-c".into(),
                "sync && tar --xattrs --acls --numeric-owner \
                 --exclude=./proc --exclude=./sys --exclude=./dev --exclude=./run \
                 --exclude='./tmp/*' --exclude=./var/lib/mixum \
                 --exclude=./etc/machine-id --exclude='./etc/ssh/ssh_host_*' \
                 -cf /tmp/rootfs.tar ."
                    .into(),
            ],
        ),
        RuntimeKind::Docker => (
            "docker",
            vec![
                "exec".to_string(),
                container.id.clone(),
                "sh".into(),
                "-c".into(),
                "sync && tar --xattrs --acls --numeric-owner \
                 --exclude=./proc --exclude=./sys --exclude=./dev --exclude=./run \
                 --exclude='./tmp/*' --exclude=./var/lib/mixum \
                 --exclude=./etc/machine-id --exclude='./etc/ssh/ssh_host_*' \
                 -cf /tmp/rootfs.tar ."
                    .into(),
            ],
        ),
        RuntimeKind::Wsl2 => {
            return Err(CliError::usage(
                "base commit is not implemented for WSL2 yet",
            ))
        }
    };
    run(program, &exec_args, "export rootfs")?;
    let (copy_program, copy_args) = match kind {
        RuntimeKind::AppleContainer => (
            "container",
            vec![
                "copy".into(),
                format!("{}:/tmp/rootfs.tar", container.id),
                dest.display().to_string(),
            ],
        ),
        RuntimeKind::Docker => (
            "docker",
            vec![
                "cp".into(),
                format!("{}:/tmp/rootfs.tar", container.id),
                dest.display().to_string(),
            ],
        ),
        RuntimeKind::Wsl2 => unreachable!(),
    };
    run(copy_program, &copy_args, "copy rootfs")?;
    Ok(())
}

fn image_config(kind: RuntimeKind, image: &str) -> Result<ImageConfig> {
    let (program, args) = match kind {
        RuntimeKind::AppleContainer => ("container", vec!["image", "inspect", image]),
        RuntimeKind::Docker => ("docker", vec!["image", "inspect", image]),
        RuntimeKind::Wsl2 => return Err(CliError::usage("WSL2 has no image config")),
    };
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| CliError::runtime(format!("inspect image: {error}")))?;
    if !output.status.success() {
        return Err(CliError::runtime(format!(
            "inspect image failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| CliError::runtime(format!("parse image inspect: {error}")))?;
    let array = value
        .as_array()
        .ok_or_else(|| CliError::runtime("image inspect is not an array"))?;
    let first = array
        .first()
        .ok_or_else(|| CliError::runtime("image inspect is empty"))?;
    let config = match kind {
        RuntimeKind::AppleContainer => first
            .get("variants")
            .and_then(Value::as_array)
            .and_then(|variants| variants.first())
            .and_then(|variant| variant.get("config"))
            .and_then(|config| config.get("config"))
            .unwrap_or(&Value::Null),
        RuntimeKind::Docker => first.get("Config").unwrap_or(&Value::Null),
        RuntimeKind::Wsl2 => unreachable!(),
    };
    let env = config
        .get("Env")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let entrypoint = string_array(config.get("Entrypoint"));
    let cmd = string_array(config.get("Cmd"));
    let workdir = config
        .get("WorkingDir")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let user = config
        .get("User")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    Ok(ImageConfig {
        env,
        entrypoint,
        cmd,
        workdir,
        user,
    })
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

struct ImageConfig {
    env: Vec<String>,
    entrypoint: Vec<String>,
    cmd: Vec<String>,
    workdir: String,
    user: String,
}

fn write_containerfile(dir: &Path, config: &ImageConfig) -> Result<()> {
    let mut text = String::from("FROM scratch\nADD rootfs.tar /\n");
    for env in &config.env {
        text.push_str(&format!(
            "ENV {}\n",
            env.replace('\\', "\\\\").replace('"', "\\\"")
        ));
    }
    if !config.entrypoint.is_empty() {
        text.push_str(&format!("ENTRYPOINT {}\n", json_array(&config.entrypoint)));
    }
    if !config.cmd.is_empty() {
        text.push_str(&format!("CMD {}\n", json_array(&config.cmd)));
    }
    if !config.workdir.is_empty() {
        text.push_str(&format!(
            "WORKDIR \"{}\"\n",
            config.workdir.replace('"', "\\\"")
        ));
    }
    if !config.user.is_empty() {
        text.push_str(&format!("USER \"{}\"\n", config.user.replace('"', "\\\"")));
    }
    std::fs::write(dir.join("Containerfile"), text)
        .map_err(|error| CliError::io(format!("write Containerfile: {error}")))
}

fn json_array(items: &[String]) -> String {
    let values = items
        .iter()
        .map(|item| format!("\"{}\"", item.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>();
    format!("[{}]", values.join(","))
}

fn build_image(kind: RuntimeKind, dir: &Path, tag: &str) -> Result<()> {
    let (program, args) = match kind {
        RuntimeKind::AppleContainer => (
            "container",
            vec![
                "build".to_string(),
                "-t".into(),
                tag.into(),
                "-f".into(),
                dir.join("Containerfile").display().to_string(),
                dir.display().to_string(),
            ],
        ),
        RuntimeKind::Docker => (
            "docker",
            vec![
                "build".to_string(),
                "-t".into(),
                tag.into(),
                "-f".into(),
                dir.join("Containerfile").display().to_string(),
                dir.display().to_string(),
            ],
        ),
        RuntimeKind::Wsl2 => return Err(CliError::usage("base build is not implemented for WSL2")),
    };
    let status = Command::new(program)
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|error| CliError::runtime(format!("base build: {error}")))?;
    if !status.success() {
        return Err(CliError::runtime(format!(
            "base build exited with {status}"
        )));
    }
    Ok(())
}

fn image_digest(kind: RuntimeKind, image: &str) -> Result<String> {
    if kind == RuntimeKind::Wsl2 {
        use sha2::{Digest, Sha256};
        let data = std::fs::read(image)
            .map_err(|error| CliError::io(format!("read WSL base tar: {error}")))?;
        return Ok(format!("{:x}", Sha256::digest(&data)));
    }
    let (program, args) = match kind {
        RuntimeKind::AppleContainer => ("container", vec!["image", "inspect", image]),
        RuntimeKind::Docker => ("docker", vec!["image", "inspect", image]),
        RuntimeKind::Wsl2 => unreachable!(),
    };
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| CliError::runtime(format!("image digest: {error}")))?;
    if !output.status.success() {
        return Err(CliError::runtime("image digest inspect failed"));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| CliError::runtime(format!("parse image digest: {error}")))?;
    let digest = value
        .as_array()
        .and_then(|items| items.first())
        .and_then(|item| item.get("id").or_else(|| item.get("Id")))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    Ok(digest.to_owned())
}

fn run(program: &str, args: &[String], what: &str) -> Result<()> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| CliError::runtime(format!("{what}: {error}")))?;
    if !output.status.success() {
        return Err(CliError::runtime(format!(
            "{what} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

fn load_meta(data_dir: &Path, name: &str) -> Result<BaseMeta> {
    read_meta(&data_dir.join("bases").join(format!("{name}.json")))
}

fn read_meta(path: &Path) -> Result<BaseMeta> {
    let data = std::fs::read(path)
        .map_err(|error| CliError::io(format!("read base metadata: {error}")))?;
    serde_json::from_slice(&data)
        .map_err(|error| CliError::io(format!("parse base metadata: {error}")))
}

fn save_meta(data_dir: &Path, meta: &BaseMeta) -> Result<()> {
    let dir = data_dir.join("bases");
    std::fs::create_dir_all(&dir)
        .map_err(|error| CliError::io(format!("create base dir: {error}")))?;
    let path = dir.join(format!("{}.json", meta.name));
    let data = serde_json::to_vec_pretty(meta)
        .map_err(|error| CliError::io(format!("serialize base metadata: {error}")))?;
    std::fs::write(path, data)
        .map_err(|error| CliError::io(format!("write base metadata: {error}")))
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 63
        || !name
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(CliError::usage(
            "base name must be 1-63 chars of lowercase letters, digits or '-'",
        ));
    }
    Ok(())
}

fn builder_name(name: &str) -> String {
    format!("mixum-base-{name}")
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}
