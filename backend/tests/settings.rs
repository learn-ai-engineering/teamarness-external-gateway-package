//! Verifies: TEAM.SYS.PKG-R05, R13, R19. Unavailable selections through the typed UI seam.
mod http_support;
use http_support::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use teamarness_generated_package_backend::router;
use teamarness_package_sdk::testing::{Harness, HarnessBuilder, ScriptedCapability};

async fn setup(
    dir: &std::path::Path,
    provider: Arc<Mutex<Provider>>,
    missing: Arc<Mutex<bool>>,
    disabled: Arc<Mutex<Vec<String>>>,
) -> Harness {
    let script = ScriptedCapability::new().calls(move |method, payload| {
        if method == "disable" {
            disabled
                .lock()
                .unwrap()
                .push(payload["contract"].as_str().unwrap().to_owned());
        }
        let mut value = provider.lock().unwrap().call(method, payload)?;
        if method == "available" && *missing.lock().unwrap() {
            let row = &mut value["contracts"][0];
            row["available"] = false.into();
            row["provider"] = Value::Null;
            row["metadataSha256"] = Value::Null;
            row["routeRevision"] = Value::Null;
            row["guideEligibility"] = "unavailable".into();
        }
        Ok(value)
    });
    let mut h = HarnessBuilder::new(router().into_backend())
        .data_directory(dir.canonicalize().unwrap())
        .capability_version("teamarness.package-ports", script, 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    h
}
fn removal() -> Value {
    json!({"provider":null,"enabled":false,"contracts":[{"contract":"dev.test.reads","major":1}]})
}

#[tokio::test]
async fn host_only_unavailable_selection_is_removable_without_provider_identity() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    p.lock().unwrap().enabled = true;
    let disabled = Arc::new(Mutex::new(vec![]));
    let mut h = setup(
        dir.path(),
        p.clone(),
        Arc::new(Mutex::new(true)),
        disabled.clone(),
    )
    .await;
    let row = h
        .request("external-gateway.catalog.page", json!({}))
        .await
        .unwrap();
    assert!(row["contracts"][0]["provider"].is_null());
    assert_eq!(row["contracts"][0]["hostEnabled"], true);
    h.request("external-gateway.package.set", removal())
        .await
        .unwrap();
    assert_eq!(*disabled.lock().unwrap(), vec!["dev.test.reads"]);
    assert!(!p.lock().unwrap().enabled);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn null_provider_enable_available_or_ambiguous_removal_is_refused_without_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let missing = Arc::new(Mutex::new(false));
    let disabled = Arc::new(Mutex::new(vec![]));
    let mut h = setup(dir.path(), p, missing.clone(), disabled.clone()).await;
    assert!(
        h.request("external-gateway.package.set", removal())
            .await
            .is_err()
    );
    *missing.lock().unwrap() = true;
    let mut enable = removal();
    enable["enabled"] = true.into();
    let mut duplicate = removal();
    duplicate["contracts"]
        .as_array_mut()
        .unwrap()
        .push(json!({"contract":"dev.test.reads","major":1}));
    let mut wrong = removal();
    wrong["contracts"][0]["major"] = 2.into();
    let mut unknown = removal();
    unknown["contracts"][0]["contract"] = "dev.test.other".into();
    for invalid in [
        enable,
        duplicate,
        wrong,
        unknown,
        json!({"provider":null,"enabled":false,"contracts":[]}),
    ] {
        assert!(
            h.request("external-gateway.package.set", invalid)
                .await
                .is_err()
        );
    }
    assert!(disabled.lock().unwrap().is_empty());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn saved_unavailable_provider_groups_and_cleanup_failure_remains_durably_denied() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let missing = Arc::new(Mutex::new(false));
    let disabled = Arc::new(Mutex::new(vec![]));
    let mut h = setup(dir.path(), p.clone(), missing.clone(), disabled.clone()).await;
    select(&mut h).await;
    *missing.lock().unwrap() = true;
    p.lock().unwrap().cleanup_fails = true;
    let rows = h
        .request("external-gateway.catalog.page", json!({}))
        .await
        .unwrap();
    assert_eq!(rows["contracts"][0]["provider"], "dev.test.provider");
    let reply = h.request("external-gateway.package.set", json!({"provider":"dev.test.provider","enabled":false,"contracts":[{"contract":"dev.test.reads","major":1}]})).await.unwrap();
    assert_eq!(reply["warning"], "host_cleanup_failed");
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["selectedTools"],
        0
    );
    h.shutdown().await.unwrap();
    let mut reopened = setup(dir.path(), p, missing, disabled).await;
    assert_eq!(
        reopened
            .request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["selectedContracts"],
        0
    );
    reopened.shutdown().await.unwrap();
}

#[tokio::test]
async fn null_provider_cleanup_failure_persists_only_requested_denial_and_keeps_tokens() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let missing = Arc::new(Mutex::new(false));
    let disabled = Arc::new(Mutex::new(vec![]));
    let mut h = setup(dir.path(), p.clone(), missing.clone(), disabled.clone()).await;
    select(&mut h).await;
    let token = h
        .request(
            "external-gateway.token.create",
            json!({"label":"retained client"}),
        )
        .await
        .unwrap();
    let mut watch = h
        .subscribe("external-gateway.status.watch", json!({}))
        .await
        .unwrap();
    let event = watch.next_event().await.unwrap().unwrap();
    assert!(
        !event
            .to_string()
            .contains(token["bearer"].as_str().unwrap())
    );
    watch.close().await;
    *missing.lock().unwrap() = true;
    p.lock().unwrap().cleanup_fails = true;
    let reply = h
        .request("external-gateway.package.set", removal())
        .await
        .unwrap();
    assert_eq!(reply["warning"], "host_cleanup_failed");
    assert_eq!(*disabled.lock().unwrap(), vec!["dev.test.reads"]);
    h.shutdown().await.unwrap();
    let mut reopened = setup(dir.path(), p, missing, disabled).await;
    let status = reopened
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(status["selectedTools"], 0);
    assert_eq!(status["tokens"][0]["id"], token["id"]);
    reopened.shutdown().await.unwrap();
}
