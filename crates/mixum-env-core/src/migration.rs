//! 迁移包元数据：`.mart` 旁边的小 JSON，记录镜像/容器定义/节点来源。
//!
//! `.mart` 只装文件；镜像、env、端口、command、GUI 会话在这里，供目标端重建容器。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::container::CreateSpec;

pub const MIGRATION_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationManifest {
    pub version: u32,
    pub source: MigrationSource,
    pub container: CreateSpec,
    pub mart: MartRef,
    #[serde(default)]
    pub gui_sessions: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSource {
    pub container_id: String,
    #[serde(default)]
    pub runtime: String,
    pub node: String,
    #[serde(default)]
    pub branch: String,
    /// Unix 秒；0 表示未记录。
    #[serde(default)]
    pub exported_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MartRef {
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

impl MigrationManifest {
    /// 镜像引用里若带 `@sha256:`，取 digest；否则 None（由目标端按 tag pull）。
    pub fn image_digest(&self) -> Option<&str> {
        self.container
            .image
            .split_once("@sha256:")
            .map(|(_, digest)| digest)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, bytes)
    }

    pub fn read(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let data = std::fs::read(path)?;
        serde_json::from_slice(&data)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn manifest() -> MigrationManifest {
        MigrationManifest {
            version: MIGRATION_VERSION,
            source: MigrationSource {
                container_id: "map-node".into(),
                runtime: "apple_container".into(),
                node: "01J".into(),
                branch: "main".into(),
                exported_at: 1,
            },
            container: CreateSpec {
                name: "map-node".into(),
                image: "ghcr.io/emergeShift/mixumos@sha256:abc".into(),
                ports: Vec::new(),
                mounts: Vec::new(),
                env: BTreeMap::new(),
                command: Vec::new(),
            },
            mart: MartRef {
                file: "mixum-01J.mart".into(),
                sha256: "abc".into(),
                size: 1,
            },
            gui_sessions: Vec::new(),
        }
    }

    #[test]
    fn roundtrip_and_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migration.json");
        let manifest = manifest();
        assert_eq!(manifest.image_digest(), Some("abc"));
        manifest.write(&path).unwrap();
        assert_eq!(MigrationManifest::read(path).unwrap(), manifest);
    }
}
