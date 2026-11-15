//! Verifies: TEAM.SYS.PKG-R03, R13, R19. Bounded discovery through typed UI and SDK v3.
mod membership_support;
use serde_json::{Value, json};
use teamarness_generated_package_backend::router;
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
    let mut h = HarnessBuilder::new(router().into_backend())
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
    let mut h = HarnessBuilder::new(router().into_backend())
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
#[tokio::test]
async fn conversations_contract_or_provider_is_visible_as_excluded_and_refused_directly() {
    for provider_exclusion in [false, true] {
        let mut offered = row(0);
        offered["guideEligibility"] = "eligible".into();
        let contract = if provider_exclusion {
            offered["provider"]["identity"] = "dev.teamarness.conversations".into();
            "dev.test.contract000"
        } else {
            offered["contract"] = "dev.teamarness.conversations".into();
            "dev.teamarness.conversations"
        };
        let script = ScriptedCapability::new().calls(move |method, _| {
            assert_eq!(method, "available");
            Ok(json!({"contracts":[offered]}))
        });
        let dir = tempfile::tempdir().unwrap();
        let mut h = HarnessBuilder::new(router().into_backend())
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
                .unwrap()["contracts"][0]["excluded"],
            true
        );
        assert_eq!(h.request("external-gateway.tool.set",json!({"contract":contract,"major":1,"provider":"dev.test.provider","port":"read.item","fingerprint":"c".repeat(64),"enabled":true})).await.unwrap_err().code(),"external_gateway.forbidden");
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn oversized_summary_page_is_refused_not_truncated() {
    let script = ScriptedCapability::new()
        .calls(|_, _| Ok(json!({"contracts":(0..33).map(row).collect::<Vec<_>>()})));
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(router().into_backend())
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
