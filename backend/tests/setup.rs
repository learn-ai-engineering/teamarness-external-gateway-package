//! Managed access through the typed router with private physical homes, real Policy/stores.
mod membership_support;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use teamarness_generated_package_backend::{router_with_environment, setup::BackendEnvironment};
use teamarness_package_sdk::testing::{Harness, HarnessBuilder, ScriptedCapability};
async fn start(data: &std::path::Path, home: &std::path::Path) -> Harness {
    let source = data.join("source");
    std::fs::write(&source, b"fixture admitted executable").unwrap();
    let digest = Sha256::digest(b"fixture admitted executable")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let env = BackendEnvironment::injected(home.canonicalize().unwrap()).with_admitted_source(
        source.canonicalize().unwrap(),
        teamarness_generated_package_backend::managed_install::Admitted::new(
            &"b".repeat(64),
            "1.0.0",
            &digest,
        )
        .unwrap(),
    );
    let mut h = HarnessBuilder::new(router_with_environment(env).into_backend())
        .package_identity("dev.teamarness.external-gateway")
        .package_version(teamarness_generated_package_backend::PACKAGE_VERSION)
        .data_directory(data.canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", ScriptedCapability::new(), 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    h
}
async fn finish(h: &mut Harness) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let s = h
                .request("external-gateway.setup.status", json!({}))
                .await
                .unwrap();
            if !["accepted", "prepared", "outputs", "committed"]
                .contains(&s["phase"].as_str().unwrap())
            {
                return s;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn preview_is_read_only_and_connect_requires_committed_running_port() {
    let data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut h = start(data.path(), home.path()).await;
    assert_eq!(
        h.request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.listener_required"
    );
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn connect_idempotent_apply_rotation_disconnect_and_no_secret_status() {
    let data = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut h = start(data.path(), home.path()).await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"}),
        )
        .await
        .unwrap();
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
    let a = h
        .request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]}),
        )
        .await
        .unwrap();
    assert_eq!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]})
        )
        .await
        .unwrap(),
        a
    );
    let s = finish(&mut h).await;
    assert_eq!(s["phase"], "complete", "{s}");
    let store = teamarness_generated_package_backend::client_store::ClientStore::for_home(
        &home.path().canonicalize().unwrap(),
    )
    .unwrap();
    let c = store.connections().unwrap().remove(0);
    assert!(!s.to_string().contains(&c.bearer));
    assert_eq!(
        h.request(
            "external-gateway.token.regenerate",
            json!({"id":c.token_id})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.managed_setup_required"
    );
    let p = h
        .request("external-gateway.setup.preview", json!({"intent":"rotate"}))
        .await
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    assert_eq!(finish(&mut h).await["phase"], "complete");
    let n = store.connections().unwrap().remove(0);
    assert_eq!(n.token_id, c.token_id);
    assert_eq!(n.generation, c.generation + 1);
    assert_ne!(n.bearer, c.bearer);
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"disconnect"}),
        )
        .await
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    assert_eq!(finish(&mut h).await["access"], "revoked");
    assert!(store.connections().unwrap().is_empty());
    h.shutdown().await.unwrap();
}

#[path = "support/setup.rs"]
mod support;
#[tokio::test]
async fn four_previews_expire_and_policy_revision_or_destination_change_requires_new_consent() {
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
    let f = support::Fixture::new();
    let clock = Arc::new(Clock(AtomicU64::new(0)));
    let mut e = f.environment();
    e.clock = clock.clone();
    let (mut h, s) = f.start_with(e).await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    let mut p = Value::Null;
    for _ in 0..4 {
        p = h
            .request(
                "external-gateway.setup.preview",
                json!({"intent":"connect"}),
            )
            .await
            .unwrap();
    }
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
    clock.0.store(600, Ordering::SeqCst);
    assert_eq!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.setup_preview_stale"
    );
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"}),
        )
        .await
        .unwrap();
    h.request(
        "external-gateway.token.create",
        json!({"label":"new policy revision"}),
    )
    .await
    .unwrap();
    assert_eq!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.setup_preview_stale"
    );
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().canonicalize().unwrap().join("chosen");
    std::fs::create_dir(&path).unwrap();
    let id = s.setup.register_selection(path.clone()).await.unwrap();
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"connect","selectionId":id}),
        )
        .await
        .unwrap();
    std::fs::rename(&path, path.with_file_name("moved")).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]})
        )
        .await
        .is_err()
    );
    assert!(f.connections().is_empty());
    assert_eq!(std::fs::read_dir(f.home.path()).unwrap().count(), 0);
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn opaque_selection_only_closed_requests_and_exact_secret_free_status_watch() {
    let f = support::Fixture::new();
    let (mut h, s) = f.start().await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    for request in [
        json!({"intent":"connect","path":"/tmp"}),
        json!({"intent":"connect","bearer":"SECRET_SENTINEL"}),
        json!({"intent":"connect","overwrite":true}),
        json!({"intent":"connect","selectionId":"unknown"}),
    ] {
        assert!(
            h.request("external-gateway.setup.preview", request)
                .await
                .is_err()
        );
    }
    let folder = tempfile::tempdir().unwrap();
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
    assert_eq!(std::fs::read_dir(folder.path()).unwrap().count(), 0);
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    assert_eq!(support::finish(&mut h).await["skill"], "installed");
    let c = f.connections().remove(0);
    let mut watch = h
        .subscribe("external-gateway.setup.watch", json!({}))
        .await
        .unwrap();
    let event = watch.next_event().await.unwrap().unwrap();
    let mut keys = event
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "access",
            "command",
            "commandPath",
            "error",
            "intent",
            "operationId",
            "phase",
            "skill",
            "skillPath"
        ]
    );
    assert!(!event.to_string().contains(&c.bearer));
    assert!(!event.to_string().contains(&c.origin_nonce));
    assert!(
        !std::fs::read_to_string(folder.path().join("teamarness/SKILL.md"))
            .unwrap()
            .contains(&c.bearer)
    );
    watch.close().await;
    s.policy
        .change(teamarness_generated_package_backend::policy::Change::Revoke(c.token_id))
        .await
        .unwrap();
    let mut watch = h
        .subscribe("external-gateway.setup.watch", json!({}))
        .await
        .unwrap();
    assert_eq!(
        watch.next_event().await.unwrap().unwrap()["access"],
        "repair-required"
    );
    watch.close().await;
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn duplicate_origin_never_hands_over_and_disconnect_does_not_depend_on_command_integrity() {
    use teamarness_generated_package_backend::client_store::ClientStore;
    let f = support::Fixture::new();
    let (mut h, _) = f.start().await;
    support::connect(&mut h).await;
    let c = f.connections().remove(0);
    let other = support::Fixture::new();
    // Same home but independently initialized Data Directory: no inferred winner.
    // Environment constructor pins exact home; source provenance remains independent.
    let e = BackendEnvironment::injected(f.home.path().canonicalize().unwrap())
        .with_admitted_source(
            other.source.clone(),
            teamarness_generated_package_backend::managed_install::Admitted::new(
                &"b".repeat(64),
                "1.0.0",
                &teamarness_generated_package_backend::catalog::digest(
                    &std::fs::read(&other.source).unwrap(),
                ),
            )
            .unwrap(),
        );
    let (mut o, _) = other.start_with(e).await;
    o.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    assert_eq!(
        o.request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.connection_origin_conflict"
    );
    o.shutdown().await.unwrap();
    std::fs::write(
        f.home.path().join(".local/bin/tmx"),
        b"person modified command",
    )
    .unwrap();
    assert_eq!(
        support::apply(&mut h, "disconnect").await["access"],
        "revoked"
    );
    assert!(
        ClientStore::for_home(&f.home.path().canonicalize().unwrap())
            .unwrap()
            .connections()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read(f.home.path().join(".local/bin/tmx")).unwrap(),
        b"person modified command"
    );
    assert_eq!(
        h.request("external-gateway.status.get", json!({}))
            .await
            .unwrap()["tokens"],
        json!([])
    );
    assert!(!c.bearer.is_empty());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn real_activation_equal_noop_strict_newer_refresh_and_explicit_refusals() {
    let f = support::Fixture::new();
    let (mut h, _) = f.start().await;
    support::connect(&mut h).await;
    h.shutdown().await.unwrap();
    let command = f.home.path().join(".local/bin/tmx");
    let old = std::fs::metadata(&command).unwrap().modified().unwrap();
    let (h, _) = f.start().await;
    assert_eq!(
        std::fs::metadata(&command).unwrap().modified().unwrap(),
        old
    );
    h.shutdown().await.unwrap();
    std::fs::write(&f.source, b"admitted setup fixture v2").unwrap();
    let (h, _) = f.start_with(f.environment_version("1.1.0")).await;
    assert_eq!(
        std::fs::read(&command).unwrap(),
        b"admitted setup fixture v2"
    );
    h.shutdown().await.unwrap();
    for version in ["1.1.0", "1.0.0"] {
        std::fs::write(&f.source, b"different same-version or lower").unwrap();
        let (mut h, _) = f.start_with(f.environment_version(version)).await;
        assert_eq!(
            std::fs::read(&command).unwrap(),
            b"admitted setup fixture v2"
        );
        assert_eq!(
            h.request("external-gateway.setup.status", json!({}))
                .await
                .unwrap()["error"],
            "external_gateway.output_consent_required"
        );
        h.shutdown().await.unwrap();
    }
    std::fs::write(&command, b"user modified").unwrap();
    let (mut h, _) = f.start_with(f.environment_version("2.0.0")).await;
    assert_eq!(std::fs::read(&command).unwrap(), b"user modified");
    assert_eq!(
        h.request("external-gateway.setup.status", json!({}))
            .await
            .unwrap()["error"],
        "external_gateway.output_collision"
    );
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn policy_aba_is_stale_even_when_durable_authority_bytes_return_to_equal() {
    let f = support::Fixture::new();
    let (mut h, _) = f.start().await;
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    let before = std::fs::read(f.data.path().join("config/state.json")).unwrap();
    let p = h
        .request(
            "external-gateway.setup.preview",
            json!({"intent":"connect"}),
        )
        .await
        .unwrap();
    let token = h
        .request(
            "external-gateway.token.create",
            json!({"label":"temporary"}),
        )
        .await
        .unwrap();
    h.request("external-gateway.token.revoke", json!({"id":token["id"]}))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(f.data.path().join("config/state.json")).unwrap(),
        before
    );
    assert_eq!(
        h.request(
            "external-gateway.setup.apply",
            json!({"planId":p["planId"]})
        )
        .await
        .unwrap_err()
        .code(),
        "external_gateway.setup_preview_stale"
    );
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn local_record_deletion_is_not_revocation_and_never_recreates_a_credential() {
    let f = support::Fixture::new();
    let (mut h, s) = f.start().await;
    support::connect(&mut h).await;
    let c = f.connections().remove(0);
    let store = teamarness_generated_package_backend::client_store::ClientStore::for_home(
        &f.home.path().canonicalize().unwrap(),
    )
    .unwrap();
    store.lock().unwrap().remove_matching(&c).unwrap();
    s.policy.authenticate(&c.bearer).await.unwrap();
    h.shutdown().await.unwrap();
    drop(s);
    let (mut h, s) = f.start().await;
    assert!(f.connections().is_empty());
    s.policy.authenticate(&c.bearer).await.unwrap();
    assert!(
        h.request("external-gateway.setup.preview", json!({"intent":"repair"}))
            .await
            .is_err()
    );
    assert_eq!(s.policy.status()["tokens"].as_array().unwrap().len(), 1);
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn another_installation_activation_cannot_take_refresh_ownership() {
    let f = support::Fixture::new();
    let (mut h, _) = f.start().await;
    support::connect(&mut h).await;
    h.shutdown().await.unwrap();
    let command = f.home.path().join(".local/bin/tmx");
    let before = std::fs::read(&command).unwrap();
    let other = support::Fixture::new();
    std::fs::write(&other.source, b"other Installation admitted v2").unwrap();
    let owner = "c".repeat(64);
    let e = BackendEnvironment::injected(f.home.path().canonicalize().unwrap())
        .with_admitted_source(
            other.source.clone(),
            teamarness_generated_package_backend::managed_install::Admitted::new(
                &owner,
                "2.0.0",
                &teamarness_generated_package_backend::catalog::digest(
                    &std::fs::read(&other.source).unwrap(),
                ),
            )
            .unwrap(),
        );
    let (mut h, _) = other.start_as(e, &owner).await;
    assert_eq!(std::fs::read(&command).unwrap(), before);
    assert_eq!(
        h.request("external-gateway.setup.status", json!({}))
            .await
            .unwrap()["error"],
        "external_gateway.output_consent_required"
    );
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn install_skill_and_its_explicit_repair_need_no_listener_or_credential_authority() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use teamarness_generated_package_backend::managed_install::{Boundary, InstallFault};
    struct Fault(AtomicBool);
    impl InstallFault for Fault {
        fn before(&self, b: Boundary) -> std::io::Result<()> {
            if b == Boundary::CopyWrite && self.0.swap(false, Ordering::SeqCst) {
                Err(std::io::Error::other("output write"))
            } else {
                Ok(())
            }
        }
    }
    let f = support::Fixture::new();
    let folder = tempfile::tempdir().unwrap();
    let mut e = f.environment();
    e.install_fault = Some(Arc::new(Fault(AtomicBool::new(true))));
    let (mut h, s) = f.start_with(e).await;
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
    assert_eq!(p["credentialChange"], false);
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    assert_eq!(support::finish(&mut h).await["phase"], "repair-required");
    assert_eq!(support::apply(&mut h, "repair").await["phase"], "complete");
    assert!(folder.path().join("teamarness/SKILL.md").is_file());
    assert!(f.connections().is_empty());
    let snapshot = s.policy.setup_snapshot().await.unwrap();
    assert_eq!(snapshot.config.config_version, 1);
    assert!(!snapshot.config.desired_enabled);
    assert!(snapshot.config.tokens.is_empty());
    h.shutdown().await.unwrap();
}

#[tokio::test]
async fn explicit_preview_discloses_changed_command_bytes_and_repairs_without_token_rotation() {
    let f = support::Fixture::new();
    let (mut h, _) = f.start().await;
    support::connect(&mut h).await;
    let old = f.connections().remove(0);
    h.shutdown().await.unwrap();
    std::fs::write(&f.source, b"same version new admitted bytes").unwrap();
    let (mut h, _) = f.start().await;
    let p = h
        .request("external-gateway.setup.preview", json!({"intent":"repair"}))
        .await
        .unwrap();
    assert_eq!(p["commandChange"]["disposition"], "explicit-repair");
    assert_eq!(
        p["commandChange"]["installed"]["installation"],
        "b".repeat(64)
    );
    assert_eq!(p["commandChange"]["installed"]["version"], "1.0.0");
    assert_eq!(p["commandChange"]["proposed"]["version"], "1.0.0");
    assert_ne!(
        p["commandChange"]["installed"]["sha256"],
        p["commandChange"]["proposed"]["sha256"]
    );
    assert_eq!(p["credentialChange"], false);
    let contract = teamarness_package_contract::ui_contract::compile_ui_contract(include_bytes!(
        "../../ui-contract.json"
    ))
    .unwrap();
    contract
        .validate_response("external-gateway.setup.preview", &p)
        .unwrap();
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    let result = support::finish(&mut h).await;
    contract
        .validate_response("external-gateway.setup.status", &result)
        .unwrap();
    assert_eq!(result["phase"], "complete");
    assert_eq!(
        std::fs::read(f.home.path().join(".local/bin/tmx")).unwrap(),
        b"same version new admitted bytes"
    );
    let current = f.connections().remove(0);
    assert_eq!(current.generation, old.generation);
    assert_eq!(current.bearer, old.bearer);
    h.shutdown().await.unwrap();
}
