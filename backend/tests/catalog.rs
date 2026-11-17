//! Verifies: TEAM.SYS.PKG-R03, R13, R19. Bounded discovery through typed UI and SDK v3.
use http_support::backend_environment;
use http_support::membership_support;
use serde_json::{Value, json};
use teamarness_package_sdk::{
    BackendError,
    testing::{HarnessBuilder, ScriptedCapability},
};
fn row(n: usize) -> Value {
    json!({"contract":format!("dev.test.contract{n:03}"),"major":1,"minor":0,"description":"Read","available":true,"enabled":false,"enabledMajor":null,"metadataSha256":"b".repeat(64),"provider":{"identity":"dev.test.provider","version":"1.0.0","displayName":"Unknown","description":null},"guideEligibility":"missing","routeRevision":"a".repeat(64)})
}
#[tokio::test]
async fn one_ui_request_reads_one_bounded_host_page_and_never_enables_it() {
    let script = ScriptedCapability::new().calls(|method, payload| {
        assert_eq!(method, "available");
        assert_eq!(payload["access"], "person");
        if payload.get("cursor").is_none() {
            Ok(json!({"contracts":(0..32).map(row).collect::<Vec<_>>(),"nextCursor":"next-page"}))
        } else {
            assert_eq!(payload["cursor"], "next-page");
            Ok(json!({"contracts":[row(32)]}))
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(backend_environment::router().into_backend())
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", script.clone(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    let first = h
        .request("external-gateway.catalog.page", json!({}))
        .await
        .unwrap();
    assert_eq!(first["contracts"].as_array().unwrap().len(), 32);
    assert_eq!(first["nextCursor"], "next-page");
    assert_eq!(script.recorded_calls().len(), 1);
    let second = h
        .request(
            "external-gateway.catalog.page",
            json!({"cursor":first["nextCursor"]}),
        )
        .await
        .unwrap();
    assert_eq!(second["contracts"].as_array().unwrap().len(), 1);
    assert_eq!(second["nextCursor"], Value::Null);
    assert_eq!(script.recorded_calls().len(), 2);
    let refused=h.request("external-gateway.tool.set",json!({"contract":"dev.test.contract000","major":1,"provider":"dev.test.provider","port":"read.item","fingerprint":"c".repeat(64),"enabled":true})).await.unwrap_err();
    assert_eq!(refused.code(), "external_gateway.forbidden");
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn catalog_churn_restarts_once_then_refuses_without_dispatch() {
    let script = ScriptedCapability::new().calls(|method, _| {
        assert_eq!(method, "available");
        Err(BackendError::new("port_catalog_changed", false, json!({})))
    });
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(backend_environment::router().into_backend())
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", script.clone(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    assert_eq!(
        h.request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"})
        )
        .await
        .unwrap_err()
        .code(),
        "port_catalog_changed"
    );
    assert_eq!(script.recorded_calls().len(), 2);
    h.shutdown().await.unwrap();
}
mod http_support;
#[tokio::test]
async fn conversations_contract_and_provider_identities_support_exact_explicit_selection() {
    for contract_identity in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut provider = http_support::Provider::new();
        let contract = if contract_identity {
            "dev.teamarness.conversations"
        } else {
            "dev.test.reads"
        };
        provider.document["contracts"][0]["identity"] = contract.into();
        if !contract_identity {
            provider.provider = "dev.teamarness.conversations".into();
        }
        let identity = provider.provider.clone();
        let p = std::sync::Arc::new(std::sync::Mutex::new(provider));
        let mut h = http_support::start(p.clone(), dir.path()).await;
        let (port, bearer) = http_support::enable(&mut h).await;
        let body = json!({"contract":contract,"major":1,"port":"read.item","request":{}});
        assert_eq!(http_support::post(port, &bearer, body.clone()).await.0, 403);
        let page = h
            .request("external-gateway.catalog.page", json!({}))
            .await
            .unwrap();
        assert!(page["contracts"][0].get("excluded").is_none());
        let tools = h
            .request(
                "external-gateway.catalog.page",
                json!({"contract":contract}),
            )
            .await
            .unwrap();
        for tool in tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["port"] != "runtime.guide")
        {
            assert_eq!(tool["eligible"], true);
            h.request("external-gateway.tool.set",json!({"contract":contract,"major":1,"provider":identity,"port":tool["port"],"fingerprint":tool["fingerprint"],"enabled":true})).await.unwrap();
        }
        assert_eq!(http_support::post(port, &bearer, body).await.0, 200);
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn oversized_summary_page_is_refused_not_truncated() {
    let script = ScriptedCapability::new()
        .calls(|_, _| Ok(json!({"contracts":(0..33).map(row).collect::<Vec<_>>()})));
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(backend_environment::router().into_backend())
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", script, 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    assert_eq!(
        h.request("external-gateway.catalog.page", json!({}))
            .await
            .unwrap_err()
            .code(),
        "package.invalid_capability_response"
    );
    h.shutdown().await.unwrap();
}
