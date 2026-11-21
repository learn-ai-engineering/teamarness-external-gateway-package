//! Real durable files, owned work, process death and OS faults across setup publication.
#[path = "support/setup.rs"]
mod support;
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use support::*;
use teamarness_generated_package_backend::{
    config::{StorageFault, WriteStage},
    policy::Change,
    setup::{Effect, SetupFault, Transition},
};
struct Cut {
    point: String,
    armed: AtomicBool,
    death: bool,
}
fn stage(s: WriteStage) -> &'static str {
    match s {
        WriteStage::Write => "write",
        WriteStage::FileSync => "fsync",
        WriteStage::Rename => "rename",
        WriteStage::DirectorySync => "dirsync",
    }
}
fn transition(t: Transition) -> &'static str {
    match t {
        Transition::Prepared => "prepared",
        Transition::Outputs => "outputs",
        Transition::Committed => "committed",
        Transition::Published => "published",
    }
}
impl Cut {
    fn hit(&self, p: String) -> std::io::Result<()> {
        if self.point == p && self.armed.swap(false, Ordering::SeqCst) {
            if self.death {
                std::process::exit(77);
            }
            return Err(std::io::Error::other("injected boundary"));
        }
        Ok(())
    }
}
impl SetupFault for Cut {
    fn before(&self, e: Effect, s: WriteStage) -> std::io::Result<()> {
        self.hit(format!(
            "{}:{}",
            match e {
                Effect::Journal => "journal",
                Effect::Receipt => "receipt",
                Effect::JournalCleanup => "cleanup",
            },
            stage(s)
        ))
    }
    fn checkpoint(&self, t: Transition) -> std::io::Result<()> {
        self.hit(transition(t).into())
    }
}
struct StoreCut {
    cut: Arc<Cut>,
    kind: &'static str,
}
impl StorageFault for StoreCut {
    fn before_origin(&self, s: WriteStage) -> std::io::Result<()> {
        self.cut.hit(format!("origin:{}", stage(s)))
    }
    fn before(&self, s: WriteStage) -> std::io::Result<()> {
        self.cut.hit(format!("{}:{}", self.kind, stage(s)))
    }
}
struct Pause {
    at: Transition,
    entered: tokio::sync::Notify,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    once: AtomicBool,
}
impl SetupFault for Pause {
    fn checkpoint(&self, t: Transition) -> std::io::Result<()> {
        if self.at == t && self.once.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        }
        Ok(())
    }
}
fn pause(at: Transition) -> (Arc<Pause>, std::sync::mpsc::Sender<()>) {
    let (tx, rx) = std::sync::mpsc::channel();
    (
        Arc::new(Pause {
            at,
            entered: Default::default(),
            release: Mutex::new(rx),
            once: AtomicBool::new(true),
        }),
        tx,
    )
}
async fn entered(p: &Pause) {
    tokio::time::timeout(Duration::from_secs(5), p.entered.notified())
        .await
        .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_acceptance_cancelled_observer_remount_and_slow_disk_preserve_one_owner_and_health() {
    let f = Fixture::new();
    let (p, release) = pause(Transition::Prepared);
    let mut e = f.environment();
    e.setup_fault = Some(p.clone());
    let (mut h, _) = f.start_with(e).await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    let plan = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"}),
        )
        .await
        .unwrap();
    let mut watch = h
        .subscribe("external-gateway.setup.watch", json!({}))
        .await
        .unwrap();
    watch.next_event().await.unwrap().unwrap();
    let pending = h
        .start_request(
            "external-gateway.setup.apply",
            json!({"planId":plan["planId"]}),
        )
        .await;
    entered(&p).await;
    pending.cancel().await;
    drop(pending);
    watch.close().await;
    assert!(h.ping().await < Duration::from_secs(1));
    let accepted = h
        .request(
            "external-gateway.setup.apply",
            json!({"planId":plan["planId"]}),
        )
        .await
        .unwrap();
    assert_eq!(accepted["operationId"], plan["planId"]);
    assert_eq!(
        h.request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.busy"
    );
    let mut remount = h
        .subscribe("external-gateway.setup.watch", json!({}))
        .await
        .unwrap();
    assert_eq!(
        remount.next_event().await.unwrap().unwrap()["operationId"],
        accepted["operationId"]
    );
    release.send(()).unwrap();
    assert_eq!(finish(&mut h).await["phase"], "complete");
    remount.close().await;
    assert_eq!(f.connections().len(), 1);
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["tokens"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    h.shutdown().await.unwrap();
    let (mut h, _) = f.start().await;
    assert_eq!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":plan["planId"]})
        )
        .await
        .unwrap(),
        accepted
    );
    h.shutdown().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoke_disable_all_or_new_generation_after_commit_never_publish_or_resurrect() {
    for action in ["revoke", "disable-all", "supersede"] {
        let f = Fixture::new();
        let (p, release) = pause(Transition::Committed);
        let mut e = f.environment();
        e.setup_fault = Some(p.clone());
        let (mut h, s) = f.start_with(e).await;
        h.request("external-gateway.listener.set", json!({"enabled":true}))
            .await
            .unwrap();
        let plan = h
            .request(
                "external-gateway.setup.preview",
                json!({"intent":"connect"}),
            )
            .await
            .unwrap();
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":plan["planId"]}),
        )
        .await
        .unwrap();
        entered(&p).await;
        let snap = s.policy.setup_snapshot().await.unwrap();
        let m = snap.config.managed.unwrap();
        match action {
            "revoke" => {
                s.policy
                    .change(Change::Revoke(m.token_id.clone()))
                    .await
                    .unwrap();
            }
            "disable-all" => {
                s.policy.change(Change::DisableAll).await.unwrap();
            }
            _ => {
                let (r, _) = teamarness_generated_package_backend::tokens::issue(
                    "Managed CLI",
                    m.generation + 1,
                    Some(&m.token_id),
                    &teamarness_generated_package_backend::tokens::OsEntropy,
                )
                .unwrap();
                let a = teamarness_generated_package_backend::config::ManagedAssociation {
                    operation_id: "f".repeat(64),
                    generation: r.generation,
                    ..m.clone()
                };
                s.policy
                    .commit_managed(snap.revision, a, Some(r))
                    .await
                    .unwrap();
            }
        }
        release.send(()).unwrap();
        assert_ne!(finish(&mut h).await["access"], "configured");
        assert!(f.connections().is_empty());
        h.shutdown().await.unwrap();
        drop(s);
        let (mut h, s) = f.start().await;
        assert!(f.connections().is_empty());
        assert_ne!(
            h.request("external-gateway.setup.status", json!({}))
                .await
                .unwrap()["access"],
            "configured"
        );
        let tokens = s.policy.status()["tokens"].as_array().unwrap().clone();
        assert_eq!(tokens.len(), usize::from(action == "supersede"));
        if action == "supersede" {
            assert_eq!(tokens[0]["generation"], 2);
        }
        h.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_name_refresh_cannot_undo_rotation_and_client_lock_never_blocks_manual_revocation() {
    let f = Fixture::new();
    let (mut h, _) = f.start().await;
    connect(&mut h).await;
    let old = f.connections().remove(0);
    h.shutdown().await.unwrap();
    let (p, release) = pause(Transition::Committed);
    let mut e = f.environment();
    e.setup_fault = Some(p.clone());
    let (mut h, s) = f.start_with(e).await;
    let plan = h
        .request("external-gateway.setup.preview", json!({"intent":"rotate"}))
        .await
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":plan["planId"]}),
    )
    .await
    .unwrap();
    entered(&p).await;
    let store = teamarness_generated_package_backend::client_store::ClientStore::for_home(
        &f.home.path().canonicalize().unwrap(),
    )
    .unwrap();
    assert!(store.refresh_name(&old, "new name").is_err());
    assert!(s.policy.authenticate(&old.bearer).await.is_err());
    release.send(()).unwrap();
    assert_eq!(finish(&mut h).await["phase"], "complete");
    let current = f.connections().remove(0);
    assert!(store.refresh_name(&old, "stale").is_err());
    assert_eq!(f.connections()[0].operation_id, current.operation_id);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn uncertain_disconnect_revoke_is_not_success_and_cleanup_pending_retry_never_mints() {
    for point in ["policy:dirsync", "client:rename", "client:dirsync"] {
        let f = Fixture::new();
        let (mut h, _) = f.start().await;
        connect(&mut h).await;
        let old = f.connections().remove(0);
        h.shutdown().await.unwrap();
        let cut = Arc::new(Cut {
            point: point.into(),
            armed: AtomicBool::new(false),
            death: false,
        });
        let mut e = f.environment();
        e.storage_fault = Some(Arc::new(StoreCut {
            cut: cut.clone(),
            kind: "policy",
        }));
        e.client_fault = Some(Arc::new(StoreCut {
            cut: cut.clone(),
            kind: "client",
        }));
        let (mut h, s) = f.start_with(e).await;
        cut.armed.store(true, Ordering::SeqCst);
        let status = apply(&mut h, "disconnect").await;
        if point.starts_with("policy") {
            assert_eq!(status["phase"], "repair-required");
            assert_ne!(status["access"], "revoked");
            assert_eq!(s.policy.status()["storageFailed"], true);
        } else {
            assert_eq!(status["phase"], "cleanup-pending");
            assert_eq!(status["access"], "revoked");
            assert_eq!(apply(&mut h, "repair").await["phase"], "complete");
        }
        assert!(s.policy.authenticate(&old.bearer).await.is_err());
        h.shutdown().await.unwrap();
        drop(s);
        let (h, s) = f.start().await;
        assert!(s.policy.authenticate(&old.bearer).await.is_err());
        assert!(f.connections().is_empty());
        assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
        h.shutdown().await.unwrap();
    }
}
#[test]
fn setup_process_child() {
    let Some(data) = std::env::var_os("TMX_SETUP_TEST_DATA") else {
        return;
    };
    let home = std::path::PathBuf::from(std::env::var_os("TMX_SETUP_TEST_HOME").unwrap());
    let source = std::path::PathBuf::from(&data).join("source");
    let cut = Arc::new(Cut {
        point: std::env::var("TMX_SETUP_TEST_POINT").unwrap(),
        armed: AtomicBool::new(false),
        death: true,
    });
    let digest =
        teamarness_generated_package_backend::catalog::digest(&std::fs::read(&source).unwrap());
    let mut e = teamarness_generated_package_backend::setup::BackendEnvironment::injected(home)
        .with_admitted_source(
            source,
            teamarness_generated_package_backend::managed_install::Admitted::new(
                &"b".repeat(64),
                "1.0.0",
                &digest,
            )
            .unwrap(),
        );
    e.setup_fault = Some(cut.clone());
    e.storage_fault = Some(Arc::new(StoreCut {
        cut: cut.clone(),
        kind: "policy",
    }));
    e.client_fault = Some(Arc::new(StoreCut {
        cut: cut.clone(),
        kind: "client",
    }));
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut h = teamarness_package_sdk::testing::HarnessBuilder::new(
            teamarness_generated_package_backend::router_with_environment(e).into_backend(),
        )
        .process_minor(1)
        .package_identity("dev.teamarness.external-gateway")
        .data_directory(std::path::PathBuf::from(data))
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_present())
        .capability_version(
            "teamarness.package-ports",
            teamarness_package_sdk::testing::ScriptedCapability::new(),
            3,
            0,
        )
        .start()
        .await;
        h.activate().await.unwrap();
        h.request("external-gateway.listener.set", json!({"enabled":true}))
            .await
            .unwrap();
        cut.armed.store(true, Ordering::SeqCst);
        let intent = std::env::var("TMX_SETUP_TEST_INTENT").unwrap();
        let status = apply(&mut h, &intent).await;
        panic!("cut did not execute: {}", status["phase"]);
    });
}
#[path = "membership_support/mod.rs"]
mod membership_support;
fn membership_present() -> teamarness_package_sdk::testing::ScriptedCapability {
    membership_support::present()
}
#[tokio::test]
async fn actual_process_death_at_every_setup_policy_connection_and_receipt_boundary() {
    let mut points = vec![
        "prepared".to_string(),
        "outputs".into(),
        "committed".into(),
        "published".into(),
    ];
    for kind in ["journal", "policy", "client", "receipt"] {
        for stage in ["write", "fsync", "rename", "dirsync"] {
            points.push(format!("{kind}:{stage}"));
        }
    }
    points.extend(["cleanup:rename".into(), "cleanup:dirsync".into()]);
    for intent in ["connect", "rotate"] {
        for point in &points {
            let f = Fixture::new();
            let old = if intent == "rotate" {
                let (mut h, _) = f.start().await;
                connect(&mut h).await;
                h.shutdown().await.unwrap();
                Some(f.connections().remove(0))
            } else {
                None
            };
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "setup_process_child", "--nocapture"])
                .env("TMX_SETUP_TEST_DATA", f.data.path().canonicalize().unwrap())
                .env("TMX_SETUP_TEST_HOME", f.home.path().canonicalize().unwrap())
                .env("TMX_SETUP_TEST_POINT", point)
                .env("TMX_SETUP_TEST_INTENT", intent)
                .output()
                .unwrap();
            assert_eq!(
                status.status.code(),
                Some(77),
                "{intent}/{point}: {}",
                String::from_utf8_lossy(&status.stderr)
            );
            let (mut h, s) = f.start().await;
            let state = h
                .request("external-gateway.setup.status", json!({}))
                .await
                .unwrap();
            let snapshot = s.policy.setup_snapshot().await.unwrap();
            let managed = snapshot.config.managed.as_ref();
            let committed =
                managed.is_some_and(|m| m.generation == if intent == "rotate" { 2 } else { 1 });
            if committed {
                assert_eq!(state["phase"], "complete", "{intent}/{point}: {state}");
                let c = f.connections().remove(0);
                assert_eq!(c.generation, managed.unwrap().generation);
                s.policy.authenticate(&c.bearer).await.unwrap();
                if let Some(old) = &old {
                    assert!(s.policy.authenticate(&old.bearer).await.is_err());
                }
            } else {
                if let Some(old) = &old {
                    s.policy.authenticate(&old.bearer).await.unwrap();
                } else {
                    assert!(snapshot.config.tokens.is_empty());
                    assert!(f.connections().is_empty());
                }
                if state["phase"] == "repair-required" {
                    let repaired = apply(&mut h, "repair").await;
                    assert_eq!(
                        repaired["phase"], "complete",
                        "{intent}/{point}: {repaired}"
                    );
                } else if intent == "connect" {
                    assert_eq!(apply(&mut h, "connect").await["phase"], "complete");
                }
            }
            assert_eq!(s.policy.status()["tokens"].as_array().unwrap().len(), 1);
            h.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn required_output_faults_leave_no_verifier_and_repair_only_recognized_bytes() {
    use teamarness_generated_package_backend::managed_install::{Boundary, InstallFault};
    struct OutputCut {
        at: Boundary,
        once: AtomicBool,
    }
    impl InstallFault for OutputCut {
        fn before(&self, b: Boundary) -> std::io::Result<()> {
            if b == self.at && self.once.swap(false, Ordering::SeqCst) {
                Err(std::io::Error::other("actual output boundary"))
            } else {
                Ok(())
            }
        }
    }
    for at in [
        Boundary::JournalWrite,
        Boundary::JournalSync,
        Boundary::JournalRename,
        Boundary::JournalDirectorySync,
        Boundary::CopyWrite,
        Boundary::CopiedVerify,
        Boundary::FileSync,
        Boundary::OutputRename,
        Boundary::OutputDirectorySync,
        Boundary::ReceiptWrite,
        Boundary::ReceiptSync,
        Boundary::ReceiptRename,
        Boundary::ReceiptDirectorySync,
        Boundary::JournalRemove,
        Boundary::CompletionSync,
    ] {
        let f = Fixture::new();
        let mut e = f.environment();
        e.install_fault = Some(Arc::new(OutputCut {
            at,
            once: AtomicBool::new(true),
        }));
        let (mut h, s) = f.start_with(e).await;
        h.request("external-gateway.listener.set", json!({"enabled":true}))
            .await
            .unwrap();
        let result = apply(&mut h, "connect").await;
        assert_eq!(result["phase"], "repair-required", "{at:?}: {result}");
        assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
        assert!(f.connections().is_empty());
        h.shutdown().await.unwrap();
        drop(s);
        let (mut h, _) = f.start().await;
        assert_eq!(
            h.request("external-gateway.setup.status", json!({}))
                .await
                .unwrap()["phase"],
            "repair-required"
        );
        let result = apply(&mut h, "repair").await;
        assert_eq!(result["phase"], "complete", "{at:?}: {result}");
        assert_eq!(f.connections().len(), 1);
        h.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skill_failure_after_command_publication_and_user_edit_never_activate_access() {
    let f = Fixture::new();
    let folder = support::private_dir();
    let (p, release) = pause(Transition::Outputs);
    let mut e = f.environment();
    e.setup_fault = Some(p.clone());
    let (mut h, s) = f.start_with(e).await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    let id = s
        .setup
        .register_selection(folder.path().canonicalize().unwrap())
        .await
        .unwrap();
    let plan = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"connect","selectionId":id}),
        )
        .await
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":plan["planId"]}),
    )
    .await
    .unwrap();
    entered(&p).await;
    std::fs::write(
        folder.path().join("teamarness/SKILL.md"),
        b"Person edit, do not overwrite",
    )
    .unwrap();
    release.send(()).unwrap();
    assert_eq!(finish(&mut h).await["phase"], "repair-required");
    assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
    assert!(f.connections().is_empty());
    h.shutdown().await.unwrap();
    drop(s);
    let (mut h, s) = f.start().await;
    assert!(
        h.request("external-gateway.setup.preview", json!({"intent":"repair"}))
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read(folder.path().join("teamarness/SKILL.md")).unwrap(),
        b"Person edit, do not overwrite"
    );
    assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn future_or_invalid_journals_and_receipts_are_untouched_and_do_not_become_overwrite_authority()
 {
    for suffix in ["operation", "receipt"] {
        let f = Fixture::new();
        let (mut h, _) = f.start().await;
        connect(&mut h).await;
        h.shutdown().await.unwrap();
        let setup = f
            .home
            .path()
            .join(".local/share/teamarness/external-gateway/setup");
        let receipt = std::fs::read_dir(&setup)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|s| s == "receipt"))
            .unwrap();
        let path = receipt.with_extension(suffix);
        let bytes = br#"{"formatVersion":99,"unsafe":"never overwrite"}"#;
        std::fs::write(&path, bytes).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let (mut h, _) = f.start().await;
        assert_eq!(
            h.request("external-gateway.setup.status", json!({}))
                .await
                .unwrap()["phase"],
            "repair-required"
        );
        assert!(
            h.request("external-gateway.setup.preview", json!({"intent":"rotate"}))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
        h.shutdown().await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn suspension_invalidates_admission_before_waiting_for_owned_disk_checkpoint() {
    let f = Fixture::new();
    let (p, release) = pause(Transition::Outputs);
    let mut e = f.environment();
    e.setup_fault = Some(p.clone());
    let (mut h, s) = f.start_with(e).await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    let plan = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"}),
        )
        .await
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":plan["planId"]}),
    )
    .await
    .unwrap();
    entered(&p).await;
    let suspend = tokio::spawn(async move {
        h.suspend().await.unwrap();
        h
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while s.policy.status()["running"] == true {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    release.send(()).unwrap();
    let mut h = suspend.await.unwrap();
    assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
    assert!(f.connections().is_empty());
    h.recover().await.unwrap();
    assert_eq!(apply(&mut h, "repair").await["phase"], "complete");
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn every_private_journal_update_fault_requires_reconciliation_not_new_authority() {
    use std::sync::atomic::AtomicUsize;
    struct Nth {
        at: WriteStage,
        nth: usize,
        count: AtomicUsize,
    }
    impl SetupFault for Nth {
        fn before(&self, e: Effect, s: WriteStage) -> std::io::Result<()> {
            if e == Effect::Journal
                && s == self.at
                && self.count.fetch_add(1, Ordering::SeqCst) + 1 == self.nth
            {
                Err(std::io::Error::other("journal update durability"))
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
        for nth in 1..=5 {
            let f = Fixture::new();
            let folder = support::private_dir();
            let mut e = f.environment();
            e.setup_fault = Some(Arc::new(Nth {
                at: stage,
                nth,
                count: AtomicUsize::new(0),
            }));
            let (mut h, s) = f.start_with(e).await;
            h.request("external-gateway.listener.set", json!({"enabled":true}))
                .await
                .unwrap();
            let id = s
                .setup
                .register_selection(folder.path().canonicalize().unwrap())
                .await
                .unwrap();
            let p = h
                .request(
                    "external-gateway.setup.preview",
                    json!({"intent":"connect","selectionId":id}),
                )
                .await
                .unwrap();
            h.request(
                "external-gateway.setup.apply",
                json!({"planId":p["planId"]}),
            )
            .await
            .unwrap();
            assert_eq!(
                finish(&mut h).await["phase"],
                if nth == 5 {
                    "complete"
                } else {
                    "repair-required"
                }
            );
            let committed = !s.policy.status()["tokens"].as_array().unwrap().is_empty();
            assert_eq!(committed, nth == 5);
            if !committed {
                assert!(f.connections().is_empty());
            }
            h.shutdown().await.unwrap();
            drop(s);
            let (mut h, s) = f.start().await;
            let state = h
                .request("external-gateway.setup.status", json!({}))
                .await
                .unwrap();
            if committed {
                assert_eq!(state["phase"], "complete", "{stage:?}/{nth}: {state}");
            } else if state["phase"] == "repair-required" {
                assert_eq!(apply(&mut h, "repair").await["phase"], "complete");
            } else {
                assert_eq!(state["phase"], "idle");
                assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
            }
            if committed || state["phase"] != "idle" {
                let c = f.connections().remove(0);
                s.policy.authenticate(&c.bearer).await.unwrap();
                let dir = f
                    .home
                    .path()
                    .join(".local/share/teamarness/external-gateway/setup");
                for entry in std::fs::read_dir(dir).unwrap() {
                    let p = entry.unwrap().path();
                    assert!(!std::fs::read_to_string(p).unwrap().contains(&c.bearer));
                }
            }
            h.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn in_process_explicit_repair_reconciles_uncertain_journal_progress_without_restart() {
    let f = Fixture::new();
    let cut = Arc::new(Cut {
        point: "journal:dirsync".into(),
        armed: AtomicBool::new(true),
        death: false,
    });
    let mut e = f.environment();
    e.setup_fault = Some(cut);
    let (mut h, _) = f.start_with(e).await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    assert_eq!(apply(&mut h, "connect").await["phase"], "repair-required");
    assert_eq!(apply(&mut h, "repair").await["phase"], "complete");
    assert_eq!(f.connections().len(), 1);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn origin_process_death_before_journal_retries_without_adopting_abandoned_bytes() {
    for point in ["write", "fsync", "rename", "dirsync"] {
        let f = Fixture::new();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "setup_process_child", "--nocapture"])
            .env("TMX_SETUP_TEST_DATA", f.data.path().canonicalize().unwrap())
            .env("TMX_SETUP_TEST_HOME", f.home.path().canonicalize().unwrap())
            .env("TMX_SETUP_TEST_POINT", format!("origin:{point}"))
            .env("TMX_SETUP_TEST_INTENT", "connect")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(77),
            "{point}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let config = f.data.path().join("config");
        let published = std::fs::read(config.join("origin")).ok();
        assert_eq!(published.is_some(), point == "dirsync");
        let abandoned: Vec<_> = std::fs::read_dir(&config)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("origin-")
            })
            .map(|p| {
                let bytes = std::fs::read(&p).unwrap();
                (p, bytes)
            })
            .collect();
        assert_eq!(abandoned.len(), usize::from(point != "dirsync"));
        let (mut h, s) = f.start().await;
        assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
        assert!(f.connections().is_empty());
        assert_eq!(apply(&mut h, "connect").await["phase"], "complete");
        let connection = f.connections().remove(0);
        let origin = std::fs::read_to_string(config.join("origin")).unwrap();
        assert_eq!(connection.origin_nonce, origin);
        assert_eq!(
            s.policy
                .setup_snapshot()
                .await
                .unwrap()
                .config
                .managed
                .unwrap()
                .origin_nonce,
            origin
        );
        if let Some(published) = published {
            assert_eq!(published, origin.as_bytes());
        }
        for (path, bytes) in abandoned {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
            if bytes.len() == 64 {
                assert_ne!(bytes, origin.as_bytes());
            }
        }
        s.policy.authenticate(&connection.bearer).await.unwrap();
        assert_eq!(s.policy.status()["tokens"].as_array().unwrap().len(), 1);
        h.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn origin_initial_fault_then_explicit_connect_retry_uses_same_published_association() {
    for point in ["write", "fsync", "rename", "dirsync"] {
        let f = Fixture::new();
        let cut = Arc::new(Cut {
            point: format!("origin:{point}"),
            armed: AtomicBool::new(true),
            death: false,
        });
        let mut e = f.environment();
        e.storage_fault = Some(Arc::new(StoreCut {
            cut,
            kind: "policy",
        }));
        let (mut h, s) = f.start_with(e).await;
        h.request("external-gateway.listener.set", json!({"enabled":true}))
            .await
            .unwrap();
        assert_eq!(apply(&mut h, "connect").await["phase"], "repair-required");
        assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
        assert!(f.connections().is_empty());
        assert_eq!(apply(&mut h, "connect").await["phase"], "complete");
        let c = f.connections().remove(0);
        assert_eq!(
            c.origin_nonce,
            std::fs::read_to_string(f.data.path().join("config/origin")).unwrap()
        );
        h.shutdown().await.unwrap();
        let (h, s) = f.start().await;
        assert_eq!(f.connections()[0].origin_nonce, c.origin_nonce);
        s.policy.authenticate(&c.bearer).await.unwrap();
        h.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn explicit_denial_retry_after_transient_storage_failure_durably_clears_warning_without_restart()
 {
    for target in ["manual", "managed"] {
        for retry_all in [false, true] {
            let f = Fixture::new();
            let cut = Arc::new(Cut {
                point: "policy:write".into(),
                armed: AtomicBool::new(false),
                death: false,
            });
            let mut e = f.environment();
            e.storage_fault = Some(Arc::new(StoreCut {
                cut: cut.clone(),
                kind: "policy",
            }));
            let (mut h, s) = f.start_with(e).await;
            connect(&mut h).await;
            let managed = f.connections().remove(0);
            let manual = h
                .request("external-gateway.token.create", json!({"label":"Manual"}))
                .await
                .unwrap();
            let id = if target == "manual" {
                manual["id"].as_str().unwrap()
            } else {
                &managed.token_id
            };
            if retry_all {
                // Exercise the zero-token denial intersection as well as a remaining
                // manual/managed row: only revoke-all is then visible to the caller.
                let other = if target == "manual" {
                    &managed.token_id
                } else {
                    manual["id"].as_str().unwrap()
                };
                h.request("external-gateway.token.revoke", json!({"id":other}))
                    .await
                    .unwrap();
            }
            let mut watch = h
                .subscribe("external-gateway.status.watch", json!({}))
                .await
                .unwrap();
            watch.next_event().await.unwrap().unwrap();
            cut.armed.store(true, Ordering::SeqCst);
            assert_eq!(
                h.request("external-gateway.token.revoke", json!({"id":id}))
                    .await
                    .unwrap_err()
                    .code(),
                "external_gateway.storage_failed"
            );
            let failed = watch.next_event().await.unwrap().unwrap();
            assert_eq!(failed["storageFailed"], true);
            assert_eq!(
                failed["tokens"].as_array().unwrap().len(),
                usize::from(!retry_all)
            );
            let disk = teamarness_generated_package_backend::config::FileStore::open(
                &f.data.path().canonicalize().unwrap(),
            )
            .unwrap();
            assert_eq!(
                disk.read().unwrap().tokens.len(),
                if retry_all { 1 } else { 2 }
            );
            assert!(s.policy.authenticate(&managed.bearer).await.is_err());
            assert!(
                s.policy
                    .authenticate(manual["bearer"].as_str().unwrap())
                    .await
                    .is_err()
            );
            assert!(s.policy.setup_snapshot().await.is_err());
            // The one-shot IO fault is now removed. A status read cannot clear the latch.
            assert_eq!(
                h.request("external-gateway.status.get", json!({}))
                    .await
                    .unwrap()["storageFailed"],
                true
            );
            if retry_all {
                h.request("external-gateway.disable-and-revoke-all", json!({}))
                    .await
                    .unwrap();
            } else {
                let remaining = &failed["tokens"][0]["id"];
                h.request("external-gateway.token.revoke", json!({"id":remaining}))
                    .await
                    .unwrap();
            }
            let reconciled = watch.next_event().await.unwrap().unwrap();
            assert_eq!(reconciled["storageFailed"], false);
            assert_eq!(reconciled["tokens"], json!([]));
            assert!(disk.read().unwrap().tokens.is_empty());
            if retry_all {
                assert!(!disk.read().unwrap().desired_enabled);
            }
            assert!(s.policy.authenticate(&managed.bearer).await.is_err());
            assert!(
                s.policy
                    .authenticate(manual["bearer"].as_str().unwrap())
                    .await
                    .is_err()
            );
            watch.close().await;
            h.shutdown().await.unwrap();
            let (h, s) = f.start().await;
            assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
            assert!(s.policy.authenticate(&managed.bearer).await.is_err());
            h.shutdown().await.unwrap();
        }
    }
}
