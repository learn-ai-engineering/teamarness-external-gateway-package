//! Verifies: TEAM.SYS.PKG-R13, R14, R19. HTTP-only generic discovery, not real-Host or human AC14 proof.
mod http_support;
use http_support::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

// Deliberately receives only transport coordinates and a credential, no Provider/contract/tool names.
async fn generic_read(port: u16, bearer: &str) -> Value {
    let (status, manifest) = get(port, bearer, "/v1").await;
    assert_eq!(status, 200);
    assert_eq!(manifest["readOnly"], true);
    let (status, onboarding) = get(
        port,
        bearer,
        manifest["routes"]["onboarding"].as_str().unwrap(),
    )
    .await;
    assert_eq!(status, 200);
    let guidance = onboarding["markdown"].as_str().unwrap();
    assert!(guidance.contains("<BEARER>"));
    assert!(!guidance.contains(bearer));
    assert!(guidance.contains("12 outstanding"));
    assert!(guidance.contains("25-second upper"));
    assert!(guidance.contains("at most one delayed retry"));
    assert!(guidance.contains("guideSha256"));
    assert!(guidance.contains("projectionSha256"));
    assert!(guidance.contains("may still be sent to the Host"));
    assert!(guidance.contains("external agent's own stop control"));
    let contracts_route = manifest["routes"]["contracts"].as_str().unwrap();
    let mut cursor = None;
    loop {
        let path = cursor.as_ref().map_or_else(
            || contracts_route.to_owned(),
            |c| format!("{contracts_route}?cursor={c}"),
        );
        let (status, page) = get(port, bearer, &path).await;
        assert_eq!(status, 200);
        for row in page["contracts"].as_array().unwrap() {
            let path = format!(
                "{contracts_route}/{}/{}",
                row["contract"].as_str().unwrap(),
                row["major"]
            );
            let (status, schema) = get(port, bearer, &path).await;
            assert_eq!(status, 200);
            let (status, guide) = get(port, bearer, &format!("{path}/guide")).await;
            assert_eq!(status, 200);
            assert_eq!(guide["contract"], row["contract"]);
            let ports = schema["schema"]["contracts"][0]["ports"]
                .as_array()
                .unwrap();
            // This generic demonstration supports an empty-object request; it does not invent domain inputs.
            if let Some(read) = ports.iter().find(|p| {
                p["kind"] == "query"
                    && p["name"] != "runtime.guide"
                    && p["request"]["type"] == "object"
                    && p["request"]
                        .get("required")
                        .is_none_or(|r| r.as_array().is_some_and(|r| r.is_empty()))
            }) {
                let body = json!({"contract":row["contract"],"major":row["major"],"port":read["name"],"request":{}}).to_string();
                let call_route = manifest["routes"]["call"].as_str().unwrap();
                let bytes = raw(port, format!("POST {call_route} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await;
                let (status, result) = response(&bytes);
                assert_eq!(status, 200);
                return result["result"].clone();
            }
        }
        cursor = page["nextCursor"].as_str().map(str::to_owned);
        assert!(
            cursor.is_some(),
            "no compatible enabled ordinary query discovered"
        );
    }
}
#[tokio::test]
async fn generic_http_client_follows_manifest_schemas_live_guide_and_discovered_read() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    assert_eq!(generic_read(port, &bearer).await, json!({"value":42}));
    assert_eq!(
        p.lock().unwrap().invocations,
        2,
        "one guide, one ordinary query; no replay or writes"
    );
    h.shutdown().await.unwrap();
}
