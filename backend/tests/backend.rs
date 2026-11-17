//! Verifies: TEAM.SYS.PKG-R03, R05, R19. Typed UI requests through the real SDK router.
#[path = "support/environment.rs"]
mod backend_environment;
mod membership_support;
use serde_json::json;
use teamarness_package_sdk::testing::{
    HarnessBuilder, ScriptedCapability, assert_router_matches_contract,
};
#[test]
fn router_matches_the_ui_contract() {
    assert_router_matches_contract(
        &backend_environment::router().operations(),
        include_str!("../../ui-contract.json"),
    );
}
#[tokio::test]
async fn default_disabled_tokens_are_durable_and_secrets_are_one_time() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(backend_environment::router().into_backend())
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    let s = h
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(s["desiredEnabled"], false);
    assert_eq!(s["tokens"], json!([]));
    assert_eq!(s["running"], false);
    let token = h
        .request("external-gateway.token.create", json!({"label":"client"}))
        .await
        .unwrap();
    let bearer = token["bearer"].as_str().unwrap();
    let s = h
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(s["tokens"][0]["label"], "client");
    assert!(!s.to_string().contains(bearer));
    assert!(
        !std::fs::read_to_string(dir.path().join("config/state.json"))
            .unwrap()
            .contains(bearer)
    );
    let regenerated = h
        .request(
            "external-gateway.token.regenerate",
            json!({"id":token["id"]}),
        )
        .await
        .unwrap();
    assert_ne!(regenerated["bearer"], token["bearer"]);
    h.request("external-gateway.disable-and-revoke-all", json!({}))
        .await
        .unwrap();
    h.shutdown().await.unwrap();
    let mut h = HarnessBuilder::new(backend_environment::router().into_backend())
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["tokens"],
        json!([])
    );
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_revocation_cannot_be_resurrected_by_a_later_unrelated_write() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use teamarness_generated_package_backend::{
        config::{StorageFault, WriteStage},
        tokens::OsEntropy,
    };
    struct Fault(AtomicBool);
    impl StorageFault for Fault {
        fn before(&self, s: WriteStage) -> Result<(), std::io::Error> {
            if s == WriteStage::Write && self.0.swap(false, Ordering::SeqCst) {
                Err(std::io::Error::other("injected storage failure"))
            } else {
                Ok(())
            }
        }
    }
    let fault = Arc::new(Fault(AtomicBool::new(false)));
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(
        backend_environment::router_with_storage_fault(Arc::new(OsEntropy), Some(fault.clone()))
            .into_backend(),
    )
    .data_directory(dir.path().canonicalize().unwrap())
    .installation(&"b".repeat(64), "Local")
    .capability("teamarness.membership", membership_support::present())
    .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
    .start()
    .await;
    h.activate().await.unwrap();
    let token = h
        .request(
            "external-gateway.token.create",
            json!({"label":"revoke me"}),
        )
        .await
        .unwrap();
    fault.0.store(true, Ordering::SeqCst);
    assert_eq!(
        h.request("external-gateway.token.revoke", json!({"id":token["id"]}))
            .await
            .unwrap_err()
            .code(),
        "external_gateway.storage_failed"
    );
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["storageFailed"],
        true
    );
    h.request("external-gateway.port.set", json!({"port":12345}))
        .await
        .unwrap();
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["tokens"],
        json!([])
    );
    h.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_waiter_cannot_release_actual_file_transaction_ownership() {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use teamarness_generated_package_backend::{
        config::{StorageFault, WriteStage},
        tokens::OsEntropy,
    };
    struct Pause {
        entered: tokio::sync::Notify,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        once: AtomicBool,
    }
    impl StorageFault for Pause {
        fn before(&self, s: WriteStage) -> Result<(), std::io::Error> {
            if s == WriteStage::FileSync && self.once.swap(false, Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.lock().unwrap().recv().unwrap();
            }
            Ok(())
        }
    }
    let (release, rx) = std::sync::mpsc::channel();
    let fault = Arc::new(Pause {
        entered: tokio::sync::Notify::new(),
        release: Mutex::new(rx),
        once: AtomicBool::new(true),
    });
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(
        backend_environment::router_with_storage_fault(Arc::new(OsEntropy), Some(fault.clone()))
            .into_backend(),
    )
    .data_directory(dir.path().canonicalize().unwrap())
    .installation(&"b".repeat(64), "Local")
    .capability("teamarness.membership", membership_support::present())
    .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
    .start()
    .await;
    h.activate().await.unwrap();
    let mut status = h
        .subscribe("external-gateway.status.watch", json!({}))
        .await
        .unwrap();
    status.next_event().await.unwrap().unwrap();
    let pending = h
        .start_request("external-gateway.port.set", json!({"port":43210}))
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), fault.entered.notified())
        .await
        .expect("file transaction reached fsync barrier");
    pending.cancel().await;
    assert_eq!(
        h.request("external-gateway.port.set", json!({"port":43211}))
            .await
            .unwrap_err()
            .code(),
        "external_gateway.busy"
    );
    release.send(()).unwrap();
    let committed = tokio::time::timeout(std::time::Duration::from_secs(5), status.next_event())
        .await
        .expect("owned commit publishes status")
        .unwrap()
        .unwrap();
    assert_eq!(committed["port"], 43210);
    status.close().await;
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["port"],
        43210
    );
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn token_limit_and_closed_ui_requests_fail_without_secret_or_partial_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(backend_environment::router().into_backend())
        .data_directory(dir.path().canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    assert!(
        h.request(
            "external-gateway.token.create",
            json!({"label":"a","scope":"all"})
        )
        .await
        .is_err()
    );
    assert!(
        h.request("external-gateway.port.set", json!({"port":0}))
            .await
            .is_err()
    );
    for n in 0..32 {
        h.request(
            "external-gateway.token.create",
            json!({"label":format!("client {n}")}),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        h.request("external-gateway.token.create", json!({"label":"overflow"}))
            .await
            .unwrap_err()
            .code(),
        "external_gateway.too_large"
    );
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["tokens"]
            .as_array()
            .unwrap()
            .len(),
        32
    );
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn entropy_failure_is_visible_and_cannot_create_credentials() {
    use teamarness_generated_package_backend::tokens::{Entropy, EntropyError};
    struct Broken;
    impl Entropy for Broken {
        fn fill(&self, _: &mut [u8]) -> Result<(), EntropyError> {
            Err(EntropyError)
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(
        backend_environment::router_with_entropy(std::sync::Arc::new(Broken)).into_backend(),
    )
    .data_directory(dir.path().canonicalize().unwrap())
    .installation(&"b".repeat(64), "Local")
    .capability("teamarness.membership", membership_support::present())
    .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
    .start()
    .await;
    h.activate().await.unwrap();
    let error = h
        .request("external-gateway.token.create", json!({"label":"client"}))
        .await
        .unwrap_err();
    assert_eq!(error.code(), "external_gateway.unavailable");
    assert_eq!(error.details(), &json!({}));
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["tokens"],
        json!([])
    );
    h.shutdown().await.unwrap();
}
