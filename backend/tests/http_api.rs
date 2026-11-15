//! Verifies: TEAM.SYS.PKG-R02, R03, R13, R14. Real sockets; only Host capability scripted.
mod http_support;
use http_support::*;
use serde_json::json;
use std::sync::{Arc, Mutex};
#[tokio::test]
async fn listener_authenticates_before_revealing_manifest_or_unknown_routes() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    let (port, bearer) = enable(&mut h).await;
    let denied = get(port, "invalid", "/v1").await;
    assert_eq!(denied.0, 401);
    assert_eq!(denied, get(port, "invalid", "/unknown").await);
    let (status, manifest) = get(port, &bearer, "/v1").await;
    assert_eq!(status, 200);
    assert_eq!(manifest["readOnly"], true);
    assert_eq!(manifest["workspace"]["id"], "workspace1");
    assert_eq!(manifest["routes"]["contracts"], "/v1/contracts");
    assert_eq!(
        get(port, &bearer, "/v1/contracts").await.1,
        json!({"contracts":[],"nextCursor":null})
    );
    assert_eq!(p.lock().unwrap().invocations, 0);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn enabled_schema_contains_only_selected_reads_and_reachable_definitions_and_live_guide() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    p.lock().unwrap().document["$defs"]["Unused"] = json!({"type":"string"});
    let mut h = start(p.clone(), dir.path()).await;
    let (port, bearer) = enable(&mut h).await;
    assert_eq!(
        get(port, &bearer, "/v1/contracts/dev.test.reads/1").await.0,
        403
    );
    select(&mut h).await;
    let page = get(port, &bearer, "/v1/contracts").await;
    assert_eq!(page.0, 200);
    assert_eq!(page.1["contracts"][0]["contract"], "dev.test.reads");
    let (status, schema) = get(port, &bearer, "/v1/contracts/dev.test.reads/1").await;
    assert_eq!(status, 200);
    let names: Vec<_> = schema["schema"]["contracts"][0]["ports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["read.item", "runtime.guide"]);
    assert_eq!(schema["schema"]["$defs"]["Result"]["type"], "object");
    assert!(schema["schema"]["$defs"].get("Unused").is_none());
    use sha2::{Digest, Sha256};
    let digest: String = Sha256::digest(serde_json::to_vec(&schema["schema"]).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(schema["projectionSha256"], digest);
    let guide = get(port, &bearer, "/v1/contracts/dev.test.reads/1/guide").await;
    assert_eq!(guide.0, 200);
    assert_eq!(guide.1["markdown"], "Unknown provider guide");
    assert!(guide.1["preface"].as_str().unwrap().contains("read.item"));
    assert!(!guide.1["preface"].as_str().unwrap().contains("write.item"));
    assert_eq!(
        guide.1["guideSha256"],
        Sha256::digest(b"Unknown provider guide")
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        post(port, &bearer, call(json!({}))).await,
        (200, json!({"result":{"value":42}}))
    );
    let onboard = get(port, &bearer, "/v1/onboarding").await;
    assert_eq!(onboard.0, 200);
    assert!(onboard.1["markdown"].as_str().unwrap().contains("<BEARER>"));
    assert!(!onboard.1.to_string().contains(&bearer));
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn arbitrary_values_provider_errors_and_live_guide_bytes_are_preserved_without_secrets() {
    use teamarness_package_sdk::BackendError;
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    p.lock().unwrap().document["contracts"][0]["ports"][1]["response"] = json!({});
    let mut h = start(p.clone(), dir.path()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    for value in [
        json!(null),
        json!(42),
        json!("text"),
        json!([1, false]),
        json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":null}),
    ] {
        p.lock().unwrap().result = value.clone();
        assert_eq!(
            post(port, &bearer, call(json!({}))).await,
            (200, json!({"result":value}))
        );
    }
    p.lock().unwrap().guide = "New guide: write.item is described but not granted.\n".into();
    let guide = get(port, &bearer, "/v1/contracts/dev.test.reads/1/guide")
        .await
        .1;
    assert_eq!(guide["markdown"], p.lock().unwrap().guide);
    assert_eq!(
        guide["guideSha256"],
        teamarness_generated_package_backend::catalog::digest(p.lock().unwrap().guide.as_bytes())
    );
    let mut write = call(json!({}));
    write["port"] = "write.item".into();
    assert_eq!(post(port, &bearer, write).await.0, 403);
    p.lock().unwrap().failure = Some(BackendError::new(
        "provider_error",
        false,
        json!({"provider":{"code":"fixture.capacity","retryable":true,"outcomeUncertain":false,"details":{"limit":8,"path":"/private/data","token":bearer,"reason":"capacity"}}}),
    ));
    let (status, e) = post(port, &bearer, call(json!({}))).await;
    assert_eq!(status, 503);
    assert_eq!(
        e,
        json!({"error":{"source":"provider","code":"fixture.capacity","retryable":true,"outcomeUncertain":false,"details":{"limit":8,"reason":"capacity"}}})
    );
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn metadata_cache_rechecks_current_selection_and_schema_before_exposure() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), d.path()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let before = p.lock().unwrap().describes;
    assert_eq!(
        get(port, &bearer, "/v1/contracts/dev.test.reads/1").await.0,
        200
    );
    assert_eq!(
        get(port, &bearer, "/v1/contracts/dev.test.reads/1").await.0,
        200
    );
    assert_eq!(
        p.lock().unwrap().describes,
        before,
        "digest-identical metadata should use the bounded cache"
    );
    p.lock().unwrap().document["$defs"]["Result"]["properties"]["value"]["maximum"] = json!(50);
    assert_eq!(
        get(port, &bearer, "/v1/contracts/dev.test.reads/1").await.0,
        403
    );
    assert_eq!(
        get(port, &bearer, "/v1/contracts").await.1["contracts"],
        json!([])
    );
    h.shutdown().await.unwrap();
}
