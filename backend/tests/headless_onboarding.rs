//! Headless operator sequence (AC9, Backend half) through declared requests only.
//!
//! Seams: the typed router runs in the SDK harness with a private injected home and Data
//! Directory, a scripted `teamarness.package-ports` Provider and the real HTTP listener.
//! The admitted source is a private copy of the unassembled `cli-fixture` executable, so
//! `install-cli` publishes real executable bytes at `<home>/.local/bin/tmx`. That installed
//! copy runs as a child process with `TEAMARNESS_FIXTURE_HOME` set to the injected home and
//! reaches the harness listener over numeric loopback. No subscription, picker or UI is used,
//! and nothing resolves, reads or installs into the ordinary home.
mod http_support;
use http_support::{Provider, get, membership_support};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use teamarness_generated_package_backend::{
    client_store::{ClientStore, Connection},
    managed_install::Admitted,
    router_with_environment,
    setup::BackendEnvironment,
};
use teamarness_package_contract::ui_contract::{CompiledUiContract, compile_ui_contract};
use teamarness_package_sdk::testing::{Harness, HarnessBuilder, ScriptedCapability};

const WORKSPACE: &str = "workspace1";
const INSTALLATION: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn cli_fixture() -> PathBuf {
    static EXE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| {
        let build = std::process::Command::new("cargo")
            .args([
                "build",
                "--locked",
                "-p",
                "teamarness-generated-package-backend",
                "--example",
                "cli-fixture",
            ])
            .output()
            .unwrap();
        assert!(build.status.success(), "fixture driver compilation failed");
        let metadata = std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&metadata.stdout).unwrap();
        Path::new(value["target_directory"].as_str().unwrap()).join("debug/examples/cli-fixture")
    })
    .clone()
}

struct Operator {
    harness: Harness,
    contract: CompiledUiContract,
    home: PathBuf,
}
impl Operator {
    /// One declared request. Every response is contract-valid and never carries a bearer.
    async fn request(&mut self, operation: &str, body: Value) -> Value {
        let response = self
            .harness
            .request(operation, body.clone())
            .await
            .unwrap_or_else(|e| panic!("{operation} {body}: {}", e.code()));
        self.contract
            .validate_response(operation, &response)
            .unwrap();
        assert!(
            !response.to_string().contains("eg1."),
            "{operation} response carries a bearer"
        );
        response
    }
    /// Poll `setup.status` until the accepted operation leaves its in-progress phases.
    ///
    /// The bound only guards against a hang. Connect and rotate hash the admitted and installed
    /// fixture bytes about eight times with debug-built SHA-256. On an x86_64 CPU without
    /// SHA-NI (factory2's i7-8550U) that runs at about 16 MB/s, so one operation takes about 10
    /// seconds there, against under 2 on Apple Silicon, which has SHA-2 instructions.
    async fn finish(&mut self) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                let s = self
                    .request("external-gateway.setup.status", json!({}))
                    .await;
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
    async fn setup(&mut self, intent: &str) -> Value {
        let plan = self
            .request("external-gateway.setup.preview", json!({"intent":intent}))
            .await;
        assert_eq!(plan["intent"], intent);
        assert_eq!(plan["skillChange"], "not-requested");
        let accepted = self
            .request(
                "external-gateway.setup.apply",
                json!({"planId":plan["planId"]}),
            )
            .await;
        let status = self.finish().await;
        assert_eq!(status["operationId"], accepted["operationId"]);
        assert_eq!(status["intent"], intent);
        assert_eq!(status["phase"], "complete", "{status}");
        assert_eq!(status["error"], Value::Null);
        status
    }
    async fn inspect(&mut self) -> Value {
        self.request("external-gateway.setup.inspect", json!({}))
            .await
    }
    async fn status(&mut self) -> Value {
        self.request("external-gateway.status.get", json!({})).await
    }
    fn connections(&self) -> Vec<Connection> {
        ClientStore::for_home(&self.home)
            .unwrap()
            .connections()
            .unwrap()
    }
    /// Execute the installed managed `tmx` copy against the injected home only.
    async fn tmx(&self, args: &[&str]) -> (Option<i32>, Value, String) {
        let output = tokio::process::Command::new(self.home.join(".local/bin/tmx"))
            .args(args)
            .env_clear()
            .env("TEAMARNESS_FIXTURE_HOME", &self.home)
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stdout.contains("eg1."), "tmx stdout carries a bearer");
        assert!(!stderr.contains("eg1."), "tmx stderr carries a bearer");
        let value = serde_json::from_str(&stdout).unwrap_or_else(|_| panic!("{stdout}{stderr}"));
        (output.status.code(), value, stderr)
    }
}

async fn start(data: &Path, home: &Path, admitted: &Path) -> Operator {
    let digest: String = Sha256::digest(std::fs::read(admitted).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let environment = BackendEnvironment::injected(home.to_path_buf()).with_admitted_source(
        admitted.to_path_buf(),
        Admitted::new(
            INSTALLATION,
            teamarness_generated_package_backend::PACKAGE_VERSION,
            &digest,
        )
        .unwrap(),
    );
    let provider = Arc::new(Mutex::new(Provider::new()));
    let ports = ScriptedCapability::new().calls(move |m, p| provider.lock().unwrap().call(m, p));
    let mut harness = HarnessBuilder::new(router_with_environment(environment).into_backend())
        .package_identity("dev.teamarness.external-gateway")
        .package_version(teamarness_generated_package_backend::PACKAGE_VERSION)
        .process_minor(1)
        .data_directory(data.to_path_buf())
        .installation(INSTALLATION, "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", ports, 3, 0)
        .start()
        .await;
    harness.activate().await.unwrap();
    Operator {
        harness,
        contract: compile_ui_contract(include_bytes!("../../ui-contract.json")).unwrap(),
        home: home.to_path_buf(),
    }
}

#[tokio::test]
async fn request_only_install_listen_select_connect_call_rotate_and_disconnect() {
    use std::os::unix::fs::PermissionsExt;
    // Owner-only modes are explicit, so the fixtures do not depend on the process umask
    // (Ubuntu's default 0002 would make them group-writable, which the stores refuse).
    let private = || {
        tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap()
    };
    let (data, home, staging) = (private(), private(), private());
    let (data_path, home_path) = (
        data.path().canonicalize().unwrap(),
        home.path().canonicalize().unwrap(),
    );
    // A private copy keeps the admitted bytes stable while other tests rebuild the example.
    // It is owner-only, as the Host admits Backend bytes; `copy` keeps Cargo's build mode.
    let admitted = staging.path().canonicalize().unwrap().join("tmx-fixture");
    #[cfg(not(target_os = "linux"))]
    std::fs::copy(cli_fixture(), &admitted).unwrap();
    // Test-only Linux adapter: an ELF debug build embeds its DWARF (about 147 MB here, against
    // 22 MB on macOS, which keeps it outside the binary). Every inspect, preview and apply
    // hashes the admitted bytes with debug-built SHA-256, which then overruns the bounded
    // setup poll. Admitting a debuginfo-stripped copy restores the intended fixture size.
    // binutils `strip` ships with the documented `build-essential` prerequisite.
    #[cfg(target_os = "linux")]
    assert!(
        std::process::Command::new("/usr/bin/strip")
            .arg("--strip-debug")
            .arg("-o")
            .args([&admitted, &cli_fixture()])
            .env_clear()
            .status()
            .unwrap()
            .success(),
        "strip the admitted fixture copy"
    );
    std::fs::set_permissions(&admitted, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut o = start(&data_path, &home_path, &admitted).await;

    // 1. Nothing is installed and this Workspace has no connection.
    let v = o.inspect().await;
    assert_eq!(v["cli"]["state"], "missing");
    assert_eq!(v["cli"]["change"], "install");
    assert_eq!(v["cli"]["issue"], Value::Null);
    assert_eq!(v["connection"]["state"], "not-connected");
    assert_eq!(v["connection"]["authority"], "none");
    assert_eq!(v["recovery"]["state"], "none");

    // 2. Install the CLI alone: no credential, listener or Policy change.
    let s = o.setup("install-cli").await;
    assert_eq!(s["command"], "installed");
    assert_eq!(s["access"], "unchanged");
    let v = o.inspect().await;
    assert_eq!(v["cli"]["state"], "installed");
    assert_eq!(v["cli"]["change"], "none");
    assert_eq!(v["cli"]["issue"], Value::Null);
    assert_eq!(
        v["cli"]["version"],
        teamarness_generated_package_backend::PACKAGE_VERSION
    );
    assert_eq!(v["connection"]["state"], "not-connected");
    assert_eq!(
        std::fs::read(home_path.join(".local/bin/tmx")).unwrap(),
        std::fs::read(&admitted).unwrap()
    );
    assert!(o.connections().is_empty());

    // 3. Setup never enables the listener; the operator does, explicitly.
    let st = o.status().await;
    assert_eq!(st["desiredEnabled"], false);
    assert_eq!(st["running"], false);
    assert_eq!(st["tokens"], json!([]));
    assert_eq!(
        o.harness
            .request(
                "external-gateway.setup.preview",
                json!({"intent":"connect"})
            )
            .await
            .unwrap_err()
            .code(),
        "external_gateway.listener_required"
    );
    o.request("external-gateway.listener.set", json!({"enabled":true}))
        .await;
    let st = o.status().await;
    assert_eq!(st["running"], true);
    let port = st["port"].as_u64().unwrap() as u16;

    // 4. Discover the catalog, then select one package and exactly one read tool.
    let page = o.request("external-gateway.catalog.page", json!({})).await;
    let row = &page["contracts"][0];
    assert_eq!(row["contract"], "dev.test.reads");
    assert_eq!(row["provider"], "dev.test.provider");
    assert_eq!(row["selected"], false);
    o.request(
        "external-gateway.package.set",
        json!({"provider":"dev.test.provider","enabled":true,"contracts":[{"contract":"dev.test.reads","major":1}]}),
    )
    .await;
    let tools = o
        .request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"}),
        )
        .await;
    let tools = tools["tools"].as_array().unwrap().clone();
    assert!(!tools.is_empty());
    assert!(
        tools.iter().all(|t| t["selected"] == false),
        "package enable alone selects no tool"
    );
    assert_eq!(o.status().await["selectedTools"], 0);
    let read = tools.iter().find(|t| t["port"] == "read.item").unwrap();
    assert_eq!(read["eligible"], true);
    o.request(
        "external-gateway.tool.set",
        json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"read.item","fingerprint":read["fingerprint"],"enabled":true}),
    )
    .await;
    assert_eq!(o.status().await["selectedTools"], 1);

    // 5. Connect this Workspace; the result reports current authority, not selections.
    let s = o.setup("connect").await;
    assert_eq!(s["access"], "configured");
    let v = o.inspect().await;
    assert_eq!(v["cli"]["state"], "installed");
    assert_eq!(v["cli"]["change"], "none");
    assert_eq!(v["connection"]["state"], "connected");
    assert_eq!(v["connection"]["authority"], "current");
    assert_eq!(v["connection"]["repair"], "none");
    assert_eq!(v["connection"]["issue"], Value::Null);
    let st = o.status().await;
    assert_eq!(st["selectedTools"], 1, "connect never expands selections");
    assert_eq!(st["tokens"].as_array().unwrap().len(), 1);
    assert_eq!(st["tokens"][0]["managed"], true);
    let first = o.connections().remove(0);
    assert_eq!(first.workspace_id, WORKSPACE);
    assert_eq!(first.port, port);

    // 6. The installed `tmx` sees this Workspace and performs one permitted call.
    let (code, workspaces, _) = o.tmx(&["workspaces"]).await;
    assert_eq!(code, Some(0));
    assert_eq!(workspaces["workspaces"][0]["id"], WORKSPACE);
    assert_eq!(workspaces["workspaces"][0]["port"], port);
    let (code, manifest, _) = o.tmx(&["--workspace", WORKSPACE, "status"]).await;
    assert_eq!(code, Some(0), "{manifest}");
    assert_eq!(manifest["workspace"]["id"], WORKSPACE);
    assert_eq!(manifest["reachable"], true);
    assert_eq!(manifest["endpoint"], format!("http://127.0.0.1:{port}"));
    let (code, call, _) = o
        .tmx(&[
            "--workspace",
            WORKSPACE,
            "call",
            "dev.test.reads",
            "read.item",
            "--major",
            "1",
            "--json",
            "{}",
        ])
        .await;
    assert_eq!(code, Some(0), "{call}");
    assert_eq!(call["result"], json!({"value":42}));
    // An unselected tool stays refused: setup authorized nothing by itself.
    let (code, refused, _) = o
        .tmx(&[
            "--workspace",
            WORKSPACE,
            "call",
            "dev.test.reads",
            "write.item",
            "--major",
            "1",
            "--json",
            "{}",
        ])
        .await;
    assert_eq!(code, Some(1), "{refused}");
    assert_eq!(refused["error"]["source"], "gateway");
    assert_eq!(refused["error"]["code"], "external_gateway.forbidden");
    assert_eq!(refused["error"]["outcomeUncertain"], false);

    // 7. Rotate: the old bearer is rejected over HTTP and `tmx` keeps working.
    let s = o.setup("rotate").await;
    assert_eq!(s["access"], "configured");
    let rotated = o.connections().remove(0);
    assert_eq!(rotated.token_id, first.token_id);
    assert_eq!(rotated.generation, first.generation + 1);
    assert_ne!(rotated.bearer, first.bearer);
    assert_eq!(get(port, &first.bearer, "/v1").await.0, 401);
    assert_eq!(get(port, &rotated.bearer, "/v1").await.0, 200);
    let (code, manifest, _) = o.tmx(&["--workspace", WORKSPACE, "status"]).await;
    assert_eq!(code, Some(0), "{manifest}");
    assert_eq!(manifest["reachable"], true);
    let v = o.inspect().await;
    assert_eq!(v["connection"]["state"], "connected");
    assert_eq!(v["connection"]["authority"], "current");

    // 8. Disconnect: the credential is revoked, the local record removed, `tmx` refuses.
    let s = o.setup("disconnect").await;
    assert_eq!(s["access"], "revoked");
    assert!(o.connections().is_empty());
    let v = o.inspect().await;
    assert_eq!(v["connection"]["state"], "not-connected");
    assert_eq!(v["connection"]["authority"], "disconnected");
    assert_eq!(v["cli"]["state"], "installed");
    assert_eq!(o.status().await["tokens"], json!([]));
    assert_eq!(get(port, &rotated.bearer, "/v1").await.0, 401);
    let (code, refused, stderr) = o.tmx(&["--workspace", WORKSPACE, "status"]).await;
    assert_eq!(code, Some(2), "{refused}");
    assert_eq!(refused["error"]["source"], "cli");
    assert_eq!(refused["error"]["code"], "cli.workspace_not_configured");
    assert_eq!(refused["error"]["retryable"], false);
    assert_eq!(refused["error"]["outcomeUncertain"], false);
    assert!(!stderr.is_empty());

    o.harness.shutdown().await.unwrap();
}
