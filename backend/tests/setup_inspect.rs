//! Current facts and independent command-only setup through the real SDK/router and private homes.
mod membership_support;
#[path = "support/setup.rs"]
mod support;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};
use support::*;
use teamarness_generated_package_backend::client_store::ClientStore;
use teamarness_package_sdk::testing::Harness;

fn files(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, (u32, Vec<u8>)>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let m = fs::symlink_metadata(&p).unwrap();
            out.insert(
                p.strip_prefix(root).unwrap().into(),
                (
                    m.permissions().mode(),
                    if m.is_file() {
                        fs::read(&p).unwrap()
                    } else {
                        vec![]
                    },
                ),
            );
            if m.is_dir() {
                walk(root, &p, out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
async fn inspect(h: &mut Harness) -> Value {
    let v = h
        .request("external-gateway.setup.inspect", json!({}))
        .await
        .unwrap();
    let c = teamarness_package_contract::ui_contract::compile_ui_contract(include_bytes!(
        "../../ui-contract.json"
    ))
    .unwrap();
    c.validate_response("external-gateway.setup.inspect", &v)
        .unwrap();
    v
}
fn record_path(f: &Fixture, workspace: &str) -> PathBuf {
    f.home
        .path()
        .join(".local/share/teamarness/external-gateway")
        .join(ClientStore::filename(workspace))
}
#[tokio::test]
async fn cli_only_install_is_read_only_until_apply_and_has_no_credential_effects() {
    let f = Fixture::new();
    let (mut h, s) = f.start().await;
    let config = serde_json::to_value(s.policy.setup_snapshot().await.unwrap().config).unwrap();
    let data = files(f.data.path());
    let home = files(f.home.path());
    let v = inspect(&mut h).await;
    assert_eq!(v["cli"]["state"], "missing");
    assert_eq!(v["cli"]["change"], "install");
    assert_eq!(v["connection"]["state"], "not-connected");
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"install-cli"}),
        )
        .await
        .unwrap();
    assert_eq!(p["credentialChange"], false);
    assert_eq!(p["skillChange"], "not-requested");
    assert_eq!(files(f.home.path()), home);
    assert_eq!(files(f.data.path()), data);
    let a = h
        .request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]}),
        )
        .await
        .unwrap();
    assert_eq!(finish(&mut h).await["phase"], "complete");
    assert_eq!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]})
        )
        .await
        .unwrap(),
        a
    );
    assert_eq!(apply(&mut h, "install-cli").await["access"], "unchanged");
    assert_eq!(
        serde_json::to_value(s.policy.setup_snapshot().await.unwrap().config).unwrap(),
        config
    );
    assert_eq!(files(f.data.path()), data);
    assert!(f.connections().is_empty());
    assert!(
        !f.home
            .path()
            .join(".local/share/teamarness/external-gateway/setup")
            .exists()
    );
    let before = files(f.home.path());
    let v = inspect(&mut h).await;
    assert_eq!(v["cli"]["state"], "installed");
    assert_eq!(v["cli"]["change"], "none");
    assert_eq!(files(f.home.path()), before);
    assert!(
        h.request(
            "external-gateway.setup.preview",
            json!({"intent":"install-cli","selectionId":"unknown"})
        )
        .await
        .is_err()
    );
    assert!(
        h.request("external-gateway.setup.inspect", json!({"extra":true}))
            .await
            .is_err()
    );
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn malformed_future_and_conflicting_private_records_do_not_block_cli_or_get_rewritten() {
    for kind in ["malformed", "future", "origin"] {
        let f = Fixture::new();
        let (mut h, _) = f.start().await;
        connect(&mut h).await;
        let c = f.connections().remove(0);
        h.shutdown().await.unwrap();
        let path = record_path(&f, &c.workspace_id);
        let mut v = serde_json::to_value(&c).unwrap();
        match kind {
            "malformed" => {
                fs::write(&path, b"{broken").unwrap();
            }
            "future" => {
                v["formatVersion"] = json!(99);
                fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
            }
            _ => {
                v["installationId"] = json!("another-installation");
                fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
            }
        }
        let credentials = fs::read(&path).unwrap();
        let data = files(f.data.path());
        let (mut h, _) = f.start().await;
        let v = inspect(&mut h).await;
        assert_ne!(v["connection"]["state"], "connected");
        assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
        assert_eq!(fs::read(&path).unwrap(), credentials);
        assert_eq!(files(f.data.path()), data);
        h.shutdown().await.unwrap();
    }
}
#[tokio::test]
async fn connection_truth_survives_listener_stop_skill_and_cli_history_but_detects_local_loss() {
    let f = Fixture::new();
    let (mut h, s) = f.start().await;
    connect(&mut h).await;
    let c = f.connections().remove(0);
    h.request("external-gateway.listener.set", json!({"enabled":false}))
        .await
        .unwrap();
    assert_eq!(inspect(&mut h).await["connection"]["state"], "connected");
    let folder = support::private_dir();
    let id = s
        .setup
        .register_selection(folder.path().canonicalize().unwrap())
        .await
        .unwrap();
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"install-skill","selectionId":id}),
        )
        .await
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    assert_eq!(finish(&mut h).await["phase"], "complete");
    assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
    assert_eq!(inspect(&mut h).await["connection"]["state"], "connected");
    let before = files(f.home.path());
    let v = inspect(&mut h).await;
    assert_eq!(files(f.home.path()), before);
    for secret in [
        &c.bearer,
        &c.origin_nonce,
        &c.token_id,
        &c.operation_id,
        &c.workspace_id,
        &c.continuity_id,
        &c.installation_id,
    ] {
        assert!(!v.to_string().contains(secret));
    }
    h.shutdown().await.unwrap();
    let (mut h, s) = f.start().await;
    assert_eq!(inspect(&mut h).await["connection"]["state"], "connected");
    let store = ClientStore::for_home(&f.home.path().canonicalize().unwrap()).unwrap();
    store.lock().unwrap().remove_matching(&c).unwrap();
    let v = inspect(&mut h).await;
    assert_eq!(v["connection"]["authority"], "current");
    assert_eq!(v["connection"]["repair"], "replace-credentials");
    s.policy
        .change(teamarness_generated_package_backend::policy::Change::Revoke(c.token_id))
        .await
        .unwrap();
    let v = inspect(&mut h).await;
    assert_eq!(v["connection"]["authority"], "revoked");
    assert_eq!(v["connection"]["state"], "not-connected");
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn connection_inspection_distinguishes_publication_replacement_conflict_and_tombstones() {
    for kind in [
        "port",
        "bearer",
        "generation",
        "operation",
        "origin",
        "continuity",
        "installation",
        "future",
        "disconnect",
    ] {
        let f = Fixture::new();
        let (mut h, _) = f.start().await;
        connect(&mut h).await;
        let c = f.connections().remove(0);
        let path = record_path(&f, &c.workspace_id);
        let mut v = serde_json::to_value(&c).unwrap();
        match kind {
            "port" => v["port"] = json!(if c.port == 65535 { 65534 } else { c.port + 1 }),
            "bearer" => {
                let last = v["bearer"].as_str().unwrap();
                let mut b = last.as_bytes().to_vec();
                b[30] = if b[30] == b'A' { b'B' } else { b'A' };
                v["bearer"] = json!(String::from_utf8(b).unwrap());
            }
            "generation" => v["generation"] = json!(c.generation + 1),
            "operation" => v["operationId"] = json!("f".repeat(64)),
            "origin" => v["originNonce"] = json!("f".repeat(64)),
            "continuity" => v["continuityId"] = json!("another"),
            "installation" => v["installationId"] = json!("another"),
            "future" => v["formatVersion"] = json!(2),
            "disconnect" => {
                assert_eq!(apply(&mut h, "disconnect").await["phase"], "complete");
            }
            _ => unreachable!(),
        }
        if kind != "disconnect" {
            fs::write(&path, serde_json::to_vec(&v).unwrap()).unwrap();
        }
        let home = files(f.home.path());
        let data = files(f.data.path());
        let fact = inspect(&mut h).await;
        let connection = &fact["connection"];
        match kind {
            "port" | "operation" => {
                assert_eq!(connection["state"], "needs-repair");
                assert_eq!(connection["repair"], "publication");
            }
            "bearer" | "generation" => {
                assert_eq!(connection["state"], "needs-repair");
                assert_eq!(connection["repair"], "replace-credentials");
            }
            "future" => assert_eq!(connection["state"], "unavailable"),
            "disconnect" => {
                assert_eq!(connection["state"], "not-connected");
                assert_eq!(connection["authority"], "disconnected");
            }
            _ => assert_eq!(connection["state"], "conflict"),
        }
        assert_eq!(files(f.home.path()), home);
        assert_eq!(files(f.data.path()), data);
        h.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cli_inspection_keeps_recognition_when_candidate_differs_or_is_invalid() {
    for kind in [
        "older",
        "other-owner",
        "candidate-mismatch",
        "edited",
        "missing-receipt",
        "missing-command",
        "future",
        "mode",
    ] {
        let f = Fixture::new();
        let (mut h, _) = f.start().await;
        assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
        h.shutdown().await.unwrap();
        let mut e = f.environment();
        let command = f.home.path().join(".local/bin/tmx");
        let receipt = f
            .home
            .path()
            .join(".local/share/teamarness/external-gateway/managed/command.receipt");
        match kind {
            "older" => {
                fs::write(&f.source, b"new candidate").unwrap();
                e = f.environment_version("0.9.0");
            }
            "other-owner" => {
                e = teamarness_generated_package_backend::setup::BackendEnvironment::injected(
                    f.home.path().canonicalize().unwrap(),
                )
                .with_admitted_source(
                    f.source.clone(),
                    teamarness_generated_package_backend::managed_install::Admitted::new(
                        "other",
                        "1.0.0",
                        &teamarness_generated_package_backend::catalog::digest(
                            &fs::read(&f.source).unwrap(),
                        ),
                    )
                    .unwrap(),
                );
            }
            "candidate-mismatch" => fs::write(&f.source, b"not admitted").unwrap(),
            "edited" => fs::write(&command, b"edited").unwrap(),
            "missing-receipt" => fs::remove_file(&receipt).unwrap(),
            "missing-command" => fs::remove_file(&command).unwrap(),
            "future" => {
                let mut r: Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
                r["formatVersion"] = json!(2);
                fs::write(&receipt, serde_json::to_vec(&r).unwrap()).unwrap();
            }
            "mode" => fs::set_permissions(&command, fs::Permissions::from_mode(0o777)).unwrap(),
            _ => unreachable!(),
        }
        let (mut h, _) = f.start_with(e).await;
        let before = files(f.home.path());
        let v = inspect(&mut h).await;
        match kind {
            "older" | "other-owner" => {
                assert_eq!(v["cli"]["state"], "installed");
                assert_eq!(v["cli"]["change"], "explicit-repair");
            }
            "candidate-mismatch" => {
                assert_eq!(v["cli"]["state"], "installed");
                assert_eq!(v["cli"]["version"], "1.0.0");
                assert_eq!(v["cli"]["issue"], "external_gateway.reload_required");
            }
            _ => {
                assert_eq!(v["cli"]["state"], "blocked");
                assert!(v["cli"]["version"].is_null());
            }
        }
        assert_eq!(files(f.home.path()), before);
        h.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn cli_only_preserves_private_history_and_manual_policy_even_with_credential_lock_held() {
    let f = Fixture::new();
    let (mut h, _) = f.start().await;
    h.request("external-gateway.token.create", json!({"label":"manual"}))
        .await
        .unwrap();
    let store = ClientStore::for_home(&f.home.path().canonicalize().unwrap()).unwrap();
    let lock = store.lock().unwrap();
    let data = files(f.data.path());
    assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
    assert_eq!(files(f.data.path()), data);
    drop(lock);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn setup_collision_errors_identify_command_or_skill_without_private_metadata() {
    let f = Fixture::new();
    let (mut h, s) = f.start().await;
    assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
    let folder = support::private_dir();
    support::create_private_dir(folder.path().join("teamarness")).unwrap();
    fs::write(
        folder.path().join("teamarness/SKILL.md"),
        b"unrecognized skill",
    )
    .unwrap();
    let id = s
        .setup
        .register_selection(folder.path().canonicalize().unwrap())
        .await
        .unwrap();
    let e = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"install-skill","selectionId":id}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.details(), &json!({"output":"skill"}));
    fs::write(
        f.home.path().join(".local/bin/tmx"),
        b"unrecognized command",
    )
    .unwrap();
    let e = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"install-cli"}),
        )
        .await
        .unwrap_err();
    assert_eq!(e.details(), &json!({"output":"command"}));
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn inspection_never_creates_or_reclaims_private_temporary_files() {
    let f = Fixture::new();
    let (mut h, _) = f.start().await;
    connect(&mut h).await;
    use std::os::unix::fs::PermissionsExt;
    for path in [
        f.home.path().join(".local/bin/.tmx.tmp"),
        f.home
            .path()
            .join(".local/share/teamarness/external-gateway/record.tmp"),
        f.data.path().join("config/state.tmp"),
    ] {
        fs::write(&path, b"abandoned but never promoted").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let home = files(f.home.path());
    let data = files(f.data.path());
    assert_eq!(inspect(&mut h).await["connection"]["state"], "connected");
    assert_eq!(files(f.home.path()), home);
    assert_eq!(files(f.data.path()), data);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn cli_only_preserves_v1_saved_endpoint_desired_grants_and_manual_generation() {
    use teamarness_generated_package_backend::config::{
        Config, ContractSelection, FileStore, ToolSelection,
    };
    let f = Fixture::new();
    let (manual, _) = teamarness_generated_package_backend::tokens::issue(
        "manual",
        7,
        None,
        &teamarness_generated_package_backend::tokens::OsEntropy,
    )
    .unwrap();
    let mut golden = Config {
        port: Some(45123),
        desired_enabled: false,
        tokens: vec![manual],
        ..Config::default()
    };
    golden.contracts.insert(
        "dev.test.reads".into(),
        ContractSelection {
            major: 1,
            provider: "dev.test.provider".into(),
            enabled: true,
            tools: BTreeMap::from([(
                "read.item".into(),
                ToolSelection {
                    fingerprint: "a".repeat(64),
                },
            )]),
        },
    );
    let store = FileStore::open(&f.data.path().canonicalize().unwrap()).unwrap();
    store.write(&golden).unwrap();
    let (mut h, _) = f.start().await;
    let before = files(f.data.path());
    assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
    assert_eq!(files(f.data.path()), before);
    assert_eq!(
        serde_json::to_value(store.read().unwrap()).unwrap(),
        serde_json::to_value(golden).unwrap()
    );
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn cli_only_admission_ignores_failed_policy_snapshot_and_inspect_closes_connection_fact() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use teamarness_generated_package_backend::config::{StorageFault, WriteStage};
    struct Fail(AtomicBool);
    impl StorageFault for Fail {
        fn before(&self, _: WriteStage) -> std::io::Result<()> {
            if self.0.swap(false, Ordering::SeqCst) {
                Err(std::io::Error::other("policy write fault"))
            } else {
                Ok(())
            }
        }
    }
    let f = Fixture::new();
    let fault = Arc::new(Fail(AtomicBool::new(false)));
    let mut e = f.environment();
    e.storage_fault = Some(fault.clone());
    let (mut h, s) = f.start_with(e).await;
    let token = h
        .request("external-gateway.token.create", json!({"label":"manual"}))
        .await
        .unwrap();
    fault.0.store(true, Ordering::SeqCst);
    assert!(
        h.request("external-gateway.token.revoke", json!({"id":token["id"]}))
            .await
            .is_err()
    );
    assert!(s.policy.setup_snapshot().await.is_err());
    let before = files(f.data.path());
    assert_eq!(apply(&mut h, "install-cli").await["phase"], "complete");
    let v = inspect(&mut h).await;
    assert_eq!(v["cli"]["state"], "installed");
    assert_eq!(v["connection"]["authority"], "unavailable");
    assert_eq!(files(f.data.path()), before);
    assert!(s.policy.setup_snapshot().await.is_err());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn unavailable_membership_does_not_erase_durable_connection() {
    use teamarness_package_sdk::testing::HarnessBuilder;
    let f = Fixture::new();
    let (mut h, _) = f.start().await;
    connect(&mut h).await;
    h.shutdown().await.unwrap();
    let mut h = HarnessBuilder::new(
        teamarness_generated_package_backend::router_with_environment(f.environment())
            .into_backend(),
    )
    .process_minor(1)
    .package_identity("dev.teamarness.external-gateway")
    .data_directory(f.data.path().canonicalize().unwrap())
    .installation(&"b".repeat(64), "Local")
    .capability(
        "teamarness.membership",
        teamarness_package_sdk::testing::ScriptedCapability::new().calls(|_, _| {
            Err(teamarness_package_sdk::BackendError::new(
                "membership.unavailable",
                false,
                json!({}),
            ))
        }),
    )
    .capability_version(
        "teamarness.package-ports",
        teamarness_package_sdk::testing::ScriptedCapability::new(),
        3,
        0,
    )
    .start()
    .await;
    h.activate().await.unwrap();
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["running"],
        false
    );
    assert_eq!(inspect(&mut h).await["connection"]["state"], "connected");
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn cli_only_plans_keep_capacity_expiry_and_exact_source_rechecks() {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };
    struct Clock(AtomicU64);
    impl teamarness_generated_package_backend::setup::Clock for Clock {
        fn seconds(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    let f = Fixture::new();
    let clock = Arc::new(Clock(AtomicU64::new(0)));
    let mut e = f.environment();
    e.clock = clock.clone();
    let (mut h, _) = f.start_with(e).await;
    let mut first = json!(null);
    for _ in 0..4 {
        first = h
            .request(
                "external-gateway.setup.preview",
                json!({"intent":"install-cli"}),
            )
            .await
            .unwrap();
    }
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
    clock.0.store(601, Ordering::SeqCst);
    assert_eq!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":first["planId"]})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.setup_preview_stale"
    );
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"install-cli"}),
        )
        .await
        .unwrap();
    fs::write(&f.source, b"changed after preview").unwrap();
    assert_eq!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.reload_required"
    );
    assert!(files(f.home.path()).is_empty());
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_origin_inspection_is_unavailable_without_denying_existing_policy_access() {
    let f = Fixture::new();
    let (mut h, s) = f.start().await;
    connect(&mut h).await;
    let c = f.connections().remove(0);
    assert!(s.policy.authenticate(&c.bearer).await.is_ok());
    fs::write(f.data.path().join("config/origin"), b"invalid-origin").unwrap();
    let home = files(f.home.path());
    let data = files(f.data.path());
    let fact = inspect(&mut h).await;
    assert_eq!(
        fact["connection"],
        json!({"state":"unavailable", "authority":"current", "repair":"none", "issue":"external_gateway.storage_failed"})
    );
    let status = h
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["storageFailed"], false);
    assert!(s.policy.authenticate(&c.bearer).await.is_ok());
    assert_eq!(files(f.home.path()), home);
    assert_eq!(files(f.data.path()), data);
    h.shutdown().await.unwrap();
}
