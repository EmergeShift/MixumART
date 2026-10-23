//! 真实 gateway 端到端(默认 ignored,由 `node tests/gateway-live.mjs` 驱动)。
//!
//! 环境变量:
//!   MAP_GATEWAY_URL    wss://127.0.0.1:17701
//!   MAP_GATEWAY_TOKEN  gateway bootstrap(admin)令牌
//!   MAP_GATEWAY_FP     gateway 证书指纹(可选,首次 TOFU)
//!   MAP_NODE_NAME      已注册节点名(默认 a)
use mixum_env_map::MapClient;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("缺少环境变量 {name}"))
}

#[tokio::test]
#[ignore = "需要运行中的 gateway + 节点,由 tests/gateway-live.mjs 驱动"]
async fn gateway_control_plane_roundtrip() {
    let url = env("MAP_GATEWAY_URL");
    let token = env("MAP_GATEWAY_TOKEN");
    let fingerprint = std::env::var("MAP_GATEWAY_FP").ok();
    let node = std::env::var("MAP_NODE_NAME").unwrap_or_else(|_| "a".into());

    let client = MapClient::connect(&url, &token, fingerprint.as_deref())
        .await
        .expect("connect gateway");
    assert!(client.is_gateway(), "对端应是 gateway");

    let nodes = client.node_list().await.expect("gateway.node.list");
    assert!(
        nodes.iter().any(|name| name == &node),
        "节点 {node} 应在线: {nodes:?}"
    );

    client.set_node(Some(node.clone()));
    let surfaces = client.list().await.expect("display.list via gateway");
    assert!(!surfaces.is_empty(), "节点应至少有一个窗口");
    assert!(client.node().as_deref() == Some(node.as_str()));

    let (code, expires) = client.pair_create().await.expect("gateway.pair.create");
    assert_eq!(code.len(), 6, "配对码应为 6 位");
    assert!(expires > 0);

    let claim = MapClient::pair_claim(&url, &code, "mixumart-e2e", fingerprint.as_deref())
        .await
        .expect("gateway.pair.claim");
    assert!(!claim.token.is_empty());
    assert!(!claim.device_id.is_empty());

    let device = MapClient::connect(&url, &claim.token, fingerprint.as_deref())
        .await
        .expect("设备令牌 connect");
    assert!(device.is_gateway());
    device.list().await.expect("设备令牌 display.list");

    let devices = client.device_list().await.expect("gateway.device.list");
    let target = devices
        .iter()
        .find(|device| device.sub == format!("device:{}", claim.device_id))
        .expect("新设备应出现在设备列表");
    assert!(
        client.device_revoke(&target.jti).await.expect("device.revoke"),
        "吊销应返回 revoked=true"
    );

    assert!(
        MapClient::connect(&url, &claim.token, fingerprint.as_deref())
            .await
            .is_err(),
        "吊销后的设备令牌必须被拒绝"
    );
}
