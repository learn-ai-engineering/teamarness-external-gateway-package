//! Independent CLI publication uses only the existing installer journal, including faults/lifetimes.
mod membership_support;
#[path = "support/setup.rs"]
mod support;
use serde_json::json;
use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use support::*;
use teamarness_generated_package_backend::{
    config::{StorageFault, WriteStage},
    managed_install::{Boundary, InstallFault},
    setup::{Effect, SetupFault},
};
const BOUNDARIES: [Boundary; 15] = [
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
];
struct Cut {
    at: Boundary,
    once: AtomicBool,
    death: bool,
}
impl InstallFault for Cut {
    fn before(&self, at: Boundary) -> std::io::Result<()> {
        if at == self.at && self.once.swap(false, Ordering::SeqCst) {
            if self.death {
                std::process::exit(77);
            }
            Err(std::io::Error::other("isolated installer fault"))
        } else {
            Ok(())
        }
    }
}
struct NoCredentialEffects;
impl StorageFault for NoCredentialEffects {
    fn before(&self, _: WriteStage) -> std::io::Result<()> {
        panic!("CLI tried credential write")
    }
    fn before_origin(&self, _: WriteStage) -> std::io::Result<()> {
        panic!("CLI tried origin creation")
    }
}
impl SetupFault for NoCredentialEffects {
    fn before(&self, _: Effect, _: WriteStage) -> std::io::Result<()> {
        panic!("CLI tried credential journal/receipt")
    }
}
fn root(f: &Fixture) -> std::path::PathBuf {
    f.home
        .path()
        .join(".local/share/teamarness/external-gateway")
}
#[tokio::test]
async fn cli_faults_at_every_installer_boundary_never_fall_back_to_credential_recovery() {
    for at in BOUNDARIES {
        let f = Fixture::new();
        let mut e = f.environment();
        e.install_fault = Some(Arc::new(Cut {
            at,
            once: AtomicBool::new(true),
            death: false,
        }));
        e.client_fault = Some(Arc::new(NoCredentialEffects));
        e.setup_fault = Some(Arc::new(NoCredentialEffects));
        let (mut h, s) = f.start_with(e).await;
        let config = serde_json::to_value(s.policy.setup_snapshot().await.unwrap().config).unwrap();
        let result = apply(&mut h, "install-cli").await;
        assert_eq!(result["phase"], "repair-required", "{at:?}: {result}");
        assert_eq!(result["access"], "unchanged");
        assert_eq!(
            serde_json::to_value(s.policy.setup_snapshot().await.unwrap().config).unwrap(),
            config
        );
        assert!(f.connections().is_empty());
        assert!(!root(&f).join("setup").exists());
        assert!(!f.data.path().join("config/origin").exists());
        let fact = h
            .request("external-gateway.setup.inspect", json!({}))
            .await
            .unwrap();
        assert_eq!(fact["recovery"]["state"], "none");
        h.shutdown().await.unwrap();
        drop(s);
        let (mut h, _) = f.start().await;
        assert_eq!(
            apply(&mut h, "install-cli").await["phase"],
            "complete",
            "{at:?}"
        );
        assert!(!root(&f).join("setup").exists());
        assert!(f.connections().is_empty());
        h.shutdown().await.unwrap();
    }
}

#[test]
fn cli_only_process_child() {
    let Some(home) = std::env::var_os("TMX_CLI_SETUP_TEST_HOME") else {
        return;
    };
    let data = std::path::PathBuf::from(std::env::var_os("TMX_CLI_SETUP_TEST_DATA").unwrap());
    let source = data.join("source");
    let digest = teamarness_generated_package_backend::catalog::digest(&fs::read(&source).unwrap());
    let mut e =
        teamarness_generated_package_backend::setup::BackendEnvironment::injected(home.into())
            .with_admitted_source(
                source,
                teamarness_generated_package_backend::managed_install::Admitted::new(
                    &"b".repeat(64),
                    "1.0.0",
                    &digest,
                )
                .unwrap(),
            );
    let index: usize = std::env::var("TMX_CLI_SETUP_TEST_BOUNDARY")
        .unwrap()
        .parse()
        .unwrap();
    e.install_fault = Some(Arc::new(Cut {
        at: BOUNDARIES[index],
        once: AtomicBool::new(true),
        death: true,
    }));
    e.setup_fault = Some(Arc::new(NoCredentialEffects));
    e.client_fault = Some(Arc::new(NoCredentialEffects));
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut h = teamarness_package_sdk::testing::HarnessBuilder::new(
            teamarness_generated_package_backend::router_with_environment(e).into_backend(),
        )
        .process_minor(1)
        .package_identity("dev.teamarness.external-gateway")
        .data_directory(data)
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version(
            "teamarness.package-ports",
            teamarness_package_sdk::testing::ScriptedCapability::new(),
            3,
            0,
        )
        .start()
        .await;
        h.activate().await.unwrap();
        let r = apply(&mut h, "install-cli").await;
        panic!("death cut missed: {r}");
    });
}
#[tokio::test]
async fn cli_process_death_recovers_exact_publication_despite_malformed_private_records() {
    use std::os::unix::fs::PermissionsExt;
    for (index, at) in BOUNDARIES.iter().enumerate() {
        let f = Fixture::new();
        // Create only safe private directories via the real ClientStore, then seed invalid data.
        let store = teamarness_generated_package_backend::client_store::ClientStore::for_home(
            &f.home.path().canonicalize().unwrap(),
        )
        .unwrap();
        drop(store.lock().unwrap());
        let record = root(&f).join("invalid.json");
        fs::write(&record, b"{private-invalid").unwrap();
        fs::set_permissions(&record, fs::Permissions::from_mode(0o600)).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "cli_only_process_child", "--nocapture"])
            .env(
                "TMX_CLI_SETUP_TEST_HOME",
                f.home.path().canonicalize().unwrap(),
            )
            .env(
                "TMX_CLI_SETUP_TEST_DATA",
                f.data.path().canonicalize().unwrap(),
            )
            .env("TMX_CLI_SETUP_TEST_BOUNDARY", index.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(77),
            "{at:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let (mut h, s) = f.start().await;
        let fact = h
            .request("external-gateway.setup.inspect", json!({}))
            .await
            .unwrap();
        assert_eq!(fact["connection"]["state"], "unavailable");
        assert_ne!(fact["cli"]["state"], "blocked", "{at:?}: {fact}");
        assert_eq!(
            apply(&mut h, "install-cli").await["phase"],
            "complete",
            "{at:?}"
        );
        assert!(fs::read(root(&f).join("managed/command.receipt")).is_ok());
        assert_eq!(fs::read(&record).unwrap(), b"{private-invalid");
        assert!(!root(&f).join("setup").exists());
        assert!(!f.data.path().join("config/origin").exists());
        let snap = s.policy.setup_snapshot().await.unwrap();
        assert_eq!(snap.config.config_version, 1);
        assert!(snap.config.tokens.is_empty());
        h.shutdown().await.unwrap();
    }
}
struct Pause {
    entered: tokio::sync::Notify,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    once: AtomicBool,
}
impl InstallFault for Pause {
    fn before(&self, at: Boundary) -> std::io::Result<()> {
        if at == Boundary::CopyWrite && self.once.swap(false, Ordering::SeqCst) {
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
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_owned_work_survives_observer_loss_and_lifecycle_joins_without_holding_policy() {
    for mode in ["cancel", "suspend", "dispose"] {
        let f = Fixture::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let pause = Arc::new(Pause {
            entered: Default::default(),
            release: Mutex::new(rx),
            once: AtomicBool::new(true),
        });
        let mut e = f.environment();
        e.install_fault = Some(pause.clone());
        let (mut h, s) = f.start_with(e).await;
        let token = h
            .request("external-gateway.token.create", json!({"label":"manual"}))
            .await
            .unwrap();
        let p = h
            .request(
                "external-gateway.setup.preview",
                json!({"intent":"install-cli"}),
            )
            .await
            .unwrap();
        let pending = h
            .start_request(
                "external-gateway.setup.apply",
                json!({"planId":p["planId"]}),
            )
            .await;
        tokio::time::timeout(Duration::from_secs(5), pause.entered.notified())
            .await
            .unwrap();
        pending.cancel().await;
        drop(pending);
        assert!(h.ping().await < Duration::from_secs(1));
        let a = h
            .request(
                "external-gateway.setup.apply",
                json!({"planId":p["planId"]}),
            )
            .await
            .unwrap();
        assert_eq!(a["operationId"], p["planId"]);
        assert_eq!(
            h.request(
                "external-gateway.setup.preview",
                json!({"intent":"install-cli"})
            )
            .await
            .unwrap_err()
            .code(),
            "external_gateway.busy"
        );
        h.request("external-gateway.token.revoke", json!({"id":token["id"]}))
            .await
            .unwrap();
        let fact = h
            .request("external-gateway.setup.inspect", json!({}))
            .await
            .unwrap();
        assert_eq!(fact["cli"]["change"], "explicit-repair");
        if mode == "cancel" {
            tx.send(()).unwrap();
            assert_eq!(finish(&mut h).await["phase"], "complete");
        } else {
            let mut lifecycle = tokio::spawn(async move {
                if mode == "suspend" {
                    h.suspend().await.unwrap();
                } else {
                    h.dispose().await.unwrap();
                }
                h
            });
            assert!(
                tokio::time::timeout(Duration::from_millis(30), &mut lifecycle)
                    .await
                    .is_err()
            );
            tx.send(()).unwrap();
            h = lifecycle.await.unwrap();
            if mode == "suspend" {
                h.recover().await.unwrap();
                assert_eq!(
                    h.request("external-gateway.setup.status", json!({}))
                        .await
                        .unwrap()["phase"],
                    "complete"
                );
            }
        }
        assert!(s.policy.status()["tokens"].as_array().unwrap().is_empty());
        assert!(f.connections().is_empty());
        assert!(!root(&f).join("setup").exists());
        h.shutdown().await.unwrap();
    }
}

struct PreparedFailure;
impl SetupFault for PreparedFailure {
    fn checkpoint(
        &self,
        t: teamarness_generated_package_backend::setup::Transition,
    ) -> std::io::Result<()> {
        if t == teamarness_generated_package_backend::setup::Transition::Prepared {
            Err(std::io::Error::other("preserve legacy prepared journal"))
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn all_legacy_journals_remain_readable_and_reserve_outputs_but_cli_intent_is_never_disk_authority()
 {
    for intent in ["connect", "rotate", "repair", "disconnect", "install-skill"] {
        let f = Fixture::new();
        let (mut h, _) = f.start().await;
        if intent != "connect" {
            connect(&mut h).await;
        } else {
            h.request("external-gateway.listener.set", json!({"enabled":true}))
                .await
                .unwrap();
        }
        h.shutdown().await.unwrap();
        let mut e = f.environment();
        e.setup_fault = Some(Arc::new(PreparedFailure));
        let (mut h, s) = f.start_with(e).await;
        let folder = tempfile::tempdir().unwrap();
        let mut request = json!({"intent":intent});
        if intent == "install-skill" {
            request["selectionId"] = json!(
                s.setup
                    .register_selection(folder.path().canonicalize().unwrap())
                    .await
                    .unwrap()
            );
        }
        let p = h
            .request("external-gateway.setup.preview", request)
            .await
            .unwrap();
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]}),
        )
        .await
        .unwrap();
        assert_eq!(finish(&mut h).await["phase"], "repair-required");
        let facts = h
            .request("external-gateway.setup.inspect", json!({}))
            .await
            .unwrap();
        assert_eq!(facts["recovery"]["state"], "pending", "{intent}: {facts}");
        assert_eq!(facts["recovery"]["intent"], intent);
        assert_eq!(
            h.request(
                "external-gateway.setup.preview",
                json!({"intent":"install-cli"})
            )
            .await
            .unwrap_err()
            .code(),
            "external_gateway.setup_repair_required"
        );
        h.shutdown().await.unwrap();
        drop(s);
        let setup = root(&f).join("setup");
        let journal = fs::read_dir(&setup)
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "operation"))
            .unwrap();
        let original = fs::read(&journal).unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&original).unwrap();
        // Includes legacy InstallSkill carrying association+connection: it remains valid.
        if intent == "install-skill" {
            assert!(v["association"].is_object());
            assert!(v["connection"].is_object());
        }
        // A private-record observation failure cannot hide a valid accepted reservation on restart.
        let invalid = root(&f).join("invalid.json");
        fs::write(&invalid, b"{broken").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&invalid, fs::Permissions::from_mode(0o600)).unwrap();
        let (mut h, _) = f.start().await;
        assert_eq!(
            h.request("external-gateway.setup.inspect", json!({}))
                .await
                .unwrap()["recovery"]["state"],
            "pending"
        );
        assert_eq!(
            h.request(
                "external-gateway.setup.preview",
                json!({"intent":"install-cli"})
            )
            .await
            .unwrap_err()
            .code(),
            "external_gateway.setup_repair_required"
        );
        assert_eq!(fs::read(&journal).unwrap(), original);
        h.shutdown().await.unwrap();
        v["intent"] = json!("install-cli");
        v["receipt"]["intent"] = json!("install-cli");
        v["record"] = json!(null);
        v["association"] = json!(null);
        v["connection"] = json!(null);
        v["oldConnection"] = json!(null);
        v["selected"] = json!(null);
        v["receipt"]["skillPath"] = json!(null);
        let invalid_journal = serde_json::to_vec(&v).unwrap();
        fs::write(&journal, &invalid_journal).unwrap();
        let (mut h, _) = f.start().await;
        assert_eq!(
            h.request("external-gateway.setup.inspect", json!({}))
                .await
                .unwrap()["recovery"]["state"],
            "invalid"
        );
        assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
        assert_eq!(fs::read(&journal).unwrap(), invalid_journal);
        assert_eq!(fs::read(&invalid).unwrap(), b"{broken");
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn cli_never_overwrites_invalid_or_future_completed_credential_history() {
    for variant in ["future", "cli-intent"] {
        let f = Fixture::new();
        let (mut h, _) = f.start().await;
        connect(&mut h).await;
        h.shutdown().await.unwrap();
        let receipt = fs::read_dir(root(&f).join("setup"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|e| e == "receipt"))
            .unwrap();
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
        if variant == "future" {
            value["formatVersion"] = json!(99);
        } else {
            value["receipt"]["intent"] = json!("install-cli");
        }
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&receipt, &bytes).unwrap();
        let (mut h, _) = f.start().await;
        assert_eq!(
            h.request("external-gateway.setup.inspect", json!({}))
                .await
                .unwrap()["recovery"]["state"],
            "invalid"
        );
        assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
        assert_eq!(fs::read(&receipt).unwrap(), bytes);
        h.shutdown().await.unwrap();
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn separate_profiles_contend_on_installer_not_private_credential_owner() {
    let f = Fixture::new();
    let (tx, rx) = std::sync::mpsc::channel();
    let pause = Arc::new(Pause {
        entered: Default::default(),
        release: Mutex::new(rx),
        once: AtomicBool::new(true),
    });
    let mut e = f.environment();
    e.install_fault = Some(pause.clone());
    let (mut h, _) = f.start_with(e).await;
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"install-cli"}),
        )
        .await
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(5), pause.entered.notified())
        .await
        .unwrap();
    let other = Fixture::new();
    let e = teamarness_generated_package_backend::setup::BackendEnvironment::injected(
        f.home.path().canonicalize().unwrap(),
    )
    .with_admitted_source(
        other.source.clone(),
        teamarness_generated_package_backend::managed_install::Admitted::new(
            &"b".repeat(64),
            "1.0.0",
            &teamarness_generated_package_backend::catalog::digest(
                &fs::read(&other.source).unwrap(),
            ),
        )
        .unwrap(),
    );
    let (mut o, _) = other.start_with(e).await;
    let p = o
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"install-cli"}),
        )
        .await
        .unwrap();
    o.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    let result = finish(&mut o).await;
    assert_eq!(result["error"], "external_gateway.installer_busy");
    o.shutdown().await.unwrap();
    tx.send(()).unwrap();
    assert_eq!(finish(&mut h).await["phase"], "complete");
    assert!(f.connections().is_empty());
    h.shutdown().await.unwrap();
}

struct PersistentReceiptFault {
    enabled: AtomicBool,
    failures: AtomicUsize,
}
impl InstallFault for PersistentReceiptFault {
    fn before(&self, at: Boundary) -> std::io::Result<()> {
        if at == Boundary::ReceiptWrite && self.enabled.load(Ordering::SeqCst) {
            self.failures.fetch_add(1, Ordering::SeqCst);
            Err(std::io::Error::other("persistent private receipt fault"))
        } else {
            Ok(())
        }
    }
}
#[tokio::test]
async fn published_command_with_missing_receipt_requires_reload_not_unusable_update() {
    published_command_with_pending_receipt(false).await;
}
#[tokio::test]
async fn published_command_with_old_receipt_requires_reload_not_unusable_update() {
    published_command_with_pending_receipt(true).await;
}
async fn published_command_with_pending_receipt(old_receipt: bool) {
    let f = Fixture::new();
    let receipt = root(&f).join("managed/command.receipt");
    let before = if old_receipt {
        let (mut h, _) = f.start().await;
        assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
        h.shutdown().await.unwrap();
        let bytes = fs::read(&receipt).unwrap();
        fs::write(&f.source, b"changed admitted fixture build").unwrap();
        Some(bytes)
    } else {
        None
    };
    let fault = Arc::new(PersistentReceiptFault {
        enabled: AtomicBool::new(true),
        failures: AtomicUsize::new(0),
    });
    let mut e = f.environment();
    e.install_fault = Some(fault.clone());
    e.client_fault = Some(Arc::new(NoCredentialEffects));
    e.setup_fault = Some(Arc::new(NoCredentialEffects));
    let (mut h, s) = f.start_with(e).await;
    assert_eq!(
        apply(&mut h, "install-cli").await["phase"],
        "repair-required"
    );
    assert_eq!(
        fs::read(f.home.path().join(".local/bin/tmx")).unwrap(),
        fs::read(&f.source).unwrap()
    );
    assert_eq!(fs::read(&receipt).ok(), before);
    assert!(fault.failures.load(Ordering::SeqCst) >= 2);
    let journal = root(&f).join("managed/command.journal");
    let journal_before = fs::read(&journal).unwrap();
    // Fault survived both owned apply and immediate recovery. Storage now recovers,
    // but read-only inspection must not reconcile or advertise an unusable preview.
    fault.enabled.store(false, Ordering::SeqCst);
    let fact = h
        .request("external-gateway.setup.inspect", json!({}))
        .await
        .unwrap();
    assert_eq!(fact["cli"]["state"], "installed");
    assert_eq!(fact["cli"]["issue"], "external_gateway.reload_required");
    assert_eq!(fs::read(&receipt).ok(), before);
    assert_eq!(
        h.request(
            "external-gateway.setup.preview",
            json!({"intent":"install-cli"})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.output_collision"
    );
    assert_eq!(fs::read(&journal).unwrap(), journal_before);
    assert!(f.connections().is_empty());
    assert!(!f.data.path().join("config/origin").exists());
    h.shutdown().await.unwrap();
    drop(s);
    let (mut h, _) = f.start().await;
    let fact = h
        .request("external-gateway.setup.inspect", json!({}))
        .await
        .unwrap();
    assert_eq!(fact["cli"]["change"], "none");
    assert!(fact["cli"]["issue"].is_null());
    assert!(!root(&f).join("managed/command.journal").exists());
    h.shutdown().await.unwrap();
}
