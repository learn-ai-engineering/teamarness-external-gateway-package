//! Real children/private pipes + injected-home Backend; not native AppKit/manual qualification.
mod http_support;
use http_support::{Provider, membership_support};
#[path = "support/setup.rs"]
mod support;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use teamarness_package_sdk::testing::{Harness, HarnessSubscription};
const PICK: &str = "external-gateway.setup.choose-skill-folder";
fn executable() -> PathBuf {
    static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| {
        assert!(
            std::process::Command::new("cargo")
                .args([
                    "build",
                    "--locked",
                    "-p",
                    "teamarness-generated-package-backend",
                    "--example",
                    "picker-fixture"
                ])
                .status()
                .unwrap()
                .success()
        );
        let meta = std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .output()
            .unwrap();
        let v: Value = serde_json::from_slice(&meta.stdout).unwrap();
        Path::new(v["target_directory"].as_str().unwrap()).join("debug/examples/picker-fixture")
    })
    .clone()
}
fn fixture(mode: &str) -> support::Fixture {
    let mut f = support::Fixture::new();
    f.source = f.data.path().canonicalize().unwrap().join(mode);
    std::fs::copy(executable(), &f.source).unwrap();
    std::fs::create_dir(f.data.path().join("chosen")).unwrap();
    f
}
async fn result(
    s: &mut HarnessSubscription,
) -> Option<Result<Value, teamarness_package_sdk::BackendError>> {
    tokio::time::timeout(Duration::from_secs(10), s.next_event())
        .await
        .expect("bounded terminal")
}
async fn pid(f: &support::Fixture) -> u32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(s) = std::fs::read_to_string(f.source.with_extension("pid")) {
                return s.parse().unwrap();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn reaped(pid: u32) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let s = std::process::Command::new("/bin/ps")
                .args(["-p", &pid.to_string(), "-o", "stat="])
                .output()
                .unwrap();
            if !s.status.success() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owned helper reaped, not zombie");
}
async fn preview(h: &mut Harness, id: &Value) -> Value {
    h.request(
        "external-gateway.setup.preview",
        json!({"intent":"install-skill","selectionId":id}),
    )
    .await
    .unwrap()
}
#[tokio::test]
async fn child_faults_have_safe_errors_and_never_select() {
    for mode in [
        "malformed",
        "oversize",
        "extra",
        "nonzero",
        "stderr",
        "eof",
        "error",
    ] {
        let f = fixture(mode);
        let (mut h, _) = f.start().await;
        let mut s = h.subscribe(PICK, json!({})).await.unwrap();
        let e = result(&mut s).await.unwrap().unwrap_err();
        assert!(
            e.code().starts_with("external_gateway.picker_"),
            "{mode}: {e:?}"
        );
        assert_eq!(e.details(), &json!({}));
        let p = pid(&f).await;
        reaped(p).await;
        h.shutdown().await.unwrap();
    }
    let f = fixture("cancelled");
    let (mut h, _) = f.start().await;
    let mut s = h.subscribe(PICK, json!({})).await.unwrap();
    assert_eq!(
        result(&mut s).await.unwrap().unwrap(),
        json!({"outcome":"cancelled"})
    );
    assert!(result(&mut s).await.is_none());
    reaped(pid(&f).await).await;
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn admitted_digest_mismatch_refuses_before_spawn() {
    let f = fixture("selected");
    let env = f.environment();
    std::fs::write(&f.source, b"changed admitted bytes").unwrap();
    let (mut h, _) = f.start_with(env).await;
    let mut s = h.subscribe(PICK, json!({})).await.unwrap();
    assert_eq!(
        result(&mut s).await.unwrap().unwrap_err().code(),
        "external_gateway.reload_required"
    );
    assert!(!f.source.with_extension("pid").exists());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn exact_selection_lease_invalidates_unaccepted_plan_but_not_owned_apply() {
    let f = fixture("safe-stderr");
    let (mut h, _) = f.start().await;
    let mut s = h.subscribe(PICK, json!({})).await.unwrap();
    let selected = result(&mut s).await.unwrap().unwrap();
    assert_eq!(
        selected
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["outcome", "path", "selectionId"]
    );
    assert_eq!(
        selected["path"],
        f.data
            .path()
            .canonicalize()
            .unwrap()
            .join("chosen")
            .to_str()
            .unwrap()
    );
    reaped(pid(&f).await).await;
    let p = preview(&mut h, &selected["selectionId"]).await;
    s.close().await;
    tokio::time::sleep(Duration::from_millis(30)).await;
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
    assert!(
        h.request(
            "external-gateway.setup.preview",
            json!({"intent":"install-skill","selectionId":selected["selectionId"]})
        )
        .await
        .is_err()
    );
    let mut s = h.subscribe(PICK, json!({})).await.unwrap();
    let selected = result(&mut s).await.unwrap().unwrap();
    let p = preview(&mut h, &selected["selectionId"]).await;
    h.request(
        "external-gateway.setup.apply",
        json!({"planId":p["planId"]}),
    )
    .await
    .unwrap();
    s.close().await;
    assert_eq!(support::finish(&mut h).await["phase"], "complete");
    assert!(f.data.path().join("chosen/teamarness/SKILL.md").is_file());
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn four_bounded_selection_leases_release_children_and_capacity() {
    let f = fixture("selected");
    let (mut h, _) = f.start().await;
    let mut leases = vec![];
    for _ in 0..4 {
        let mut s = h.subscribe(PICK, json!({})).await.unwrap();
        result(&mut s).await.unwrap().unwrap();
        leases.push(s);
    }
    assert_eq!(
        h.subscribe_confirmed(PICK, json!({}))
            .await
            .err()
            .unwrap()
            .code(),
        "external_gateway.picker_busy"
    );
    leases[0].close().await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut s = h.subscribe(PICK, json!({})).await.unwrap();
    result(&mut s).await.unwrap().unwrap();
    s.close().await;
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn unsubscribe_suspend_dispose_kill_and_reap_without_late_selection() {
    for lifecycle in ["unsubscribe", "suspend", "dispose"] {
        let f = fixture("hold");
        let (mut h, _) = f.start().await;
        let mut s = h.subscribe(PICK, json!({})).await.unwrap();
        let p = pid(&f).await;
        assert_eq!(
            h.subscribe_confirmed(PICK, json!({}))
                .await
                .err()
                .unwrap()
                .code(),
            "external_gateway.picker_busy"
        );
        match lifecycle {
            "unsubscribe" => s.close().await,
            "suspend" => {
                h.suspend().await.unwrap();
            }
            _ => {
                h.dispose().await.unwrap();
            }
        }
        reaped(p).await;
        h.shutdown().await.unwrap();
    }
    // A released result racing close cannot retain an authorizing selection/preview.
    let f = fixture("late");
    let (mut h, state) = f.start().await;
    let mut s = h.subscribe(PICK, json!({})).await.unwrap();
    let p = pid(&f).await;
    s.close().await;
    std::fs::write(f.data.path().join("release"), b"go").unwrap();
    reaped(p).await;
    state.picker.stop().await;
    assert!(result(&mut s).await.is_none());
    for _ in 0..4 {
        state
            .setup
            .register_selection(f.data.path().canonicalize().unwrap().join("chosen"))
            .await
            .unwrap();
    }
    assert_eq!(
        state
            .setup
            .register_selection(f.data.path().canonicalize().unwrap().join("chosen"))
            .await
            .unwrap_err()
            .code(),
        "external_gateway.busy"
    );
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn pending_beyond_thirty_seconds_keeps_health_status_and_real_http_calls_responsive() {
    let f = fixture("hold");
    let provider = Arc::new(Mutex::new(Provider::new()));
    let scripted = provider.clone();
    let mut h = teamarness_package_sdk::testing::HarnessBuilder::new(
        teamarness_generated_package_backend::router_with_environment(f.environment())
            .into_backend(),
    )
    .process_minor(1)
    .data_directory(f.data.path().canonicalize().unwrap())
    .installation(&"b".repeat(64), "Local")
    .capability("teamarness.membership", membership_support::present())
    .capability_version(
        "teamarness.package-ports",
        teamarness_package_sdk::testing::ScriptedCapability::new()
            .calls(move |m, p| scripted.lock().unwrap().call(m, p)),
        3,
        0,
    )
    .start()
    .await;
    h.activate().await.unwrap();
    http_support::select(&mut h).await;
    let (_, bearer) = http_support::enable(&mut h).await;
    let started = std::time::Instant::now();
    let mut s = tokio::time::timeout(
        Duration::from_secs(1),
        h.subscribe_confirmed(PICK, json!({})),
    )
    .await
    .expect("prompt stream establishment")
    .unwrap();
    let p = pid(&f).await;
    for i in 0..5 {
        assert!(h.ping().await < Duration::from_secs(1));
        let status = h
            .request("external-gateway.status.get", json!({}))
            .await
            .unwrap();
        let reply = http_support::post(
            status["port"].as_u64().unwrap() as u16,
            &bearer,
            http_support::call(json!({})),
        )
        .await;
        assert_eq!(reply.0, 200);
        assert_eq!(reply.1["result"]["value"], 42, "{:?}", reply);
        if i < 4 {
            tokio::time::sleep(Duration::from_secs(8)).await;
        }
    }
    assert!(started.elapsed() > Duration::from_secs(30));
    assert_eq!(provider.lock().unwrap().invocations, 5);
    s.close().await;
    reaped(p).await;
    h.shutdown().await.unwrap();
}
#[tokio::test]
async fn parent_only_sigkill_closes_private_lifeline_without_appkit() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let f = fixture("hold");
    let mut parent = tokio::process::Command::new(&f.source)
        .arg("parent")
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut line)
        .await
        .unwrap();
    let p: u32 = line.trim().parse().unwrap();
    pid(&f).await;
    parent.start_kill().unwrap();
    parent.wait().await.unwrap();
    reaped(p).await;
}

#[tokio::test]
async fn selected_lease_suspension_invalidates_preview_and_explicit_recovery_can_choose_again() {
    let f = fixture("selected");
    let (mut h, _) = f.start().await;
    let mut s = h.subscribe_confirmed(PICK, json!({})).await.unwrap();
    let chosen = result(&mut s).await.unwrap().unwrap();
    let p = preview(&mut h, &chosen["selectionId"]).await;
    h.suspend().await.unwrap();
    h.recover().await.unwrap();
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
    let mut s = h.subscribe_confirmed(PICK, json!({})).await.unwrap();
    let fresh = result(&mut s).await.unwrap().unwrap();
    assert_ne!(fresh["selectionId"], chosen["selectionId"]);
    assert!(
        h.request(
            "external-gateway.setup.preview",
            json!({"intent":"install-skill","selectionId":chosen["selectionId"]})
        )
        .await
        .is_err()
    );
    s.close().await;
    h.shutdown().await.unwrap();
}
