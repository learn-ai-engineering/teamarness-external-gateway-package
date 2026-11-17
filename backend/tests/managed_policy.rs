//! Golden v1 migration, full-capacity generation replacement and retained manual authority.
mod membership_support;
#[path = "support/setup.rs"]
mod support;
use serde_json::json;
use support::*;
use teamarness_generated_package_backend::{
    config::{Config, FileStore},
    policy::Change,
};
#[tokio::test]
async fn v1_golden_migrates_only_on_managed_commit_preserving_manual_policy() {
    let f = Fixture::new();
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    let mut golden = Config {
        port: Some(port),
        desired_enabled: true,
        ..Config::default()
    };
    let (manual, bearer) = teamarness_generated_package_backend::tokens::issue(
        "manual",
        7,
        None,
        &teamarness_generated_package_backend::tokens::OsEntropy,
    )
    .unwrap();
    golden.tokens.push(manual);
    golden.contracts.insert(
        "dev.test.reads".into(),
        teamarness_generated_package_backend::config::ContractSelection {
            major: 1,
            provider: "dev.test.provider".into(),
            enabled: true,
            tools: std::collections::BTreeMap::from([(
                "read.item".into(),
                teamarness_generated_package_backend::config::ToolSelection {
                    fingerprint: "a".repeat(64),
                },
            )]),
        },
    );
    let store = FileStore::open(&f.data.path().canonicalize().unwrap()).unwrap();
    store.write(&golden).unwrap();
    let (mut h, s) = f.start().await;
    assert_eq!(store.read().unwrap().config_version, 1);
    let before = serde_json::to_value(store.read().unwrap()).unwrap();
    assert_eq!(apply(&mut h, "connect").await["phase"], "complete");
    let after = serde_json::to_value(store.read().unwrap()).unwrap();
    for key in ["port", "desiredEnabled", "contracts"] {
        assert_eq!(before[key], after[key]);
    }
    assert_eq!(before["tokens"][0], after["tokens"][0]);
    assert_eq!(after["configVersion"], 2);
    assert!(store.write(&Config::default()).is_err());
    assert_eq!(serde_json::to_value(store.read().unwrap()).unwrap(), after);
    s.policy.authenticate(&bearer).await.unwrap();
    assert!(
        !std::fs::read_to_string(f.data.path().join("config/state.json"))
            .unwrap()
            .contains(&f.connections()[0].bearer)
    );
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn slot_32_rotation_in_place_and_slot_33_refusal_leave_manual_workflows() {
    let f = Fixture::new();
    let (mut h, s) = f.start().await;
    connect(&mut h).await;
    for n in 0..31 {
        h.request(
            "external-gateway.token.create",
            json!({"label":format!("manual-{n}")}),
        )
        .await
        .unwrap();
    }
    let old = f.connections().remove(0);
    assert_eq!(apply(&mut h, "rotate").await["phase"], "complete");
    let new = f.connections().remove(0);
    assert_eq!(old.token_id, new.token_id);
    assert_eq!(new.generation, old.generation + 1);
    assert!(s.policy.authenticate(&old.bearer).await.is_err());
    s.policy.authenticate(&new.bearer).await.unwrap();
    assert_eq!(
        h.request("external-gateway.token.create", json!({"label":"overflow"}))
            .await
            .unwrap_err()
            .code(),
        "external_gateway.too_large"
    );
    let status = h
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(status["tokens"].as_array().unwrap().len(), 32);
    let manual = status["tokens"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] != new.token_id)
        .unwrap()["id"]
        .clone();
    let r = h
        .request("external-gateway.token.regenerate", json!({"id":manual}))
        .await
        .unwrap();
    s.policy
        .authenticate(r["bearer"].as_str().unwrap())
        .await
        .unwrap();
    h.shutdown().await.unwrap();
    let f = Fixture::new();
    let (mut h, _) = f.start().await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    for n in 0..32 {
        h.request(
            "external-gateway.token.create",
            json!({"label":format!("manual-{n}")}),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        h.request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.too_large"
    );
    assert!(f.connections().is_empty());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn manual_revoke_leaves_tombstone_repair_refuses_and_explicit_connect_changes_generation() {
    let f = Fixture::new();
    let (mut h, s) = f.start().await;
    connect(&mut h).await;
    let old = f.connections().remove(0);
    s.policy
        .change(Change::Revoke(old.token_id.clone()))
        .await
        .unwrap();
    assert!(s.policy.authenticate(&old.bearer).await.is_err());
    assert_eq!(
        h.request("external-gateway.setup.preview", json!({"intent":"repair"}))
            .await
            .unwrap_err()
            .code(),
        "external_gateway.credential_consent_required"
    );
    h.shutdown().await.unwrap();
    drop(s);
    let (mut h, s) = f.start().await;
    assert!(s.policy.authenticate(&old.bearer).await.is_err());
    assert_eq!(apply(&mut h, "connect").await["phase"], "complete");
    let new = f.connections().remove(0);
    assert_eq!(new.token_id, old.token_id);
    assert_eq!(new.generation, old.generation + 1);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn port_only_repair_keeps_intact_credential_and_generation() {
    let f = Fixture::new();
    let (mut h, s) = f.start().await;
    connect(&mut h).await;
    let old = f.connections().remove(0);
    let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = socket.local_addr().unwrap().port();
    drop(socket);
    s.policy.change(Change::Port(port)).await.unwrap();
    assert_eq!(
        h.request("external-gateway.setup.status", json!({}))
            .await
            .unwrap()["access"],
        "repair-required"
    );
    assert_eq!(apply(&mut h, "repair").await["phase"], "complete");
    let new = f.connections().remove(0);
    assert_eq!(new.bearer, old.bearer);
    assert_eq!(new.generation, old.generation);
    assert_eq!(new.port, port);
    assert_ne!(new.operation_id, old.operation_id);
    s.policy.authenticate(&new.bearer).await.unwrap();
    h.shutdown().await.unwrap();
}
#[test]
fn future_invalid_managed_associations_and_older_backend_rollback_are_not_rewritten() {
    use std::os::unix::fs::PermissionsExt;
    for raw in [
        json!({"configVersion":3,"desiredEnabled":false,"port":null,"tokens":[],"contracts":{}}),
        json!({"configVersion":2,"desiredEnabled":false,"port":null,"tokens":[],"contracts":{},"managed":{"tokenId":"x","generation":1,"operationId":"x","originNonce":"a".repeat(64),"disposition":"current"}}),
    ] {
        let f = Fixture::new();
        let store = FileStore::open(&f.data.path().canonicalize().unwrap()).unwrap();
        let p = f.data.path().join("config/state.json");
        let bytes = serde_json::to_vec(&raw).unwrap();
        std::fs::write(&p, &bytes).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(store.read().is_err());
        assert!(store.write(&Config::default()).is_err());
        assert_eq!(std::fs::read(p).unwrap(), bytes);
    }
}
