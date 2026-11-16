//! Verifies: TEAM.SYS.PKG-R03, R05, R13, R14. Real configVersion-1 files and shared exact scope.
mod http_support;
use http_support::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
#[tokio::test]
async fn saved_version_one_two_token_reads_survive_reopen_without_new_grants() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    select(&mut h).await;
    let (port, a) = enable(&mut h).await;
    let b = h
        .request("external-gateway.token.create", json!({"label":"second"}))
        .await
        .unwrap()["bearer"]
        .as_str()
        .unwrap()
        .to_owned();
    let before = std::fs::read(dir.path().join("config/state.json")).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&before).unwrap();
    assert_eq!(saved["configVersion"], 1);
    let read_fingerprint = saved["contracts"]["dev.test.reads"]["tools"]["read.item"].clone();
    assert!(!read_fingerprint.is_null());
    h.shutdown().await.unwrap();
    let mut h = start(p.clone(), dir.path()).await;
    let reopened: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("config/state.json")).unwrap())
            .unwrap();
    assert_eq!(saved, reopened);
    for token in [&a, &b] {
        assert_eq!(post(port, token, call(json!({}))).await.0, 200);
        for name in ["write.item", "watch.item", "items.changed"] {
            let body = json!({"contract":"dev.test.reads","major":1,"port":name,"request":{}});
            assert_eq!(post(port, token, body).await.0, 403);
        }
    }
    let tools = h
        .request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"}),
        )
        .await
        .unwrap();
    let write = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["port"] == "write.item")
        .unwrap();
    h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"write.item","fingerprint":write["fingerprint"],"enabled":true})).await.unwrap();
    for token in [&a, &b] {
        assert_eq!(
            post(
                port,
                token,
                json!({"contract":"dev.test.reads","major":1,"port":"write.item","request":{}})
            )
            .await
            .0,
            200
        );
    }
    let after: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("config/state.json")).unwrap())
            .unwrap();
    assert_eq!(after["configVersion"], 1);
    assert_eq!(after["tokens"], saved["tokens"]);
    assert_eq!(
        after["contracts"]["dev.test.reads"]["tools"]["read.item"],
        read_fingerprint
    );
    assert_eq!(
        after["contracts"]["dev.test.reads"]["tools"]
            .as_object()
            .unwrap()
            .len(),
        2
    );
    h.shutdown().await.unwrap();
}
