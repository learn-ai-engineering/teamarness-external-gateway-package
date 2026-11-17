//! Verifies: TEAM.SYS.PKG-R02, R05, R09. Real IPv4 sockets and SDK lifecycle.
use http_support::backend_environment;
mod http_support;
use http_support::*;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::net::TcpStream;
#[tokio::test]
async fn saved_port_is_durable_before_publication_and_resumes_without_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let mut h = start(p.clone(), dir.path()).await;
    let (port, bearer) = enable(&mut h).await;
    let saved: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("config/state.json")).unwrap())
            .unwrap();
    assert_eq!(saved["port"], port);
    assert_eq!(saved["desiredEnabled"], true);
    assert!(TcpStream::connect(("::1", port)).await.is_err());
    h.suspend().await.unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.recover().await.unwrap();
    assert_eq!(get(port, &bearer, "/v1").await.0, 200);
    h.dispose().await.unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.shutdown().await.unwrap();
    let conflict = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap();
    let mut h = start(p.clone(), dir.path()).await;
    let s = h
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(s["running"], false);
    assert_eq!(s["desiredEnabled"], true);
    assert_eq!(s["port"], port);
    assert_eq!(s["warning"], "listener_bind_failed");
    drop(conflict);
    h.suspend().await.unwrap();
    h.recover().await.unwrap();
    assert_eq!(get(port, &bearer, "/v1").await.0, 200);
    h.dispose().await.unwrap();
    h.shutdown().await.unwrap();
    let mut h = start(p, dir.path()).await;
    assert_eq!(get(port, &bearer, "/v1").await.0, 200);
    h.request("external-gateway.disable-and-revoke-all", json!({}))
        .await
        .unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn explicit_port_change_and_workspace_tokens_are_isolated() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut h = start(Arc::new(Mutex::new(Provider::new())), a.path()).await;
    let mut other = start(Arc::new(Mutex::new(Provider::new())), b.path()).await;
    let (old, bearer) = enable(&mut h).await;
    let (second, token) = enable(&mut other).await;
    assert_ne!(old, second);
    assert_eq!(get(old, &token, "/v1").await.0, 401);
    assert_eq!(get(second, &bearer, "/v1").await.0, 401);
    let socket = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let new = socket.local_addr().unwrap().port();
    drop(socket);
    h.request("external-gateway.port.set", json!({"port":new}))
        .await
        .unwrap();
    assert!(TcpStream::connect(("127.0.0.1", old)).await.is_err());
    assert_eq!(get(new, &bearer, "/v1").await.0, 200);
    h.dispose().await.unwrap();
    other.dispose().await.unwrap();
    h.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
}
#[tokio::test]
async fn failed_port_commit_closes_provisional_socket_and_never_publishes_it() {
    use teamarness_generated_package_backend::{
        config::{StorageFault, WriteStage},
        tokens::OsEntropy,
    };
    struct Fault(std::sync::atomic::AtomicBool);
    impl StorageFault for Fault {
        fn before(&self, s: WriteStage) -> std::io::Result<()> {
            if s == WriteStage::Write && self.0.swap(false, std::sync::atomic::Ordering::SeqCst) {
                Err(std::io::Error::other("fault"))
            } else {
                Ok(())
            }
        }
    }
    let d = tempfile::tempdir().unwrap();
    let fault = Arc::new(Fault(false.into()));
    let mut h = start_router(
        Arc::new(Mutex::new(Provider::new())),
        d.path(),
        backend_environment::router_with_storage_fault(Arc::new(OsEntropy), Some(fault.clone())),
    )
    .await;
    let (old, _) = enable(&mut h).await;
    let socket = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let new = socket.local_addr().unwrap().port();
    drop(socket);
    fault.0.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        h.request("external-gateway.port.set", json!({"port":new}))
            .await
            .is_err()
    );
    let s = h
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(s["running"], false);
    assert_eq!(s["port"], old);
    assert!(TcpStream::connect(("127.0.0.1", new)).await.is_err());
    let _free = tokio::net::TcpListener::bind(("127.0.0.1", new))
        .await
        .unwrap();
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn shutdown_and_suspend_end_incomplete_connections_within_drain_budget() {
    use tokio::io::AsyncReadExt;
    for shutdown in [false, true] {
        let d = tempfile::tempdir().unwrap();
        let mut h = start(Arc::new(Mutex::new(Provider::new())), d.path()).await;
        let (port, _) = enable(&mut h).await;
        let mut conn = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        if shutdown {
            h.shutdown().await.unwrap();
        } else {
            h.suspend().await.unwrap();
            h.dispose().await.unwrap();
            h.shutdown().await.unwrap();
        }
        let mut buf = [0; 1];
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(6), conn.read(&mut buf))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bounded_suspend_retains_pending_sdk_slot_across_same_state_recovery() {
    let d = tempfile::tempdir().unwrap();
    let p = Arc::new(Mutex::new(Provider::new()));
    let armed = Arc::new(Mutex::new(None));
    let (mut h, policy) = scripted(p.clone(), d.path(), armed.clone()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let (barrier, release) = pause();
    *armed.lock().unwrap() = Some(("invoke".into(), barrier.clone()));
    let token = bearer.clone();
    let lost = tokio::spawn(async move { post(port, &token, call(json!({}))).await });
    barrier.entered.notified().await;
    lost.abort();
    let _ = lost.await;
    // Drive the same lifecycle owner the SDK hooks use. The synchronous scripted capability
    // pump is paused, so a second harness control reply cannot be delivered until released.
    policy.suspend().await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    policy.resume().await;
    let mut reads = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let token = bearer.clone();
        reads.spawn(async move { post(port, &token, call(json!({}))).await });
    }
    let busy = tokio::time::timeout(std::time::Duration::from_secs(2), reads.join_next())
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
    h.dispose().await.unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.shutdown().await.unwrap();
}
