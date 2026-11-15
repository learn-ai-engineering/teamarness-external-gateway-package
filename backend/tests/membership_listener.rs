//! Verifies: TEAM.SYS.PKG-R02, R05, R09, R13. Membership signals stop idle sockets.
mod http_support;
use http_support::*;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use teamarness_generated_package_backend::router;
use teamarness_package_sdk::testing::{HarnessBuilder, ScriptedCapability};
use tokio::{io::AsyncReadExt, net::TcpStream};

fn snapshot(present: bool, epoch: u64) -> Value {
    json!({"workspaceId":"workspace1","epoch":epoch.to_string(),"membershipVersion":"a".repeat(64),"members":if present {vec![json!({"identity":"b".repeat(64),"role":"member"})]} else {vec![]}})
}
fn event(epoch: u64) -> Value {
    json!({"epoch":epoch.to_string(),"membershipVersion":"a".repeat(64),"cause":"c".repeat(64),"snapshotDigest":"d".repeat(64)})
}
#[tokio::test]
async fn membership_loss_stops_idle_listener_and_old_connections_without_requests() {
    let dir = tempfile::tempdir().unwrap();
    let roster = Arc::new(Mutex::new(snapshot(true, 1)));
    let current = roster.clone();
    let membership = ScriptedCapability::new().calls(move |method, _| {
        assert_eq!(method, "current");
        Ok(current.lock().unwrap().clone())
    });
    let stream = membership.stream();
    let p = Arc::new(Mutex::new(Provider::new()));
    let provider = p.clone();
    let mut h = tokio::time::timeout(
        Duration::from_secs(3),
        HarnessBuilder::new(router().into_backend())
            .data_directory(dir.path().canonicalize().unwrap())
            .installation(&"b".repeat(64), "Local")
            .capability("teamarness.membership", membership)
            .capability_version(
                "teamarness.package-ports",
                ScriptedCapability::new().calls(move |m, p| provider.lock().unwrap().call(m, p)),
                3,
                0,
            )
            .start(),
    )
    .await
    .expect("membership fixture handshake");
    tokio::time::timeout(Duration::from_secs(3), h.activate())
        .await
        .expect("activation")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), select(&mut h))
        .await
        .expect("selection");
    let (port, bearer) = tokio::time::timeout(Duration::from_secs(3), enable(&mut h))
        .await
        .expect("enable");
    assert_eq!(
        get(port, &bearer, "/v1/contracts/dev.test.reads/1").await.0,
        200
    );
    let mut old = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut status_events = h
        .subscribe("external-gateway.status.watch", json!({}))
        .await
        .unwrap();
    status_events.next_event().await.unwrap().unwrap();
    *roster.lock().unwrap() = snapshot(false, 2);
    let signal = tokio::spawn(async move { stream.send(event(2)).await });
    // A passive pre-existing status subscription observes completion; no request drives it.
    let stopped = tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            let state = status_events.next_event().await.unwrap().unwrap();
            if state["running"] == false && state["warning"] == "membership_removed" {
                break;
            }
        }
    })
    .await;
    let reachable = TcpStream::connect(("127.0.0.1", port)).await.is_ok();
    if stopped.is_err() {
        signal.abort();
    }
    assert!(
        !reachable,
        "listener remains reachable after membership loss (watcher/event delivery absent)"
    );
    stopped.expect("membership stop completion");
    assert!(signal.await.unwrap());
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(6), old.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    let status = h
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(status["desiredEnabled"], true);
    assert_eq!(status["port"], port);
    assert_eq!(status["warning"], "membership_removed");
    assert_eq!(p.lock().unwrap().invocations, 0);
    h.shutdown().await.unwrap();
}

use teamarness_generated_package_backend::{State, policy::Policy};
use teamarness_package_sdk::{BackendError, Context, Empty, testing::Harness};

struct Fixture {
    roster: Arc<Mutex<Result<Value, BackendError>>>,
    membership: ScriptedCapability,
    provider: Arc<Mutex<Provider>>,
}
impl Fixture {
    fn new() -> Self {
        let roster = Arc::new(Mutex::new(Ok(snapshot(true, 1))));
        let current = roster.clone();
        Self {
            roster,
            membership: ScriptedCapability::new().calls(move |m, _| {
                assert_eq!(m, "current");
                current.lock().unwrap().clone()
            }),
            provider: Arc::new(Mutex::new(Provider::new())),
        }
    }
    async fn start(&self, dir: &std::path::Path) -> (Harness, Arc<Policy>) {
        self.start_with(dir, self.membership.clone()).await
    }
    async fn start_with(
        &self,
        dir: &std::path::Path,
        membership: ScriptedCapability,
    ) -> (Harness, Arc<Policy>) {
        let (send, recv) = tokio::sync::oneshot::channel();
        let send = Arc::new(Mutex::new(Some(send)));
        let r = router().request("test.capture", move |ctx: Context<State>, _: Empty| {
            let send = send.clone();
            async move {
                let _ = send
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(ctx.state().policy.clone());
                Ok(json!({}))
            }
        });
        let provider = self.provider.clone();
        let mut h = HarnessBuilder::new(r.into_backend())
            .data_directory(dir.canonicalize().unwrap())
            .installation(&"b".repeat(64), "Local")
            .capability("teamarness.membership", membership)
            .capability_version(
                "teamarness.package-ports",
                ScriptedCapability::new().calls(move |m, p| provider.lock().unwrap().call(m, p)),
                3,
                0,
            )
            .start()
            .await;
        h.activate().await.unwrap();
        h.request("test.capture", json!({})).await.unwrap();
        (h, recv.await.unwrap())
    }
    async fn notify(&self, present: bool, epoch: u64) {
        *self.roster.lock().unwrap() = Ok(snapshot(present, epoch));
        assert!(self.membership.stream().send(event(epoch)).await);
    }
}
async fn state(policy: &Policy, predicate: impl Fn(&Value) -> bool) -> Value {
    let mut status = policy.watch();
    tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            let value = status.borrow_and_update().clone();
            if predicate(&value) {
                return value;
            }
            status.changed().await.expect("status owner alive");
        }
    })
    .await
    .expect("membership status transition within drain budget")
}
async fn warning(policy: &Policy, text: &str) -> Value {
    state(policy, |s| s["running"] == false && s["warning"] == text).await
}

#[tokio::test]
async fn removed_membership_closes_every_route_and_preserves_setup_across_reactivation() {
    use tokio::io::AsyncWriteExt;
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let (mut h, policy) = f.start(dir.path()).await;
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let routes = [
        "/v1",
        "/v1/onboarding",
        "/v1/contracts",
        "/v1/contracts/dev.test.reads/1",
        "/v1/contracts/dev.test.reads/1/guide",
        "/v1/call",
    ];
    for route in &routes[..5] {
        assert_eq!(get(port, &bearer, route).await.0, 200);
    }
    assert_eq!(post(port, &bearer, call(json!({}))).await.0, 200);
    let before = std::fs::read(dir.path().join("config/state.json")).unwrap();
    let mut old = vec![];
    for route in routes {
        let mut conn = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        conn.write_all(format!("{} {route} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\n", if route=="/v1/call" {"POST"} else {"GET"}).as_bytes()).await.unwrap();
        old.push(conn);
    }
    f.notify(false, 2).await;
    warning(&policy, "membership_removed").await;
    for mut conn in old {
        let mut bytes = vec![];
        tokio::time::timeout(Duration::from_secs(1), conn.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
        assert!(bytes.is_empty(), "old route cannot return protected data");
    }
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    assert_eq!(
        std::fs::read(dir.path().join("config/state.json")).unwrap(),
        before
    );
    assert_eq!(f.provider.lock().unwrap().invocations, 2);
    // Even an inconsistent later roster cannot causally re-admit a removed identity.
    f.notify(true, 3).await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    h.request(
        "external-gateway.token.create",
        json!({"label":"unrelated mutation"}),
    )
    .await
    .unwrap();
    h.request("external-gateway.port.set", json!({"port":port}))
        .await
        .unwrap();
    h.suspend().await.unwrap();
    h.recover().await.unwrap();
    assert!(!policy.status()["running"].as_bool().unwrap());
    h.shutdown().await.unwrap();
    *f.roster.lock().unwrap() = Ok(snapshot(false, 4));
    // A new generation observes retained desired setup but absent canonical membership.
    let (mut h, policy) = f
        .start_with(dir.path(), {
            let current = f.roster.clone();
            ScriptedCapability::new().calls(move |_, _| current.lock().unwrap().clone())
        })
        .await;
    assert_eq!(policy.status()["warning"], "membership_removed");
    assert_eq!(policy.status()["desiredEnabled"], true);
    assert_eq!(policy.status()["port"], port);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.dispose().await.unwrap();
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_events_and_failed_current_fail_closed_but_live_stream_recovers_only_on_valid_notification()
 {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let (mut h, policy) = f.start(dir.path()).await;
    let (port, bearer) = enable(&mut h).await;
    for (n, invalid) in [json!({}), json!({"epoch":"2","membershipVersion":"bad","cause":"c".repeat(64),"snapshotDigest":"d".repeat(64)}), json!({"epoch":"03","membershipVersion":"a".repeat(64),"cause":"c".repeat(64),"snapshotDigest":"d".repeat(64)})].into_iter().enumerate() {
        assert!(f.membership.stream().send(invalid).await);
        warning(&policy, "membership_unavailable").await;
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        f.notify(true, 10+n as u64).await;
        state(&policy, |s| s["running"] == true).await;
        assert_eq!(get(port, &bearer, "/v1").await.0, 200);
    }
    *f.roster.lock().unwrap() = Err(BackendError::new("capability_unavailable", true, json!({})));
    assert!(f.membership.stream().send(event(20)).await);
    warning(&policy, "membership_unavailable").await;
    // Lifecycle/desired gates remain independent of later valid membership observations.
    h.request("external-gateway.listener.set", json!({"enabled":false}))
        .await
        .unwrap();
    f.notify(true, 21).await;
    state(&policy, |s| s["warning"].is_null()).await;
    assert_eq!(policy.status()["running"], false);
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    h.suspend().await.unwrap();
    assert!(f.membership.stream().send(json!({})).await);
    warning(&policy, "membership_unavailable").await;
    f.notify(true, 22).await;
    state(&policy, |s| s["warning"].is_null()).await;
    assert_eq!(policy.status()["running"], false);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.recover().await.unwrap();
    assert_eq!(get(port, &bearer, "/v1").await.0, 200);
    h.dispose().await.unwrap();
    assert!(!f.membership.stream().is_open());
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn terminated_stream_requires_explicit_recovery_and_keeps_saved_port_conflict_visible() {
    for code in ["completed", "capability_unavailable"] {
        let dir = tempfile::tempdir().unwrap();
        let f = Fixture::new();
        let (mut h, policy) = f.start(dir.path()).await;
        let (port, bearer) = enable(&mut h).await;
        f.membership.stream().close(code).await;
        warning(&policy, "membership_unavailable").await;
        h.request("external-gateway.listener.set", json!({"enabled":true}))
            .await
            .unwrap();
        h.request("external-gateway.port.set", json!({"port":port}))
            .await
            .unwrap();
        assert_eq!(policy.status()["running"], false);
        assert_eq!(f.membership.recorded_opens().len(), 1);
        let conflict = tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .unwrap();
        h.suspend().await.unwrap();
        h.recover().await.unwrap();
        assert_eq!(policy.status()["warning"], "listener_bind_failed");
        assert_eq!(policy.status()["port"], port);
        assert_eq!(f.membership.recorded_opens().len(), 2);
        drop(conflict);
        h.suspend().await.unwrap();
        h.recover().await.unwrap();
        assert_eq!(get(port, &bearer, "/v1").await.0, 200);
        // Stream close is observable before the harness tears down its Host-side pump.
        h.dispose().await.unwrap();
        assert!(!f.membership.stream().is_open());
        h.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn absent_malformed_wrong_workspace_and_failed_initial_snapshots_never_bind() {
    let mut wrong_workspace = snapshot(true, 1);
    wrong_workspace["workspaceId"] = "other-workspace".into();
    let mut wrong_identity = snapshot(true, 1);
    wrong_identity["members"][0]["identity"] = "bad".into();
    let mut wrong_role = snapshot(true, 1);
    wrong_role["members"][0]["role"] = "administrator".into();
    let mut duplicate = snapshot(true, 1);
    duplicate["members"]
        .as_array_mut()
        .unwrap()
        .push(json!({"identity":"b".repeat(64),"role":"manager"}));
    for response in [
        Ok(snapshot(false, 1)),
        Ok(json!({})),
        Ok(wrong_workspace),
        Ok(wrong_identity),
        Ok(wrong_role),
        Ok(duplicate),
        Err(BackendError::new("capability_unavailable", true, json!({}))),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let f = Fixture::new();
        let (mut h, _) = f.start(dir.path()).await;
        let (port, _) = enable(&mut h).await;
        h.shutdown().await.unwrap();
        let expected = if response.as_ref().is_ok_and(|v| v["members"] == json!([])) {
            "membership_removed"
        } else {
            "membership_unavailable"
        };
        let membership = ScriptedCapability::new().calls(move |_, _| response.clone());
        let (mut h, policy) = f.start_with(dir.path(), membership).await;
        assert_eq!(policy.status()["warning"], expected);
        assert_eq!(policy.status()["desiredEnabled"], true);
        assert_eq!(policy.status()["running"], false);
        assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
        h.request("external-gateway.listener.set", json!({"enabled":true}))
            .await
            .unwrap();
        assert_eq!(policy.status()["running"], false);
        h.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn initialization_revalidates_and_catches_notification_between_snapshot_and_publication() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let (mut h, _) = f.start(dir.path()).await;
    let (port, _) = enable(&mut h).await;
    h.shutdown().await.unwrap();
    let (barrier, release) = pause();
    let paused = barrier.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let membership =
        ScriptedCapability::new().calls(move |_, _| match count.fetch_add(1, Ordering::SeqCst) {
            0 => Ok(snapshot(true, 1)),
            1 => {
                paused.entered.notify_one();
                paused.wait();
                Ok(snapshot(true, 1))
            }
            _ => Ok(snapshot(false, 2)),
        });
    let stream = membership.stream();
    let retained = membership.clone();
    let path = dir.path().to_owned();
    let startup = tokio::spawn(async move { f.start_with(&path, membership).await });
    tokio::time::timeout(Duration::from_secs(3), barrier.entered.notified())
        .await
        .expect("second current reached after stream open");
    assert!(stream.is_open());
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    assert!(stream.send(event(2)).await);
    drop(release);
    let (mut h, policy) = tokio::time::timeout(Duration::from_secs(7), startup)
        .await
        .unwrap()
        .unwrap();
    warning(&policy, "membership_removed").await;
    assert_eq!(retained.recorded_opens()[0]["afterEpoch"], "1");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.dispose().await.unwrap();
    assert!(!stream.is_open());
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn initialization_second_snapshot_detects_loss_without_waiting_for_an_event() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let (mut h, _) = f.start(dir.path()).await;
    let (port, _) = enable(&mut h).await;
    h.shutdown().await.unwrap();
    let count = AtomicUsize::new(0);
    let membership = ScriptedCapability::new().calls(move |_, _| {
        Ok(if count.fetch_add(1, Ordering::SeqCst) == 0 {
            snapshot(true, 1)
        } else {
            snapshot(false, 2)
        })
    });
    let (h, policy) = f.start_with(dir.path(), membership).await;
    assert_eq!(policy.status()["warning"], "membership_removed");
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn membership_loss_retires_pending_http_observer_without_replaying_owned_sdk_call() {
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let (barrier, release) = pause();
    let paused = barrier.clone();
    let provider = f.provider.clone();
    let (send, recv) = tokio::sync::oneshot::channel();
    let send = Arc::new(Mutex::new(Some(send)));
    let r = router().request("test.capture", move |ctx: Context<State>, _: Empty| {
        let send = send.clone();
        async move {
            send.lock()
                .unwrap()
                .take()
                .unwrap()
                .send(ctx.state().policy.clone())
                .ok();
            Ok(json!({}))
        }
    });
    let mut h = HarnessBuilder::new(r.into_backend())
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", f.membership.clone())
        .capability_version(
            "teamarness.package-ports",
            ScriptedCapability::new().calls(move |m, p| {
                if m == "invoke" {
                    paused.entered.notify_one();
                    paused.wait();
                }
                provider.lock().unwrap().call(m, p)
            }),
            3,
            0,
        )
        .start()
        .await;
    h.activate().await.unwrap();
    h.request("test.capture", json!({})).await.unwrap();
    let policy = recv.await.unwrap();
    select(&mut h).await;
    let (port, bearer) = enable(&mut h).await;
    let read = tokio::spawn(async move { post(port, &bearer, call(json!({}))).await });
    tokio::time::timeout(Duration::from_secs(3), barrier.entered.notified())
        .await
        .unwrap();
    read.abort();
    let _ = read.await;
    f.notify(false, 2).await;
    warning(&policy, "membership_unavailable").await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    // Membership invalidates admission before waiting behind the pending SDK call.
    assert_eq!(f.provider.lock().unwrap().invocations, 0);
    drop(release);
    warning(&policy, "membership_removed").await;
    assert_eq!(f.provider.lock().unwrap().invocations, 1);
    h.suspend().await.unwrap();
    h.recover().await.unwrap();
    assert_eq!(f.provider.lock().unwrap().invocations, 1);
    assert_eq!(policy.status()["running"], false);
    h.dispose().await.unwrap();
    h.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_membership_snapshot_cannot_undo_manual_suspension() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let (barrier, release) = pause();
    let paused = barrier.clone();
    let count = AtomicUsize::new(0);
    let membership = ScriptedCapability::new().calls(move |_, _| {
        if count.fetch_add(1, Ordering::SeqCst) >= 2 {
            paused.entered.notify_one();
            paused.wait();
            Ok(snapshot(true, 2))
        } else {
            Ok(snapshot(true, 1))
        }
    });
    let stream = membership.stream();
    let (mut h, policy) = f.start_with(dir.path(), membership).await;
    let (port, bearer) = enable(&mut h).await;
    assert!(stream.send(event(2)).await);
    tokio::time::timeout(Duration::from_secs(3), barrier.entered.notified())
        .await
        .unwrap();
    policy.suspend().await;
    drop(release);
    state(&policy, |s| s["warning"].is_null()).await;
    assert_eq!(policy.status()["running"], false);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    // Only the explicit lifecycle operation, not the late snapshot, restores active state.
    h.recover().await.unwrap();
    assert_eq!(get(port, &bearer, "/v1").await.0, 200);
    h.dispose().await.unwrap();
    h.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn membership_invalidation_fences_listener_enable_pending_durable_publication() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use teamarness_generated_package_backend::{
        config::{StorageFault, WriteStage},
        router_with_storage_fault,
        tokens::OsEntropy,
    };
    struct Fault {
        armed: AtomicBool,
        barrier: Arc<Pause>,
    }
    impl StorageFault for Fault {
        fn before(&self, stage: WriteStage) -> std::io::Result<()> {
            if stage == WriteStage::FileSync && self.armed.swap(false, Ordering::SeqCst) {
                self.barrier.entered.notify_one();
                self.barrier.wait();
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let (barrier, release) = pause();
    let fault = Arc::new(Fault {
        armed: AtomicBool::new(false),
        barrier: barrier.clone(),
    });
    let (send, recv) = tokio::sync::oneshot::channel();
    let send = Arc::new(Mutex::new(Some(send)));
    let r = router_with_storage_fault(Arc::new(OsEntropy), Some(fault.clone())).request(
        "test.capture",
        move |ctx: Context<State>, _: Empty| {
            let send = send.clone();
            async move {
                send.lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(ctx.state().policy.clone())
                    .ok();
                Ok(json!({}))
            }
        },
    );
    let mut h = HarnessBuilder::new(r.into_backend())
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", f.membership.clone())
        .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    h.request("test.capture", json!({})).await.unwrap();
    let policy = recv.await.unwrap();
    let (port, _) = enable(&mut h).await;
    h.request("external-gateway.listener.set", json!({"enabled":false}))
        .await
        .unwrap();
    fault.armed.store(true, Ordering::SeqCst);
    let pending = h
        .start_request("external-gateway.listener.set", json!({"enabled":true}))
        .await;
    tokio::time::timeout(Duration::from_secs(3), barrier.entered.notified())
        .await
        .unwrap();
    f.notify(false, 2).await;
    // The visible unavailable warning confirms the watcher invalidated admission
    // while the durable transaction still owns the lock; no scheduler timing guess.
    state(&policy, |s| s["warning"] == "membership_unavailable").await;
    drop(release);
    pending.result().await.unwrap();
    warning(&policy, "membership_removed").await;
    assert_eq!(policy.status()["desiredEnabled"], true);
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    h.recover().await.unwrap();
    assert_eq!(policy.status()["running"], false);
    h.dispose().await.unwrap();
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn one_workspace_membership_loss_does_not_stop_another_workspace_listener() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let f = Fixture::new();
    let (mut h, policy) = f.start(a.path()).await;
    let (port, _) = enable(&mut h).await;
    let mut roster = snapshot(true, 1);
    roster["workspaceId"] = "workspace2".into();
    let mut other = HarnessBuilder::new(router().into_backend())
        .data_directory(b.path().canonicalize().unwrap())
        .workspace("workspace2", "Other Workspace")
        .installation(&"b".repeat(64), "Local")
        .capability(
            "teamarness.membership",
            ScriptedCapability::new().calls(move |_, _| Ok(roster.clone())),
        )
        .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
        .start()
        .await;
    other.activate().await.unwrap();
    let (other_port, bearer) = enable(&mut other).await;
    f.notify(false, 2).await;
    warning(&policy, "membership_removed").await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    let (code, value) = get(other_port, &bearer, "/v1").await;
    assert_eq!(code, 200);
    assert_eq!(value["workspace"]["id"], "workspace2");
    h.dispose().await.unwrap();
    other.dispose().await.unwrap();
    h.shutdown().await.unwrap();
    other.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_or_wrong_major_membership_grant_cannot_activate() {
    for major in [None, Some(2)] {
        let dir = tempfile::tempdir().unwrap();
        let mut builder = HarnessBuilder::new(router().into_backend())
            .data_directory(dir.path().canonicalize().unwrap())
            .installation(&"b".repeat(64), "Local")
            .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0);
        if let Some(major) = major {
            builder = builder.capability_version(
                "teamarness.membership",
                ScriptedCapability::new(),
                major,
                0,
            );
        }
        let mut h = builder.start().await;
        assert_eq!(
            h.activate().await.unwrap_err().code(),
            "external_gateway.unavailable"
        );
        assert!(!dir.path().join("config/state.json").exists());
        h.shutdown().await.unwrap();
    }
}
