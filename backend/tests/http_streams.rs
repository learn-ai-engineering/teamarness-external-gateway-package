//! Verifies: TEAM.SYS.PKG-R03, R13, R14. Real sockets and Policy; only SDK capability scripted.
mod http_support;
use http_support::*;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use teamarness_generated_package_backend::router;
use teamarness_package_sdk::testing::{Harness, HarnessBuilder, ScriptedCapability};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};
async fn setup() -> (tempfile::TempDir, Harness, ScriptedCapability, u16, String) {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let script =
        ScriptedCapability::new().calls(move |m, payload| p.lock().unwrap().call(m, payload));
    let mut h = HarnessBuilder::new(router().into_backend())
        .process_minor(1)
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", script.clone(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    for name in ["read.item", "watch.item", "items.changed"] {
        select_tool(&mut h, name, true).await;
    }
    let (port, bearer) = enable(&mut h).await;
    (dir, h, script, port, bearer)
}
async fn select_tool(h: &mut Harness, name: &str, enabled: bool) {
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
    h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":name,"fingerprint":t["fingerprint"],"enabled":enabled})).await.unwrap();
}
async fn open(port: u16, bearer: &str, name: &str, request: Value) -> BufReader<TcpStream> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let body =
        json!({"contract":"dev.test.reads","major":1,"port":name,"request":request}).to_string();
    s.write_all(format!("POST /v1/subscribe HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    BufReader::new(s)
}
async fn headers(s: &mut BufReader<TcpStream>) -> String {
    let mut out = String::new();
    loop {
        let mut line = String::new();
        s.read_line(&mut line).await.unwrap();
        out.push_str(&line);
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }
    out
}
async fn frame(s: &mut BufReader<TcpStream>) -> Value {
    tokio::time::timeout(Duration::from_secs(20),async {let mut size=String::new();s.read_line(&mut size).await.unwrap();let n=usize::from_str_radix(size.trim(),16).expect("chunk length");assert!(n>0);let mut bytes=vec![0;n];s.read_exact(&mut bytes).await.unwrap();assert_eq!(bytes.last(),Some(&b'\n'));let mut crlf=[0;2];s.read_exact(&mut crlf).await.unwrap();assert_eq!(&crlf,b"\r\n");let value:Value=serde_json::from_slice(&bytes).unwrap();
 // The UI validator has its own 256 KiB envelope ceiling. HTTP frames add 4 KiB.
 // The unconstrained value schema accepts every root; validate frame structure with null
 // while independently asserting both original HTTP byte budgets and exact payload below.
 assert!(bytes.len() <= 262144 + 4096);
 let mut schema_frame=value.clone();
 if value["type"]=="value" {
     assert!(serde_json::to_vec(&value["value"]).unwrap().len() <= 262144);
     schema_frame["value"]=Value::Null;
 }
 let schema:Value=serde_json::from_str(include_str!("../../contracts/http-v1.schema.json")).unwrap();
 let contract=json!({"schemaVersion":1,"$defs":schema["$defs"],"requests":{"test.frame":{"request":{"type":"object","additionalProperties":false},"response":{"$ref":"#/$defs/streamFrame"}}},"subscriptions":{}});
 teamarness_package_contract::ui_contract::compile_ui_contract(&serde_json::to_vec(&contract).unwrap()).unwrap().validate_response("test.frame",&schema_frame).unwrap();value}).await.unwrap()
}
#[tokio::test]
async fn confirmed_ndjson_preserves_arbitrary_values_and_normal_terminal() {
    let (_dir, h, script, port, bearer) = setup().await;
    let mut s = open(port, &bearer, "watch.item", json!({})).await;
    let head = headers(&mut s).await;
    assert!(head.starts_with("HTTP/1.1 200"), "{head}");
    assert!(head.contains("application/x-ndjson"));
    assert!(head.to_lowercase().contains("transfer-encoding: chunked"));
    assert_eq!(
        frame(&mut s).await,
        json!({"type":"ready","contract":"dev.test.reads","major":1,"port":"watch.item","kind":"query","delivery":"stream"})
    );
    for value in [
        json!(null),
        json!(false),
        json!(17),
        json!("text"),
        json!([1, null]),
        json!({"type":"end","reason":"completed"}),
        json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":null}),
    ] {
        assert!(
            script
                .stream()
                .send(json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":value}))
                .await
        );
        assert_eq!(frame(&mut s).await, json!({"type":"value","value":value}));
    }
    script.stream().close("completed").await;
    assert_eq!(
        frame(&mut s).await,
        json!({"type":"end","reason":"completed"})
    );
    let mut rest = String::new();
    s.read_to_string(&mut rest).await.unwrap();
    assert_eq!(rest, "0\r\n\r\n");
    let opens = script.recorded_opens();
    assert!(opens[0].get("method").is_none());
    assert_eq!(opens[0]["kind"], "query");
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn idle_exact_tool_disable_closes_stream_while_selected_read_remains() {
    let (_dir, mut h, script, port, bearer) = setup().await;
    let mut s = open(port, &bearer, "items.changed", Value::Null).await;
    assert!(headers(&mut s).await.starts_with("HTTP/1.1 200"));
    assert_eq!(frame(&mut s).await["kind"], "event");
    assert!(script.recorded_opens()[0].get("request").is_none());
    select_tool(&mut h, "items.changed", false).await;
    let mut rest = String::new();
    tokio::time::timeout(Duration::from_secs(2), s.read_to_string(&mut rest))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rest, "0\r\n\r\n");
    assert!(!script.stream().is_open());
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 200);
    let mut denied = open(port, &bearer, "items.changed", Value::Null).await;
    assert!(headers(&mut denied).await.starts_with("HTTP/1.1 403"));
    select_tool(&mut h, "items.changed", true).await;
    let mut fresh = open(port, &bearer, "items.changed", Value::Null).await;
    assert!(headers(&mut fresh).await.starts_with("HTTP/1.1 200"));
    assert_eq!(frame(&mut fresh).await["type"], "ready");
    drop(fresh);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn wrong_endpoint_and_event_request_refuse_before_sdk_open() {
    let (_dir, h, script, port, bearer) = setup().await;
    for (name, request) in [("read.item", json!({})), ("items.changed", json!({}))] {
        let mut s = open(port, &bearer, name, request).await;
        assert!(headers(&mut s).await.starts_with("HTTP/1.1 400"));
    }
    assert!(script.recorded_opens().is_empty());
    let body = json!({"contract":"dev.test.reads","major":1,"port":"items.changed","request":null});
    assert_eq!(post(port, &bearer, body).await.0, 400);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn eight_stream_slots_are_separate_from_calls_and_disconnect_frees_capacity() {
    let (_dir, h, script, port, bearer) = setup().await;
    let mut held = vec![];
    for _ in 0..8 {
        let mut s = open(port, &bearer, "items.changed", Value::Null).await;
        assert!(headers(&mut s).await.starts_with("HTTP/1.1 200"));
        assert_eq!(frame(&mut s).await["type"], "ready");
        held.push(s);
    }
    let mut ninth = open(port, &bearer, "items.changed", Value::Null).await;
    assert!(headers(&mut ninth).await.starts_with("HTTP/1.1 503"));
    assert_eq!(script.recorded_opens().len(), 8);
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 200);
    drop(held.pop());
    // SDK sender closure is a positive cleanup observation, not a scheduling delay.
    tokio::time::timeout(Duration::from_secs(2), async {
        while script.stream().is_open() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut fresh = open(port, &bearer, "items.changed", Value::Null).await;
    assert!(headers(&mut fresh).await.starts_with("HTTP/1.1 200"));
    assert_eq!(frame(&mut fresh).await["type"], "ready");
    drop(held);
    drop(fresh);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn sparse_stream_survives_whole_response_timeout_and_heartbeats_after_fifteen_seconds() {
    let (_dir, h, script, port, bearer) = setup().await;
    let mut s = open(port, &bearer, "items.changed", Value::Null).await;
    assert!(headers(&mut s).await.starts_with("HTTP/1.1 200"));
    assert_eq!(frame(&mut s).await["type"], "ready");
    let began = tokio::time::Instant::now();
    assert_eq!(frame(&mut s).await, json!({"type":"heartbeat"}));
    assert!(began.elapsed() >= Duration::from_secs(14));
    assert!(
        script
            .stream()
            .send(json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":{}}))
            .await
    );
    assert_eq!(frame(&mut s).await, json!({"type":"value","value":{}}));
    script.stream().close("provider_exited").await;
    let end = frame(&mut s).await;
    assert_eq!(end["type"], "end");
    assert_eq!(end["reason"], "error");
    assert_eq!(end["error"]["code"], "provider_exited");
    drop(s);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn idle_revoke_regenerate_package_listener_and_suspend_close_without_waiting_for_values() {
    for action in [
        "revoke",
        "regenerate",
        "package",
        "listener",
        "suspend",
        "dispose",
    ] {
        let (_dir, mut h, script, port, bearer) = setup().await;
        let other = h
            .request("external-gateway.token.create", json!({"label":"other"}))
            .await
            .unwrap();
        let mut s = open(port, &bearer, "items.changed", Value::Null).await;
        assert!(headers(&mut s).await.starts_with("HTTP/1.1 200"));
        assert_eq!(frame(&mut s).await["type"], "ready");
        let id = h
            .request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["tokens"][0]["id"]
            .clone();
        match action {
            "revoke" => {
                h.request("external-gateway.token.revoke", json!({"id":id}))
                    .await
                    .unwrap();
            }
            "regenerate" => {
                h.request("external-gateway.token.regenerate", json!({"id":id}))
                    .await
                    .unwrap();
            }
            "package" => {
                h.request(
                    "external-gateway.package.set",
                    json!({"provider":"dev.test.provider","enabled":false,"contracts":[]}),
                )
                .await
                .unwrap();
            }
            "listener" => {
                h.request("external-gateway.listener.set", json!({"enabled":false}))
                    .await
                    .unwrap();
            }
            "suspend" => {
                h.suspend().await.unwrap();
            }
            "dispose" => {
                h.dispose().await.unwrap();
            }
            _ => unreachable!(),
        }
        let mut rest = String::new();
        tokio::time::timeout(Duration::from_secs(2), s.read_to_string(&mut rest))
            .await
            .unwrap()
            .unwrap();
        assert!(!rest.contains("\"type\":\"value\""), "{action}");
        assert!(!script.stream().is_open());
        if matches!(action, "revoke" | "regenerate") {
            assert_eq!(
                post(port, other["bearer"].as_str().unwrap(), call(json!({})))
                    .await
                    .0,
                200
            );
            assert_eq!(post(port, &bearer, call(json!({}))).await.0, 401);
        }
        h.shutdown().await.unwrap();
    }
}

#[path = "http_support/duplex.rs"]
mod duplex;
#[tokio::test]
async fn held_confirmed_open_has_no_http_success_until_ready_and_refusals_stay_json() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = duplex::Fixture::start(
        dir.path(),
        Arc::new(Mutex::new(Provider::new())),
        true,
        None,
    )
    .await;
    f.select("items.changed", true).await;
    let (port, bearer) = f.enable().await;
    let mut s = open(port, &bearer, "items.changed", Value::Null).await;
    let (id, payload) = f.opens.recv().await.unwrap();
    assert_eq!(payload["kind"], "event");
    assert!(payload.get("request").is_none());
    let mut byte = [0];
    assert!(
        tokio::time::timeout(Duration::from_millis(50), s.read(&mut byte))
            .await
            .is_err(),
        "no HTTP bytes before confirmation"
    );
    f.peer.ready(&id).await;
    assert!(headers(&mut s).await.starts_with("HTTP/1.1 200"));
    assert_eq!(frame(&mut s).await["type"], "ready");
    assert!(f.peer.send(&id, json!({"after":"ready"})).await);
    assert_eq!(
        frame(&mut s).await,
        json!({"type":"value","value":{"after":"ready"}})
    );
    drop(s);
    f.peer.closed(&id).await;
    let mut refused = open(port, &bearer, "items.changed", Value::Null).await;
    let (id, _) = f.opens.recv().await.unwrap();
    f.peer.close(&id, "fixture.setup_refused").await;
    let head = headers(&mut refused).await;
    assert!(head.starts_with("HTTP/1.1 503"));
    assert!(head.contains("application/json"));
    let mut body = String::new();
    refused.read_to_string(&mut body).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["error"]["code"],
        "fixture.setup_refused"
    );
    assert!(!body.contains("\"ready\""));
    f.dispose().await;
}
#[tokio::test]
async fn confirmed_opening_refusals_attribute_provider_host_sdk_and_gateway_without_ready() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let dir = tempfile::tempdir().unwrap();
        let mut f = duplex::Fixture::start(
            dir.path(),
            Arc::new(Mutex::new(Provider::new())),
            true,
            None,
        )
        .await;
        f.select("watch.item", true).await;
        f.select("items.changed", true).await;
        let (port, bearer) = f.enable().await;
        let mut mismatches = Vec::new();
        for name in ["watch.item", "items.changed"] {
            for (code, source, status, retryable) in [
                ("fixture.setup_refused", "provider", 503, false),
                ("provider_exited", "host", 503, false),
                ("port_not_enabled", "host", 503, false),
                ("overloaded", "host", 503, true),
                ("cancelled", "host", 503, false),
                ("package.unknown_subscription", "host", 503, false),
                ("external_gateway.too_large", "gateway", 413, false),
                ("deadline_exceeded", "host", 504, true),
                ("port_catalog_changed", "host", 409, false),
            ] {
                let mut s = open(
                    port,
                    &bearer,
                    name,
                    if name == "watch.item" {
                        json!({})
                    } else {
                        Value::Null
                    },
                )
                .await;
                let (id, _) = f.opens.recv().await.unwrap();
                let mut byte = [0];
                assert!(
                    tokio::time::timeout(Duration::from_millis(25), s.read(&mut byte))
                        .await
                        .is_err(),
                    "no HTTP bytes before refusal: {name}: {code}"
                );
                f.peer.close(&id, code).await;
                let head = headers(&mut s).await;
                assert!(head.starts_with(&format!("HTTP/1.1 {status}")), "{head}");
                assert!(head.contains("application/json"));
                let mut body = String::new();
                s.read_to_string(&mut body).await.unwrap();
                assert!(!body.contains("\"ready\""));
                let value: Value = serde_json::from_str(&body).unwrap();
                assert_eq!(value["error"]["code"], code);
                assert_eq!(value["error"]["retryable"], retryable);
                assert_eq!(value["error"]["outcomeUncertain"], false);
                assert_eq!(value["error"]["details"], json!({}));
                if value["error"]["source"] != source {
                    mismatches.push(format!(
                        "{name}: {code} expected {source}, received {}",
                        value["error"]["source"]
                    ));
                }
            }
        }
        f.dispose().await;
        assert!(
            mismatches.is_empty(),
            "ownership mismatches: {mismatches:?}"
        );
    })
    .await
    .unwrap();
}
#[tokio::test]
async fn abandoned_open_keeps_its_slot_until_late_confirmation_then_closes() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = duplex::Fixture::start(
        dir.path(),
        Arc::new(Mutex::new(Provider::new())),
        true,
        None,
    )
    .await;
    f.select("items.changed", true).await;
    f.select("read.item", true).await;
    let (port, bearer) = f.enable().await;
    let mut pending = vec![];
    let mut ids = vec![];
    for _ in 0..8 {
        let policy = f.policy.clone();
        let token = bearer.clone();
        pending.push(tokio::spawn(async move {
            policy
                .subscribe(
                    token,
                    "dev.test.reads".into(),
                    1,
                    "items.changed".into(),
                    Value::Null,
                )
                .await
        }));
        ids.push(f.opens.recv().await.unwrap().0);
    }
    pending.remove(0).abort();
    assert!(
        matches!(f.policy.subscribe(bearer.clone(),"dev.test.reads".into(),1,"items.changed".into(),Value::Null).await,Err(e) if e.code()=="external_gateway.busy")
    );
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 200);
    f.peer.ready(&ids[0]).await;
    f.peer.closed(&ids[0]).await;
    let policy = f.policy.clone();
    let token = bearer.clone();
    let fresh = tokio::spawn(async move {
        policy
            .subscribe(
                token,
                "dev.test.reads".into(),
                1,
                "items.changed".into(),
                Value::Null,
            )
            .await
    });
    let (new_id, _) = f.opens.recv().await.unwrap();
    f.select("items.changed", false).await;
    for id in ids.iter().skip(1).chain([&new_id]) {
        f.peer.ready(id).await;
        f.peer.closed(id).await;
    }
    for task in pending {
        assert!(task.await.unwrap().is_err());
    }
    assert!(fresh.await.unwrap().is_err());
    f.dispose().await;
}
#[tokio::test]
async fn idle_revocation_leaves_other_token_stream_live_and_generation_loss_requires_fresh_open() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = duplex::Fixture::start(
        dir.path(),
        Arc::new(Mutex::new(Provider::new())),
        false,
        None,
    )
    .await;
    f.select("items.changed", true).await;
    let (port, bearer) = f.enable().await;
    let other = f
        .peer
        .request("external-gateway.token.create", json!({"label":"other"}))
        .await
        .unwrap();
    let id = f.policy.status()["tokens"][0]["id"].clone();
    let mut a = open(port, &bearer, "items.changed", Value::Null).await;
    let (aid, _) = f.opens.recv().await.unwrap();
    headers(&mut a).await;
    frame(&mut a).await;
    let mut b = open(
        port,
        other["bearer"].as_str().unwrap(),
        "items.changed",
        Value::Null,
    )
    .await;
    let (bid, _) = f.opens.recv().await.unwrap();
    headers(&mut b).await;
    frame(&mut b).await;
    f.peer
        .request("external-gateway.token.revoke", json!({"id":id}))
        .await
        .unwrap();
    f.peer.closed(&aid).await;
    assert!(f.peer.send(&bid, json!({"unrelated":true})).await);
    assert_eq!(frame(&mut b).await["value"], json!({"unrelated":true}));
    f.peer.close(&bid, "provider_exited").await;
    assert_eq!(frame(&mut b).await["error"]["code"], "provider_exited");
    let mut rest = String::new();
    b.read_to_string(&mut rest).await.unwrap();
    assert_eq!(rest, "0\r\n\r\n");
    let mut fresh = open(
        port,
        other["bearer"].as_str().unwrap(),
        "items.changed",
        Value::Null,
    )
    .await;
    let (cid, _) = f.opens.recv().await.unwrap();
    headers(&mut fresh).await;
    assert_eq!(frame(&mut fresh).await["type"], "ready");
    assert_ne!(cid, bid);
    drop(fresh);
    f.peer.closed(&cid).await;
    f.dispose().await;
}

#[tokio::test]
async fn stopped_reader_times_out_bounded_credit_without_blocking_another_stream() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let mut f = duplex::Fixture::start(
        dir.path(),
        Arc::new(Mutex::new(Provider::new())),
        false,
        None,
    )
    .await;
    f.select("items.changed", true).await;
    let (port, bearer) = f.enable().await;
    let mut slow = open(port, &bearer, "items.changed", Value::Null).await;
    let (sid, _) = f.opens.recv().await.unwrap();
    headers(&mut slow).await;
    frame(&mut slow).await;
    let sent = Arc::new(AtomicUsize::new(0));
    let count = sent.clone();
    let peer = f.peer.clone();
    let stream = sid.clone();
    let producer = tokio::spawn(async move {
        for _ in 0..1024 {
            if !peer.send(&stream, json!("x".repeat(262142))).await {
                return;
            }
            count.fetch_add(1, Ordering::SeqCst);
        }
        panic!("stopped reader accepted 256 MiB");
    });
    tokio::time::timeout(Duration::from_secs(4), async {
        while f.peer.remaining_credit(&sid) > 0 || sent.load(Ordering::SeqCst) < 32 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut other = open(port, &bearer, "items.changed", Value::Null).await;
    let (oid, _) = f.opens.recv().await.unwrap();
    assert!(headers(&mut other).await.starts_with("HTTP/1.1 200"));
    assert_eq!(frame(&mut other).await["type"], "ready");
    assert!(f.peer.send(&oid, json!({"peer":"works"})).await);
    assert_eq!(frame(&mut other).await["value"], json!({"peer":"works"}));
    // Positive SDK close is caused by the five-second blocked socket deadline, not idle time.
    f.peer.closed(&sid).await;
    tokio::time::timeout(Duration::from_secs(2), producer)
        .await
        .unwrap()
        .unwrap();
    assert!(sent.load(Ordering::SeqCst) < 1024);
    assert!(f.peer.is_open(&oid));
    drop(other);
    drop(slow);
    f.dispose().await;
}

#[tokio::test]
async fn failed_durable_write_closes_all_idle_streams_at_each_real_file_boundary() {
    use teamarness_generated_package_backend::config::{StorageFault, WriteStage};
    struct Fault(Mutex<Option<WriteStage>>);
    impl StorageFault for Fault {
        fn before(&self, s: WriteStage) -> std::io::Result<()> {
            let mut armed = self.0.lock().unwrap();
            if *armed == Some(s) {
                *armed = None;
                Err(std::io::Error::other("injected file failure"))
            } else {
                Ok(())
            }
        }
    }
    for stage in [
        WriteStage::Write,
        WriteStage::FileSync,
        WriteStage::Rename,
        WriteStage::DirectorySync,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let fault = Arc::new(Fault(Mutex::new(None)));
        let mut f = duplex::Fixture::start(
            dir.path(),
            Arc::new(Mutex::new(Provider::new())),
            false,
            Some(fault.clone()),
        )
        .await;
        f.select("items.changed", true).await;
        let (port, bearer) = f.enable().await;
        let mut sockets = vec![];
        let mut ids = vec![];
        for _ in 0..2 {
            let mut s = open(port, &bearer, "items.changed", Value::Null).await;
            ids.push(f.opens.recv().await.unwrap().0);
            headers(&mut s).await;
            frame(&mut s).await;
            sockets.push(s);
        }
        *fault.0.lock().unwrap() = Some(stage);
        assert_eq!(
            f.peer
                .request("external-gateway.token.create", json!({"label":"fails"}))
                .await
                .unwrap_err()["code"],
            "external_gateway.storage_failed"
        );
        for (mut s, id) in sockets.into_iter().zip(ids) {
            f.peer.closed(&id).await;
            let mut rest = String::new();
            s.read_to_string(&mut rest).await.unwrap();
            assert!(!rest.contains("\"type\":\"value\""));
        }
        assert_eq!(post(port, &bearer, call(json!({}))).await.0, 401);
        f.dispose().await;
    }
}
#[tokio::test]
async fn membership_invalidation_and_sdk_generation_interruption_retire_idle_streams() {
    for membership in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let mut f = duplex::Fixture::start(
            dir.path(),
            Arc::new(Mutex::new(Provider::new())),
            false,
            None,
        )
        .await;
        f.select("items.changed", true).await;
        let (port, bearer) = f.enable().await;
        let mut s = open(port, &bearer, "items.changed", Value::Null).await;
        let (id, _) = f.opens.recv().await.unwrap();
        headers(&mut s).await;
        frame(&mut s).await;
        if membership {
            f.peer
                .present
                .store(false, std::sync::atomic::Ordering::SeqCst);
            let mid = f.peer.membership.lock().unwrap().clone().unwrap();
            assert!(f.peer.send_payload(&mid,json!({"epoch":"2","membershipVersion":"a".repeat(64),"cause":"c".repeat(64),"snapshotDigest":"d".repeat(64)})).await);
            f.peer.closed(&id).await;
        } else {
            f.interrupt().await;
        }
        let mut rest = String::new();
        let _ = tokio::time::timeout(Duration::from_secs(7), s.read_to_string(&mut rest))
            .await
            .expect(if membership {
                "membership EOF"
            } else {
                "generation EOF"
            });
        assert!(!rest.contains("\"type\":\"value\""));
        if !membership {
            assert!(
                !rest.contains("\"type\":\"end\""),
                "abrupt EOF is interruption, not completion"
            );
        }
        if membership {
            f.dispose().await;
        }
    }
}
#[tokio::test]
async fn selected_process_one_zero_refuses_before_http_success_without_fake_ready() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p, dir.path()).await;
    select_tool(&mut h, "items.changed", true).await;
    let (port, bearer) = enable(&mut h).await;
    let mut s = open(port, &bearer, "items.changed", Value::Null).await;
    assert!(headers(&mut s).await.starts_with("HTTP/1.1 503"));
    let mut body = String::new();
    s.read_to_string(&mut body).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&body).unwrap()["error"]["code"],
        "capability_version_unsupported"
    );
    assert!(!body.contains("\"ready\""));
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_confirmation_sdk_terminal_releases_all_pending_slots_and_ignores_late_ready() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = duplex::Fixture::start(
        dir.path(),
        Arc::new(Mutex::new(Provider::new())),
        true,
        None,
    )
    .await;
    f.select("items.changed", true).await;
    let (_, bearer) = f.enable().await;
    let mut pending = tokio::task::JoinSet::new();
    let mut ids = vec![];
    for _ in 0..8 {
        let policy = f.policy.clone();
        let token = bearer.clone();
        pending.spawn(async move {
            policy
                .subscribe(
                    token,
                    "dev.test.reads".into(),
                    1,
                    "items.changed".into(),
                    Value::Null,
                )
                .await
        });
        ids.push(f.opens.recv().await.unwrap().0);
    }
    tokio::time::timeout(Duration::from_secs(23), async {
        while let Some(result) = pending.join_next().await {
            assert!(matches!(result.unwrap(),Err(e) if e.code()=="deadline_exceeded"));
        }
    })
    .await
    .unwrap();
    for id in &ids {
        f.peer.closed(id).await;
        f.peer.ready(id).await;
    }
    f.peer
        .hold
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let output = f
        .policy
        .subscribe(
            bearer,
            "dev.test.reads".into(),
            1,
            "items.changed".into(),
            Value::Null,
        )
        .await
        .unwrap();
    drop(output);
    f.dispose().await;
}

#[tokio::test]
async fn stream_values_keep_exact_original_budget_and_oversized_values_end_without_partial_delivery()
 {
    let (_dir, h, script, port, bearer) = setup().await;
    let mut s = open(port, &bearer, "watch.item", json!({})).await;
    assert!(headers(&mut s).await.starts_with("HTTP/1.1 200"));
    frame(&mut s).await;
    let value = json!("x".repeat(262142));
    assert_eq!(serde_json::to_vec(&value).unwrap().len(), 262144);
    assert!(
        script
            .stream()
            .send(json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":value}))
            .await
    );
    assert_eq!(frame(&mut s).await, json!({"type":"value","value":value}));
    assert!(
        script
            .stream()
            .send(json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":"x".repeat(262143)}))
            .await
    );
    let end = frame(&mut s).await;
    assert_eq!(end["type"], "end");
    assert_eq!(end["error"]["code"], "external_gateway.too_large");
    assert!(end.get("value").is_none());
    let mut rest = String::new();
    s.read_to_string(&mut rest).await.unwrap();
    assert_eq!(rest, "0\r\n\r\n");
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_token_unselected_tool_and_caller_authority_fields_never_open_sdk_streams() {
    let (_dir, mut h, script, port, bearer) = setup().await;
    let mut invalid = open(port, "invalid", "items.changed", Value::Null).await;
    assert!(headers(&mut invalid).await.starts_with("HTTP/1.1 401"));
    select_tool(&mut h, "items.changed", false).await;
    let mut denied = open(port, &bearer, "items.changed", Value::Null).await;
    assert!(headers(&mut denied).await.starts_with("HTTP/1.1 403"));
    for field in ["actor", "workspace", "provider", "kind"] {
        let mut body =
            json!({"contract":"dev.test.reads","major":1,"port":"watch.item","request":{}});
        body[field] = json!("forged");
        let body = body.to_string();
        let wire = format!(
            "POST /v1/subscribe HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        assert_eq!(response(&raw(port, wire.as_bytes()).await).0, 400);
    }
    assert!(script.recorded_opens().is_empty());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn lost_opening_socket_late_success_closes_instead_of_leaking_or_reviving() {
    let dir = tempfile::tempdir().unwrap();
    let mut f = duplex::Fixture::start(
        dir.path(),
        Arc::new(Mutex::new(Provider::new())),
        true,
        None,
    )
    .await;
    f.select("items.changed", true).await;
    let (port, bearer) = f.enable().await;
    let s = open(port, &bearer, "items.changed", Value::Null).await;
    let (id, _) = f.opens.recv().await.unwrap();
    drop(s);
    f.peer.ready(&id).await;
    f.peer.closed(&id).await;
    f.peer
        .hold
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let mut fresh = open(port, &bearer, "items.changed", Value::Null).await;
    let (new_id, _) = f.opens.recv().await.unwrap();
    assert!(headers(&mut fresh).await.starts_with("HTTP/1.1 200"));
    assert_eq!(frame(&mut fresh).await["type"], "ready");
    assert_ne!(id, new_id);
    drop(fresh);
    f.dispose().await;
}

#[tokio::test]
async fn stream_terminals_attribute_provider_host_sdk_and_gateway_without_invented_details() {
    for name in ["watch.item", "items.changed"] {
        for (code, source) in [
            ("proof.stream_failed", "provider"),
            ("provider_exited", "host"),
            ("port_not_enabled", "host"),
            ("overloaded", "host"),
            ("cancelled", "host"),
            ("host_unavailable", "host"),
            ("package.unknown_subscription", "host"),
            ("external_gateway.too_large", "gateway"),
        ] {
            let (_dir, h, script, port, bearer) = setup().await;
            let mut s = open(
                port,
                &bearer,
                name,
                if name == "watch.item" {
                    json!({})
                } else {
                    Value::Null
                },
            )
            .await;
            assert!(headers(&mut s).await.starts_with("HTTP/1.1 200"));
            assert_eq!(frame(&mut s).await["type"], "ready");
            assert!(script.stream().send(json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":{"source":"host","code":"provider_exited"}})).await);
            assert_eq!(
                frame(&mut s).await,
                json!({"type":"value","value":{"source":"host","code":"provider_exited"}})
            );
            script.stream().close(code).await;
            let end = frame(&mut s).await;
            assert_eq!(end["reason"], "error");
            assert_eq!(end["error"]["source"], source, "{name}: {code}");
            assert_eq!(end["error"]["code"], code);
            assert_eq!(end["error"]["details"], json!({}));
            assert_eq!(end["error"]["outcomeUncertain"], false);
            if source == "provider" {
                assert_eq!(end["error"]["retryable"], false);
            }
            drop(s);
            h.shutdown().await.unwrap();
        }
    }
}
