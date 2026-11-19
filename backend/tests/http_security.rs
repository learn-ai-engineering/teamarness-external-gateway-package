//! Verifies: TEAM.SYS.PKG-R03, R13, R14. Raw-byte refusals must not invoke a Provider.
mod http_support;
use http_support::*;
use serde_json::json;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn conflicting_framing_and_duplicate_lengths_never_reach_provider() {
    for framing in [
        "Content-Length: 2\r\nTransfer-Encoding: chunked\r\n",
        "Transfer-Encoding: chunked\r\nContent-Length: 2\r\n",
        "Content-Length: 2\r\nContent-Length: 2\r\n",
    ] {
        let d = tempfile::tempdir().unwrap();
        let p = Arc::new(Mutex::new(Provider::new()));
        let mut h = start(p.clone(), d.path()).await;
        select(&mut h).await;
        let (port, bearer) = enable(&mut h).await;
        let body = call(json!({})).to_string();
        let wire = format!(
            "POST /v1/call HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\n{framing}\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
            body.len()
        );
        let out = raw(port, wire.as_bytes()).await;
        assert!(
            !out.starts_with(b"HTTP/1.1 200"),
            "ambiguous framing accepted"
        );
        assert_eq!(p.lock().unwrap().invocations, 0);
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn malformed_headers_targets_and_methods_cannot_dispatch() {
    for case in [
        "no-auth",
        "bad-auth",
        "duplicate-auth",
        "duplicate-host",
        "wrong-host",
        "wrong-port",
        "hostname",
        "absolute",
        "origin",
        "null-origin",
        "cookie",
        "token-query",
        "upgrade",
        "encoding",
        "media",
        "method",
        "batch",
        "actor",
        "stream",
        "unknown",
        "cursor",
        "event",
        "bad-header",
        "proxy",
    ] {
        let d = tempfile::tempdir().unwrap();
        let p = Arc::new(Mutex::new(Provider::new()));
        let mut h = start(p.clone(), d.path()).await;
        select(&mut h).await;
        let (port, bearer) = enable(&mut h).await;
        let mut method = "POST";
        let mut path = "/v1/call".to_owned();
        let mut host = format!("127.0.0.1:{port}");
        let mut auth = format!("Authorization: Bearer {bearer}\r\n");
        let mut extra = String::new();
        let mut media = "application/json";
        let mut body = call(json!({}));
        match case {
            "no-auth" => auth.clear(),
            "bad-auth" => auth = "Authorization: Bearer invalid\r\n".into(),
            "duplicate-auth" => auth.push_str("Authorization: Bearer invalid\r\n"),
            "duplicate-host" => extra = format!("Host: 127.0.0.1:{port}\r\n"),
            "wrong-host" => host = format!("127.0.0.2:{port}"),
            "wrong-port" => host = "127.0.0.1:1".into(),
            "hostname" => host = format!("localhost:{port}"),
            "absolute" => path = format!("http://127.0.0.1:{port}/v1/call"),
            "origin" => extra = "Origin: https://example.invalid\r\n".into(),
            "null-origin" => extra = "Origin: null\r\n".into(),
            "cookie" => {
                extra = format!("Cookie: bearer={bearer}\r\n");
                auth.clear();
            }
            "token-query" => {
                path = format!("/v1/call?token={bearer}");
                auth.clear();
            }
            "upgrade" => extra = "Upgrade: websocket\r\nConnection: Upgrade\r\n".into(),
            "encoding" => extra = "Content-Encoding: gzip\r\n".into(),
            "media" => media = "text/plain",
            "method" => method = "PUT",
            "batch" => body = json!([body]),
            "actor" => body["actor"] = json!({"kind":"person"}),
            "stream" => body["port"] = "watch.item".into(),
            "event" => body["port"] = "items.changed".into(),
            "bad-header" => extra = "Bad Header: invalid\r\n".into(),
            "proxy" => extra = "Forwarded: host=evil.invalid\r\n".into(),
            "unknown" => body["port"] = "not.enabled".into(),
            "cursor" => {
                method = "GET";
                path = "/v1/contracts?cursor=%xx".into();
                body = json!(null);
            }
            _ => unreachable!(),
        }
        let body = if method == "GET" {
            String::new()
        } else {
            body.to_string()
        };
        let out=raw(port,format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\n{auth}{extra}Content-Type: {media}\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await;
        assert!(!out.starts_with(b"HTTP/1.1 200"), "{case}");
        assert_eq!(p.lock().unwrap().invocations, 0, "{case}");
        h.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn token_revoke_and_regenerate_before_final_local_admission_prevent_dispatch() {
    use teamarness_generated_package_backend::policy::Change;
    for regenerate in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let p = Arc::new(Mutex::new(Provider::new()));
        let armed = Arc::new(Mutex::new(None));
        let (mut h, policy) = scripted(p.clone(), d.path(), armed.clone()).await;
        select(&mut h).await;
        let (port, bearer) = enable(&mut h).await;
        let token = policy.status()["tokens"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let (barrier, release) = pause();
        *armed.lock().unwrap() = Some(("available".into(), barrier.clone()));
        let read = tokio::spawn(async move { post(port, &bearer, call(json!({}))).await });
        barrier.entered.notified().await;
        policy
            .change(if regenerate {
                Change::Regenerate(token)
            } else {
                Change::Revoke(token)
            })
            .await
            .unwrap();
        drop(release);
        assert_eq!(read.await.unwrap().0, 401);
        assert_eq!(p.lock().unwrap().invocations, 0);
        h.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn already_authorized_read_finishes_once_after_revoke() {
    use teamarness_generated_package_backend::policy::Change;
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let armed = Arc::new(Mutex::new(None));
    let (mut h, policy) = scripted(p.clone(), d.path(), armed.clone()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let id = policy.status()["tokens"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (barrier, release) = pause();
    *armed.lock().unwrap() = Some(("invoke".into(), barrier.clone()));
    let token = bearer.clone();
    let read = tokio::spawn(async move { post(port, &token, call(json!({}))).await });
    barrier.entered.notified().await;
    // The Provider observes "invoke" during the admission's first poll, which still holds
    // the Policy owner until the call is enqueued. `change` deliberately never queues (it
    // refuses with retryable `busy`), so first await the owner through the FIFO lock.
    policy.authenticate(&bearer).await.unwrap();
    policy.change(Change::Revoke(id)).await.unwrap();
    drop(release);
    assert_eq!(read.await.unwrap(), (200, json!({"result":{"value":42}})));
    assert_eq!(p.lock().unwrap().invocations, 1);
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 401);
    assert_eq!(p.lock().unwrap().invocations, 1);
    h.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tool_or_master_disable_before_final_local_admission_prevents_dispatch() {
    use teamarness_generated_package_backend::policy::{Change, ToolChange};
    for master in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let p = Arc::new(Mutex::new(Provider::new()));
        {
            let mut p = p.lock().unwrap();
            let mut other = p.document["contracts"][0]["ports"][1].clone();
            other["name"] = "read.other".into();
            p.document["contracts"][0]["ports"]
                .as_array_mut()
                .unwrap()
                .push(other);
        }
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
        let fp = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["port"] == "read.other")
            .unwrap()["fingerprint"]
            .as_str()
            .unwrap()
            .to_owned();
        h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"read.other","fingerprint":fp,"enabled":true})).await.unwrap();
        let (port, bearer) = enable(&mut h).await;
        let (barrier, release) = pause();
        *armed.lock().unwrap() = Some(("available".into(), barrier.clone()));
        let body = call(json!({})).to_string();
        let wire = format!(
            "POST /v1/call HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let read = tokio::spawn(async move { raw(port, wire.as_bytes()).await });
        barrier.entered.notified().await;
        policy
            .change(if master {
                Change::Listener(false)
            } else {
                Change::Tool(ToolChange {
                    contract: "dev.test.reads".into(),
                    major: 1,
                    provider: "dev.test.provider".into(),
                    port: "read.item".into(),
                    fingerprint: fp,
                    enabled: false,
                })
            })
            .await
            .unwrap();
        drop(release);
        let bytes = read.await.unwrap();
        assert!(!bytes.starts_with(b"HTTP/1.1 200"));
        assert_eq!(p.lock().unwrap().invocations, 0);
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn changed_provider_and_route_are_refused_without_automatic_call_retry() {
    use teamarness_package_sdk::BackendError;
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), d.path()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    p.lock().unwrap().provider = "dev.teamarness.conversations".into();
    // Identity is selectable, but replacing the saved exact Provider requires review.
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 409);
    assert_eq!(
        get(port, &bearer, "/v1/contracts").await.1["contracts"],
        json!([])
    );
    assert_eq!(p.lock().unwrap().invocations, 0);
    p.lock().unwrap().provider = "dev.test.provider".into();
    p.lock().unwrap().failure = Some(BackendError::new("port_catalog_changed", false, json!({})));
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 409);
    assert_eq!(p.lock().unwrap().invocations, 1);
    h.shutdown().await.unwrap();
}
