//! 真实两环境之间的 `.mart` 迁移 e2e。
//!
//! 运行：
//! MAP_URL=127.0.0.1:17601 MAP_TOKEN=... TARGET_URL=127.0.0.1:17602 TARGET_TOKEN=... \
//!   cargo test -p mixum-env-map --test migration_e2e -- --ignored --nocapture

use mixum_env_map::MapClient;
use sha2::{Digest, Sha256};

fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[tokio::test]
#[ignore = "需要两个运行中的 agentd：MAP_URL/TOKEN + TARGET_URL/TOKEN"]
async fn live_migrate_snapshot_between_envs() {
    let source_url = std::env::var("MAP_URL").expect("MAP_URL");
    let source_token = std::env::var("MAP_TOKEN").expect("MAP_TOKEN");
    let target_url = std::env::var("TARGET_URL").expect("TARGET_URL");
    let target_token = std::env::var("TARGET_TOKEN").expect("TARGET_TOKEN");

    let source = MapClient::connect(&source_url, &source_token, None)
        .await
        .expect("connect source");
    let target = MapClient::connect(&target_url, &target_token, None)
        .await
        .expect("connect target");

    let tree = source.snapshot_tree().await.expect("source tree");
    let nodes = tree
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .expect("nodes");
    let node = nodes
        .iter()
        .rev()
        .find(|node| node.get("kind").and_then(serde_json::Value::as_str) == Some("disk"))
        .expect("至少一个 disk 快照");
    let node_id = node
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("node id");
    eprintln!("source node: {node_id}");

    let reply = source.snapshot_export(node_id).await.expect("snap.export");
    let file = reply
        .get("file")
        .and_then(serde_json::Value::as_str)
        .expect("file");
    let expected = reply
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let data = source
        .get_file(&format!("transfers/{file}"))
        .await
        .expect("file.get");
    assert!(!data.is_empty(), "迁移包为空");
    if !expected.is_empty() {
        assert_eq!(hex(Sha256::digest(&data)), expected, "sha256 不一致");
    }
    eprintln!("mart: {file} ({} bytes)", data.len());

    target.put_file(file, &data).await.expect("file.put");
    let imported = target
        .snapshot_import(file, "e2e")
        .await
        .expect("snap.import");
    let target_node = imported
        .get("node")
        .and_then(serde_json::Value::as_str)
        .expect("目标节点");
    eprintln!("target node: {target_node}");

    let target_tree = target.snapshot_tree().await.expect("target tree");
    let found = target_tree
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|nodes| {
            nodes
                .iter()
                .any(|node| node.get("id").and_then(serde_json::Value::as_str) == Some(target_node))
        });
    assert!(found, "目标树里找不到导入节点");
}
