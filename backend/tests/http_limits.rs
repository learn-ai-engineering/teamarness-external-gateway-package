//! Verifies: TEAM.SYS.PKG-R02, R13, R14. Exact public transport budgets.
mod http_support;
use http_support::*;
use serde_json::json;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
#[tokio::test]
async fn original_256_kib_values_fit_in_addition_to_the_fixed_envelope() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    p.lock().unwrap().document["contracts"][0]["ports"][1]["request"] = json!({});
    p.lock().unwrap().document["contracts"][0]["ports"][1]["response"] = json!({});
    let mut h = start(p.clone(), d.path()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let value = json!("x".repeat(262142));
    p.lock().unwrap().result = value.clone();
    assert_eq!(serde_json::to_vec(&value).unwrap().len(), 262144);
    assert_eq!(
        post(port, &bearer, call(value.clone())).await,
        (200, json!({"result":value}))
    );
    let before = p.lock().unwrap().invocations;
    assert_eq!(
        post(port, &bearer, call(json!("x".repeat(262143)))).await.0,
        413
    );
    assert_eq!(p.lock().unwrap().invocations, before);
    p.lock().unwrap().result = json!("x".repeat(262143));
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 413);
    let body = "x".repeat(262144 + 4096 + 1);
    let mut conn = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    conn.write_all(format!("POST /v1/call HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",body.len()).as_bytes()).await.unwrap();
    let mut bytes = vec![];
    let _ = conn.read_to_end(&mut bytes).await;
    assert!(bytes.starts_with(b"HTTP/1.1 413"));
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn header_count_bytes_and_target_enforce_limit_and_first_rejected_value() {
    for (kind, limit) in [("count", 64), ("bytes", 16384), ("target", 4096)] {
        let d = tempfile::tempdir().unwrap();
        let mut h = start(Arc::new(Mutex::new(Provider::new())), d.path()).await;
        let (port, bearer) = enable(&mut h).await;
        for size in [limit, limit + 1] {
            let path = if kind == "target" {
                format!("/{}", "x".repeat(size - 1))
            } else {
                "/v1".into()
            };
            let mut wire = format!(
                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\n"
            );
            if kind == "count" {
                for i in 2..size {
                    wire.push_str(&format!("X-{i}: x\r\n"));
                }
            }
            if kind == "bytes" {
                let padding = size - wire.len() - "X-Padding: \r\n\r\n".len();
                wire.push_str(&format!("X-Padding: {}\r\n", "x".repeat(padding)));
            }
            wire.push_str("\r\n");
            let out = raw(port, wire.as_bytes()).await;
            if size == limit {
                assert!(
                    out.starts_with(if kind == "target" {
                        b"HTTP/1.1 404"
                    } else {
                        b"HTTP/1.1 200"
                    }),
                    "limit {kind}"
                );
            } else {
                assert!(
                    !out.starts_with(b"HTTP/1.1 200") && !out.starts_with(b"HTTP/1.1 404"),
                    "first rejected {kind}"
                );
            }
        }
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn thirty_third_connection_is_refused_and_incomplete_headers_expire() {
    let d = tempfile::tempdir().unwrap();
    let mut h = start(Arc::new(Mutex::new(Provider::new())), d.path()).await;
    let (port, _) = enable(&mut h).await;
    let mut held = Vec::new();
    for _ in 0..32 {
        let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        c.write_all(b"G").await.unwrap();
        held.push(c);
    }
    let mut excess = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut buf = [0; 1];
    let n = tokio::time::timeout(Duration::from_secs(1), excess.read(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(n, 0);
    let began = tokio::time::Instant::now();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(6), held[0].read(&mut buf))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert!(began.elapsed() >= Duration::from_secs(3));
    h.dispose().await.unwrap();
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn request_body_read_has_one_five_second_deadline() {
    let d = tempfile::tempdir().unwrap();
    let mut h = start(Arc::new(Mutex::new(Provider::new())), d.path()).await;
    let (port, bearer) = enable(&mut h).await;
    let mut c = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    c.write_all(format!("POST /v1/call HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: 10\r\n\r\n{{").as_bytes()).await.unwrap();
    let mut out = vec![];
    tokio::time::timeout(Duration::from_secs(6), c.read_to_end(&mut out))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response(&out).0, 504);
    h.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twelve_socket_reads_are_owned_and_thirteenth_refused_without_waiting() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let armed = Arc::new(Mutex::new(None));
    let (mut h, _policy) = scripted(p.clone(), d.path(), armed.clone()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let (barrier, release) = pause();
    *armed.lock().unwrap() = Some(("available".into(), barrier.clone()));
    let mut reads = tokio::task::JoinSet::new();
    for _ in 0..13 {
        let token = bearer.clone();
        reads.spawn(async move { post(port, &token, call(json!({}))).await });
    }
    barrier.entered.notified().await;
    let busy = tokio::time::timeout(Duration::from_secs(2), reads.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(busy.0, 503);
    assert_eq!(busy.1["error"]["code"], "external_gateway.busy");
    // The synchronous harness callback pauses metadata delivery, not 12 Provider executions.
    // All 12 gateway call slots are retained while their real SDK futures are pending.
    drop(release);
    while let Some(read) = reads.join_next().await {
        let (status, value) = read.unwrap();
        assert_eq!(status, 200, "terminal code: {:?}", value["error"]["code"]);
    }
    assert_eq!(p.lock().unwrap().invocations, 12);
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 200);
    assert_eq!(p.lock().unwrap().invocations, 13);
    h.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_sdk_earlier_timeout_releases_slot_and_does_not_poison_recovery() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let armed = Arc::new(Mutex::new(None));
    let (mut h, _policy) = scripted(p.clone(), d.path(), armed.clone()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let (barrier, release) = pause();
    *armed.lock().unwrap() = Some(("invoke".into(), barrier.clone()));
    let token = bearer.clone();
    let read = tokio::spawn(async move { post(port, &token, call(json!({}))).await });
    barrier.entered.notified().await;
    let terminal = tokio::time::timeout(Duration::from_secs(13), read)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(terminal.0, 504);
    assert_eq!(terminal.1["error"]["code"], "deadline_exceeded");
    assert_eq!(terminal.1["error"]["source"], "host");
    drop(release);
    h.suspend().await.unwrap();
    h.recover().await.unwrap();
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 200);
    assert_eq!(p.lock().unwrap().invocations, 2);
    h.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disconnected_socket_keeps_its_sdk_slot_and_guide_shares_the_same_budget() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let armed = Arc::new(Mutex::new(None));
    let (mut h, _policy) = scripted(p.clone(), d.path(), armed.clone()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let (barrier, release) = pause();
    *armed.lock().unwrap() = Some(("invoke".into(), barrier.clone()));
    let token = bearer.clone();
    let lost = tokio::spawn(async move { post(port, &token, call(json!({}))).await });
    barrier.entered.notified().await;
    lost.abort();
    let _ = lost.await;
    let mut reads = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let token = bearer.clone();
        reads.spawn(async move { get(port, &token, "/v1/contracts/dev.test.reads/1/guide").await });
    }
    let busy = tokio::time::timeout(Duration::from_secs(2), reads.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(busy.1["error"]["code"], "external_gateway.busy");
    drop(release);
    while let Some(read) = reads.join_next().await {
        assert_eq!(read.unwrap().0, 200);
    }
    assert_eq!(p.lock().unwrap().invocations, 12);
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 200);
    assert_eq!(p.lock().unwrap().invocations, 13);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn workspace_rate_bucket_is_charged_before_invalid_authentication() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), d.path()).await;
    let (port, _) = enable(&mut h).await;
    // Pause time after activation so this tests exact burst rather than machine throughput.
    tokio::time::pause();
    // Real socket readiness is external to Tokio's paused clock. Keep one runnable test
    // task so idle auto-advance cannot refill the bucket between kernel notifications.
    let clock_guard = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    for _ in 0..16 {
        assert_eq!(get(port, "invalid", "/v1").await.0, 401);
    }
    assert_eq!(get(port, "invalid", "/v1").await.0, 429);
    assert_eq!(p.lock().unwrap().invocations, 0);
    clock_guard.abort();
    tokio::time::resume();
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn metadata_document_limit_is_exact_and_never_truncated() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use teamarness_generated_package_backend::{catalog::digest, router};
    use teamarness_package_sdk::testing::{HarnessBuilder, ScriptedCapability};
    for size in [1048576, 1048577] {
        let p = Arc::new(Mutex::new(Provider::new()));
        let source = p.lock().unwrap().document.clone();
        let mut bytes = serde_json::to_vec(&source).unwrap();
        bytes.resize(size, b' ');
        let sha = digest(&bytes);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = calls.clone();
        let script=ScriptedCapability::new().calls(move |method,payload|{
            if method=="describe"{seen.fetch_add(1,std::sync::atomic::Ordering::SeqCst);let offset=payload["offset"].as_u64().unwrap() as usize;let end=(offset+65536).min(size);let mut out=json!({"metadataSha256":sha,"totalBytes":size,"offset":offset,"data":STANDARD.encode(&bytes[offset..end])});if end<size{out["nextOffset"]=end.into();}return Ok(out);}
            let mut out=p.lock().unwrap().call(method,payload)?;if method=="available"{out["contracts"][0]["metadataSha256"]=sha.clone().into();}Ok(out)
        });
        let d = tempfile::tempdir().unwrap();
        let mut h = HarnessBuilder::new(router().into_backend())
            .data_directory(d.path().canonicalize().unwrap())
            .installation(&"b".repeat(64), "Local")
            .capability("teamarness.membership", membership_support::present())
            .capability_version("teamarness.package-ports", script, 3, 0)
            .start()
            .await;
        h.activate().await.unwrap();
        let result = h
            .request(
                "external-gateway.catalog.page",
                json!({"contract":"dev.test.reads"}),
            )
            .await;
        if size == 1048576 {
            assert!(result.is_ok());
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 16);
        } else {
            assert!(result.is_err());
            assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        }
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn summary_page_byte_budget_accepts_32768_and_refuses_32769() {
    use teamarness_generated_package_backend::router;
    use teamarness_package_sdk::testing::{HarnessBuilder, ScriptedCapability};
    for size in [32768, 32769] {
        let mut provider = Provider::new();
        let one = provider
            .call("available", json!({"access":"person"}))
            .unwrap()["contracts"][0]
            .clone();
        let mut page = json!({"contracts":(0..32).map(|i|{let mut r=one.clone();r["contract"]=format!("dev.test.read{i:03}").into();r["provider"]["description"]="x".into();r}).collect::<Vec<_>>()});
        let mut remaining = size - serde_json::to_vec(&page).unwrap().len();
        for row in page["contracts"].as_array_mut().unwrap() {
            let add = remaining.min(1023);
            row["provider"]["description"] = "x".repeat(1 + add).into();
            remaining -= add;
        }
        assert_eq!(remaining, 0);
        assert_eq!(serde_json::to_vec(&page).unwrap().len(), size);
        let script = ScriptedCapability::new().calls(move |method, _| {
            assert_eq!(method, "available");
            Ok(page.clone())
        });
        let d = tempfile::tempdir().unwrap();
        let mut h = HarnessBuilder::new(router().into_backend())
            .data_directory(d.path().canonicalize().unwrap())
            .installation(&"b".repeat(64), "Local")
            .capability("teamarness.membership", membership_support::present())
            .capability_version("teamarness.package-ports", script, 3, 0)
            .start()
            .await;
        h.activate().await.unwrap();
        let (port, bearer) = enable(&mut h).await;
        let (status, value) = get(port, &bearer, "/v1/contracts").await;
        if size == 32768 {
            assert_eq!(status, 200);
            assert_eq!(value["contracts"], json!([]));
        } else {
            assert_eq!(status, 503);
            assert_eq!(
                value["error"]["code"],
                "package.invalid_capability_response"
            );
        }
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn guide_preface_cannot_expand_the_fixed_response_envelope_budget() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    {
        let mut p = p.lock().unwrap();
        p.document["contracts"][0]["ports"][0]["response"]["properties"]["markdown"]["maxLength"] =
            262144.into();
        p.guide = "x".repeat(262129);
        assert_eq!(
            serde_json::to_vec(&json!({"markdown":p.guide}))
                .unwrap()
                .len(),
            262144
        );
        let ordinary = p.document["contracts"][0]["ports"][1].clone();
        for i in 0..40 {
            let mut port = ordinary.clone();
            port["name"] = format!("read.{}x{i:02}", "x".repeat(120)).into();
            p.document["contracts"][0]["ports"]
                .as_array_mut()
                .unwrap()
                .push(port);
        }
    }
    let mut h = start(p.clone(), d.path()).await;
    for offset in [0, 32] {
        let tools = h
            .request(
                "external-gateway.catalog.page",
                json!({"contract":"dev.test.reads","offset":offset}),
            )
            .await
            .unwrap();
        for tool in tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["eligible"] == true)
        {
            h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":tool["port"],"fingerprint":tool["fingerprint"],"enabled":true})).await.unwrap();
        }
    }
    let (port, bearer) = enable(&mut h).await;
    assert_eq!(
        get(port, &bearer, "/v1/contracts/dev.test.reads/1/guide")
            .await
            .0,
        413
    );
    h.shutdown().await.unwrap();
}
