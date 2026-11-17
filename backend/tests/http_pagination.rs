// AC01/AC06/AC15: HTTP cursor envelopes over real sockets and the existing SDK seam.
use http_support::backend_environment;
mod http_support;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use http_support::*;
use serde_json::{Value, json};
use teamarness_package_sdk::{
    BackendError,
    testing::{Harness, HarnessBuilder, ScriptedCapability},
};

async fn paged(directory: &std::path::Path, cursor: String) -> (Harness, ScriptedCapability) {
    let provider = std::sync::Mutex::new(Provider::new());
    let script = ScriptedCapability::new().calls(move |method, payload| {
        if method == "available" {
            if payload.get("cursor").is_some() {
                if payload["cursor"] == "stale" {
                    return Err(BackendError::new("port_catalog_changed", false, json!({})));
                }
                assert_eq!(
                    payload["cursor"], cursor,
                    "Host must receive its original opaque bytes"
                );
                return provider.lock().unwrap().call(method, payload);
            }
            let mut page = provider.lock().unwrap().call(method, payload)?;
            let row = page["contracts"][0].clone();
            page["contracts"] = json!(
                (0..32)
                    .map(|n| {
                        let mut row = row.clone();
                        row["contract"] = format!("dev.test.unselected{n:02}").into();
                        row["enabled"] = false.into();
                        row["enabledMajor"] = Value::Null;
                        row
                    })
                    .collect::<Vec<_>>()
            );
            page["nextCursor"] = cursor.clone().into();
            return Ok(page);
        }
        provider.lock().unwrap().call(method, payload)
    });
    let mut h = HarnessBuilder::new(backend_environment::router().into_backend())
        .data_directory(directory.canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", script.clone(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    (h, script)
}

#[tokio::test]
async fn exact_returned_http_cursor_crosses_empty_page_and_terminates_at_enabled_contract() {
    // Host cursors use standard base64 components separated by a dot; the envelope
    // must preserve these characters and arbitrary opaque UTF-8, not interpret them.
    for cursor in [
        "+/8=.YWJjZA==".to_owned(),
        "?&% ü".to_owned(),
        "a".repeat(2048),
        "é".repeat(1024),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (mut h, script) = paged(dir.path(), cursor.clone()).await;
        let ui = h
            .request("external-gateway.catalog.page", json!({}))
            .await
            .unwrap();
        assert_eq!(ui["nextCursor"], cursor, "UI cursor must remain unchanged");
        select(&mut h).await;
        let (port, bearer) = enable(&mut h).await;
        let (status, first) = get(port, &bearer, "/v1/contracts").await;
        assert_eq!(status, 200);
        assert_eq!(first["contracts"], json!([]));
        let next = first["nextCursor"].as_str().unwrap();
        let count = script.recorded_calls().len();
        let (status, last) = get(port, &bearer, &format!("/v1/contracts?cursor={next}")).await;
        assert_eq!(status, 200, "the exact returned cursor must be followable");
        assert_eq!(next, URL_SAFE_NO_PAD.encode(cursor.as_bytes()));
        assert!(next.len() <= 2731);
        if cursor.len() == 2048 {
            assert_eq!(next.len(), 2731);
        }
        assert_eq!(
            last["contracts"],
            json!([{"contract":"dev.test.reads","major":1,"minor":0}])
        );
        assert_eq!(last["nextCursor"], Value::Null);
        // Fetch the requested page, then independently reobserve the selected contract
        // across both Host pages for current metadata/policy. No stale-cursor retry.
        let cursors: Vec<_> = script.recorded_calls()[count..]
            .iter()
            .filter(|call| call.method == "available")
            .map(|call| call.payload["cursor"].clone())
            .collect();
        assert_eq!(cursors, vec![json!(cursor), Value::Null, json!(cursor)]);
        h.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn malformed_or_oversized_cursor_envelopes_are_refused_without_host_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, script) = paged(dir.path(), "unused".into()).await;
    let (port, bearer) = enable(&mut h).await;
    let before = script.recorded_calls().len();
    for cursor in [
        "".to_owned(),
        "A".into(),
        "AB".into(),
        "YQ=".into(),
        "YQ==".into(),
        ".".into(),
        "+/".into(),
        "%59Q".into(),
        "YQ&cursor=Yg".into(),
        "_w".into(),
        URL_SAFE_NO_PAD.encode("a".repeat(2049)),
    ] {
        let (status, body) = get(port, &bearer, &format!("/v1/contracts?cursor={cursor}")).await;
        assert_eq!(status, 400);
        assert_eq!(body["error"]["code"], "external_gateway.bad_request");
        assert_eq!(script.recorded_calls().len(), before);
    }
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn stale_http_cursor_reaches_host_once_and_preserves_catalog_changed() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, script) = paged(dir.path(), "unused".into()).await;
    let (port, bearer) = enable(&mut h).await;
    let before = script.recorded_calls().len();
    // Independent known vector: base64url("stale") = c3RhbGU.
    let (status, body) = get(port, &bearer, "/v1/contracts?cursor=c3RhbGU").await;
    assert_eq!(status, 409);
    assert_eq!(body["error"]["code"], "port_catalog_changed");
    assert_eq!(script.recorded_calls().len(), before + 1);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn oversized_host_cursor_is_not_published_as_an_http_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let (mut h, _) = paged(dir.path(), "a".repeat(2049)).await;
    let (port, bearer) = enable(&mut h).await;
    let (status, body) = get(port, &bearer, "/v1/contracts").await;
    assert_eq!(status, 503);
    assert_eq!(body["error"]["code"], "package.invalid_capability_response");
    assert!(body.get("nextCursor").is_none());
    h.shutdown().await.unwrap();
}
