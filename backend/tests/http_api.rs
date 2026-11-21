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
    assert_eq!(manifest["readOnly"], false);
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../contracts/http-v1.schema.json")).unwrap();
    let contract = json!({"schemaVersion":1,"$defs":schema["$defs"],"requests":{"test.manifest":{"request":{"type":"object","additionalProperties":false},"response":{"$ref":"#/$defs/manifest"}}},"subscriptions":{}});
    teamarness_package_contract::ui_contract::compile_ui_contract(
        &serde_json::to_vec(&contract).unwrap(),
    )
    .unwrap()
    .validate_response("test.manifest", &manifest)
    .unwrap();
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
        json!({"provider":{"code":"fixture.capacity","retryable":true,"outcomeUncertain":false,"details":{"limit":8,"domain":{"title":"arbitrary Provider content"},"reason":"capacity"}}}),
    ));
    let (status, e) = post(port, &bearer, call(json!({}))).await;
    assert_eq!(status, 503);
    assert_eq!(
        e,
        json!({"error":{"source":"provider","code":"fixture.capacity","retryable":true,"outcomeUncertain":false,"details":{"limit":8,"domain":{"title":"arbitrary Provider content"},"reason":"capacity"}}})
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

#[tokio::test]
async fn four_shapes_are_exact_opt_in_and_commands_dispatch_from_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    let (port, bearer) = enable(&mut h).await;
    h.request("external-gateway.package.set",json!({"provider":"dev.test.provider","enabled":true,"contracts":[{"contract":"dev.test.reads","major":1}]})).await.unwrap();
    let tools = h
        .request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"}),
        )
        .await
        .unwrap();
    for (name, kind, delivery) in [
        ("read.item", "query", "response"),
        ("write.item", "command", "response"),
        ("watch.item", "query", "stream"),
        ("items.changed", "event", "stream"),
    ] {
        let tool = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["port"] == name)
            .unwrap();
        assert_eq!(tool["eligible"], true, "{name}");
        assert_eq!(tool["kind"], kind);
        assert_eq!(tool["delivery"], delivery);
        let body = json!({"contract":"dev.test.reads","major":1,"port":name,"request":{}});
        assert_eq!(post(port, &bearer, body.clone()).await.0, 403);
        h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":name,"fingerprint":tool["fingerprint"],"enabled":true})).await.unwrap();
        let result = post(port, &bearer, body).await;
        assert_eq!(result.0, if delivery == "response" { 200 } else { 400 });
    }
    assert_eq!(p.lock().unwrap().invocations, 2);
    let schema = get(port, &bearer, "/v1/contracts/dev.test.reads/1").await.1;
    assert_eq!(
        schema["schema"]["contracts"][0]["ports"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
    assert_eq!(get(port, &bearer, "/v1").await.1["readOnly"], false);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn complete_provider_conflict_details_survive_without_host_diagnostics() {
    use teamarness_package_sdk::BackendError;
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let details = json!({"current":{"version":"opaque","title":"Concurrent edit","files":[{"path":"index.html","sha256":"a".repeat(64)}]},"missing":["blob"],"revision":12});
    p.lock().unwrap().failure = Some(BackendError::new(
        "provider_error",
        false,
        json!({"provider":{"code":"fixture.conflict","retryable":false,"outcomeUncertain":false,"details":details},"path":"/private/host","authorization":bearer}),
    ));
    let response = post(port, &bearer, call(json!({}))).await.1;
    assert_eq!(response["error"]["details"], details);
    assert!(!response.to_string().contains(&bearer));
    assert!(!response.to_string().contains("/private/host"));
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn command_result_loss_is_uncertain_but_authoritative_refusal_is_certain() {
    use teamarness_package_sdk::BackendError;
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    let tools = h
        .request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"}),
        )
        .await
        .unwrap();
    let t = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["port"] == "write.item")
        .unwrap();
    h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"write.item","fingerprint":t["fingerprint"],"enabled":true})).await.unwrap();
    let (port, bearer) = enable(&mut h).await;
    let body = json!({"contract":"dev.test.reads","major":1,"port":"write.item","request":{}});
    p.lock().unwrap().result = json!("x".repeat(262145));
    let e = post(port, &bearer, body.clone()).await.1;
    assert_eq!(e["error"]["outcomeUncertain"], true);
    assert_eq!(e["error"]["retryable"], false);
    p.lock().unwrap().failure = Some(BackendError::new(
        "provider_error",
        false,
        json!({"provider":{"code":"fixture.refused","retryable":false,"outcomeUncertain":false,"details":{}}}),
    ));
    assert_eq!(
        post(port, &bearer, body.clone()).await.1["error"]["outcomeUncertain"],
        false
    );
    p.lock().unwrap().failure = Some(BackendError::new(
        "provider_error",
        false,
        json!({"provider":{"code":"fixture.uncertain","retryable":true,"outcomeUncertain":true,"details":{}}}),
    ));
    assert_eq!(
        post(port, &bearer, body).await.1["error"]["outcomeUncertain"],
        true
    );
    assert_eq!(p.lock().unwrap().invocations, 3);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn provider_details_are_complete_at_byte_and_depth_limits_and_dropped_whole_beyond() {
    use teamarness_package_sdk::BackendError;
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let exact = json!({"data":"x".repeat(4085)});
    assert_eq!(serde_json::to_vec(&exact).unwrap().len(), 4096);
    let mut nested = json!({"leaf":1});
    for _ in 1..8 {
        nested = json!({"nested":nested});
    }
    for (details, expected) in [
        (exact.clone(), exact),
        (json!({"data":"x".repeat(4086)}), json!({})),
        (nested.clone(), nested.clone()),
        (json!({"oneTooDeep":nested}), json!({})),
    ] {
        p.lock().unwrap().failure = Some(BackendError::new(
            "provider_error",
            false,
            json!({"provider":{"code":"fixture.conflict","retryable":false,"outcomeUncertain":false,"details":details}}),
        ));
        assert_eq!(
            post(port, &bearer, call(json!({}))).await.1["error"]["details"],
            expected
        );
    }
    h.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitted_command_survives_observer_loss_and_revoke_with_slot_owned_until_terminal() {
    use teamarness_generated_package_backend::policy::Change;
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let armed = Arc::new(Mutex::new(None));
    let (mut h, policy) = scripted(p.clone(), d.path(), armed.clone()).await;
    select(&mut h).await;
    let tools = h
        .request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"}),
        )
        .await
        .unwrap();
    let t = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["port"] == "write.item")
        .unwrap();
    h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"write.item","fingerprint":t["fingerprint"],"enabled":true})).await.unwrap();
    let (port, bearer) = enable(&mut h).await;
    let other = h
        .request("external-gateway.token.create", json!({"label":"reader"}))
        .await
        .unwrap()["bearer"]
        .as_str()
        .unwrap()
        .to_owned();
    let id = policy.status()["tokens"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (barrier, release) = pause();
    *armed.lock().unwrap() = Some(("invoke".into(), barrier.clone()));
    let token = bearer.clone();
    let lost = tokio::spawn(async move {
        post(
            port,
            &token,
            json!({"contract":"dev.test.reads","major":1,"port":"write.item","request":{}}),
        )
        .await
    });
    barrier.entered.notified().await;
    lost.abort();
    let _ = lost.await;
    // The Provider observes "invoke" during the admission's first poll, which still holds
    // the Policy owner until the call is enqueued. `change` deliberately never queues (it
    // refuses with retryable `busy`), so first await the owner through the FIFO lock.
    policy.authenticate(&bearer).await.unwrap();
    policy.change(Change::Revoke(id)).await.unwrap();
    let mut readers = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let token = other.clone();
        readers
            .spawn(async move { get(port, &token, "/v1/contracts/dev.test.reads/1/guide").await });
    }
    let busy = tokio::time::timeout(std::time::Duration::from_secs(2), readers.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(busy.1["error"]["code"], "external_gateway.busy");
    drop(release);
    while let Some(r) = readers.join_next().await {
        assert_eq!(r.unwrap().0, 200);
    }
    assert_eq!(
        p.lock().unwrap().invocations,
        12,
        "one admitted command and eleven guides, no replay"
    );
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 401);
    assert_eq!(post(port, &other, call(json!({}))).await.0, 200);
    h.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn command_sdk_timeout_is_uncertain_without_replaying_or_quarantining_capacity() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let armed = Arc::new(Mutex::new(None));
    let (mut h, _) = scripted(p.clone(), d.path(), armed.clone()).await;
    let tools = h
        .request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"}),
        )
        .await
        .unwrap();
    let t = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["port"] == "write.item")
        .unwrap();
    h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"write.item","fingerprint":t["fingerprint"],"enabled":true})).await.unwrap();
    let (port, bearer) = enable(&mut h).await;
    let (barrier, release) = pause();
    *armed.lock().unwrap() = Some(("invoke".into(), barrier.clone()));
    let token = bearer.clone();
    let call = tokio::spawn(async move {
        post(
            port,
            &token,
            json!({"contract":"dev.test.reads","major":1,"port":"write.item","request":{}}),
        )
        .await
    });
    barrier.entered.notified().await;
    let result = tokio::time::timeout(std::time::Duration::from_secs(13), call)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.1["error"]["code"], "outcome_unknown");
    assert_eq!(result.1["error"]["outcomeUncertain"], true);
    assert_eq!(result.1["error"]["retryable"], false);
    drop(release);
    // This independent lifecycle request is a positive barrier after the late command response.
    h.suspend().await.unwrap();
    h.recover().await.unwrap();
    assert_eq!(p.lock().unwrap().invocations, 1);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn each_nonread_selection_alone_includes_guide_and_complete_reachable_schemas() {
    for name in ["write.item", "watch.item", "items.changed"] {
        let d = tempfile::tempdir().unwrap();
        let p = Arc::new(Mutex::new(Provider::new()));
        {
            let mut p = p.lock().unwrap();
            p.document["$defs"]["Payload"] = json!({"type":"object","properties":{"nested":{"$ref":"#/$defs/Nested"}},"required":["nested"],"additionalProperties":false});
            p.document["$defs"]["Nested"] = json!({"type":"string","maxLength":16});
            let ports = p.document["contracts"][0]["ports"].as_array_mut().unwrap();
            let port = ports.iter_mut().find(|p| p["name"] == name).unwrap();
            port[if name == "write.item" {
                "response"
            } else {
                "event"
            }] = json!({"$ref":"#/$defs/Payload"});
        }
        let mut h = start(p, d.path()).await;
        let tools = h
            .request(
                "external-gateway.catalog.page",
                json!({"contract":"dev.test.reads"}),
            )
            .await
            .unwrap();
        let t = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["port"] == name)
            .unwrap();
        h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":name,"fingerprint":t["fingerprint"],"enabled":true})).await.unwrap();
        let (port, bearer) = enable(&mut h).await;
        let schema = get(port, &bearer, "/v1/contracts/dev.test.reads/1").await.1;
        let names: std::collections::BTreeSet<_> = schema["schema"]["contracts"][0]["ports"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from([name, "runtime.guide"])
        );
        assert_eq!(
            schema["schema"]["$defs"]["Payload"]["properties"]["nested"]["$ref"],
            "#/$defs/Nested"
        );
        assert_eq!(schema["schema"]["$defs"]["Nested"]["maxLength"], 16);
        assert!(schema["schema"]["$defs"].get("Result").is_none());
        let guide = get(port, &bearer, "/v1/contracts/dev.test.reads/1/guide").await;
        assert_eq!(guide.0, 200);
        assert!(guide.1["preface"].as_str().unwrap().contains(name));
        h.shutdown().await.unwrap();
    }
}
