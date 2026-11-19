//! Wire-entry regression: actual child argv/stdio, without production activation/home lookup.
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, Command},
};

const TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
fn host_args() -> [String; 2] {
    [
        format!("--teamarness-launch-token={TOKEN}"),
        "--teamarness-protocol=1".into(),
    ]
}
async fn send(child: &mut Child, frame: Value) {
    let bytes = serde_json::to_vec(&frame).unwrap();
    let input = child.stdin.as_mut().unwrap();
    input.write_u32(bytes.len() as u32).await.unwrap();
    input.write_all(&bytes).await.unwrap();
    input.flush().await.unwrap();
}
async fn receive(child: &mut Child) -> Value {
    tokio::time::timeout(Duration::from_secs(5), async {
        let out = child.stdout.as_mut().unwrap();
        let len = out
            .read_u32()
            .await
            .expect("SDK frame length, not a CLI error");
        assert!(len <= 1_048_576, "entry did not enter SDK stdio");
        let mut bytes = vec![0; len as usize];
        out.read_exact(&mut bytes).await.unwrap();
        assert!(!bytes.windows(TOKEN.len()).any(|b| b == TOKEN.as_bytes()));
        serde_json::from_slice(&bytes).unwrap()
    })
    .await
    .expect("bounded SDK response")
}
fn spawn(exe: &Path) -> Child {
    Command::new(exe)
        .args(host_args())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}
async fn initialize(child: &mut Child, data: &Path) {
    initialize_for(child, data, &"b".repeat(64), "entry-test", false).await;
}
async fn initialize_for(
    child: &mut Child,
    data: &Path,
    installation: &str,
    workspace: &str,
    activate: bool,
) {
    send(
        child,
        json!({
            "kind":"initialize", "nonce":"bm9uY2U", "package":{
                "identity":"dev.teamarness.external-gateway", "version":"1.1.0",
                "manifestDigest":"0".repeat(64), "backendDigest":"1".repeat(64)
            }, "uiContractDigest":"2".repeat(64), "processGeneration":"1",
            "workspace":{"id":workspace,"name":"Entry test"},
            "installation":{"identity":installation,"displayName":"Private test"},
            "dataDirectory":{"path":data,"continuityId":"Y29udGludWl0eQ"},
            "protocol":{"major":1,"minor":1},"capabilities":if activate {json!([
                {"identity":"teamarness.membership","grant":"bWVtYmVyc2hpcA","major":1,"minor":0},
                {"identity":"teamarness.package-ports","grant":"cG9ydHM","major":3,"minor":0}
            ])} else {json!([])},
            "limits":{"maxFrameBytes":1048576,"maxInFlightEachDirection":32,
                "maxWriterFrames":64,"maxWriterBytes":8388608,"initialStreamCredit":32}
        }),
    )
    .await;
    let response = receive(child).await;
    assert_eq!(response["kind"], "initialized");
    assert_eq!(response["nonce"], "bm9uY2U");
    assert_eq!(response["protocol"], json!({"major":1,"minor":1}));
}
async fn health_and_shutdown(mut child: Child) {
    send(&mut child, json!({"kind":"ping","nonce":"cGluZw"})).await;
    assert_eq!(
        receive(&mut child).await,
        json!({"kind":"pong","nonce":"cGluZw"})
    );
    send(&mut child, json!({"kind":"shutdown","reason":"entry_test"})).await;
    assert_eq!(receive(&mut child).await, json!({"kind":"shutdown_ack"}));
    assert!(
        tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .await
        .unwrap();
    assert!(stderr.is_empty());
}
#[tokio::test]
async fn production_accepts_actual_host_launch_without_activation() {
    let data = tempfile::tempdir().unwrap();
    let mut child = spawn(Path::new(env!(
        "CARGO_BIN_EXE_teamarness-generated-package-backend"
    )));
    initialize(&mut child, &data.path().canonicalize().unwrap()).await;
    health_and_shutdown(child).await;
    assert_eq!(std::fs::read_dir(data.path()).unwrap().count(), 0);
}
#[test]
fn malformed_duplicate_and_mixed_host_arguments_refuse_without_token_disclosure() {
    let exe = env!("CARGO_BIN_EXE_teamarness-generated-package-backend");
    let args = host_args();
    for invalid in [
        vec![args[0].as_str()],
        vec![args[1].as_str()],
        vec![args[1].as_str(), args[0].as_str()],
        vec![args[0].as_str(), args[1].as_str(), args[0].as_str()],
        vec![args[0].as_str(), args[1].as_str(), "extra"],
        vec![args[0].as_str(), "--teamarness-protocol=2"],
        vec!["--teamarness-launch-token=bad", args[1].as_str()],
        vec![
            "--teamarness-launch-token=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB",
            args[1].as_str(),
        ],
        vec!["--tmx-cli", args[0].as_str(), args[1].as_str()],
        vec!["--tmx-picker", args[0].as_str(), args[1].as_str()],
        vec![args[0].as_str(), args[1].as_str(), "--tmx-cli"],
    ] {
        let output = std::process::Command::new(exe)
            .args(invalid)
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        for bytes in [&output.stdout, &output.stderr] {
            assert!(!bytes.windows(TOKEN.len()).any(|b| b == TOKEN.as_bytes()));
        }
    }
}

fn private_root() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
fn fixture_executable() -> std::path::PathBuf {
    static EXE: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    EXE.get_or_init(|| {
        assert!(
            std::process::Command::new("cargo")
                .args([
                    "build",
                    "--locked",
                    "-p",
                    "teamarness-generated-package-backend",
                    "--example",
                    "backend-fixture"
                ])
                .status()
                .unwrap()
                .success()
        );
        let metadata = std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&metadata.stdout).unwrap();
        Path::new(value["target_directory"].as_str().unwrap())
            .join("debug/examples/backend-fixture")
    })
    .clone()
}
#[test]
fn fixture_requires_private_canonical_root_before_sdk_without_home_fallback() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = private_root();
    let root = dir.path().canonicalize().unwrap();
    let home = tempfile::tempdir().unwrap();
    let link = root.join("link");
    symlink(&root, &link).unwrap();
    let wrong_mode = root.join("public");
    std::fs::create_dir(&wrong_mode).unwrap();
    std::fs::set_permissions(&wrong_mode, std::fs::Permissions::from_mode(0o755)).unwrap();
    let file = root.join("file");
    std::fs::write(&file, b"not a root").unwrap();
    for invalid in [
        None,
        Some(std::path::PathBuf::from("relative")),
        Some(link),
        Some(wrong_mode),
        Some(file),
        Some(root.join("missing")),
        Some(root.join("..")),
    ] {
        let mut cmd = std::process::Command::new(fixture_executable());
        cmd.args(host_args())
            .env_clear()
            .env("TEAMARNESS_FIXTURE_HOME", home.path());
        if let Some(path) = invalid {
            cmd.env("TEAMARNESS_FIXTURE_ROOT", path);
        }
        let output = cmd.output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            !output
                .stderr
                .windows(TOKEN.len())
                .any(|b| b == TOKEN.as_bytes())
        );
    }
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}
async fn activate_fixture(child: &mut Child) -> Value {
    let deadline = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis()
        + 10000)
        .to_string();
    send(child, json!({"kind":"request","id":"h:activate","operation":"lifecycle.activate","deadlineUnixMs":deadline,"payload":{}})).await;
    loop {
        let frame = receive(child).await;
        if frame["kind"] == "request" {
            // Monitoring unavailable is sufficient: activation still opens the real coordinator,
            // with policy closed and no listener or grants. No Provider domain is simulated.
            assert_eq!(frame["operation"], "capability.call");
            send(child, json!({"kind":"error","id":frame["id"],"code":"fixture.unavailable","retryable":false,"details":{}})).await;
        } else {
            assert_eq!(frame["id"], "h:activate");
            return frame;
        }
    }
}
const HOME_B: &str = "a0fab1377f49a759b57f63318262ebe89fabfc990e8e93ceac2984561482b9d4";
const HOME_C: &str = "52b6419d27bd7f547cee3b92f8c17a908b8a49601ecbec161e5030de1dfe9e0a";
#[tokio::test]
async fn fixture_activation_shares_installation_home_across_workspaces_and_separates_installations()
{
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = private_root();
    let root = root.path().canonicalize().unwrap();
    let mut homes = Vec::new();
    let mut children = Vec::new();
    for (identity, workspace, name) in [
        ('b', "workspace-one", HOME_B),
        ('b', "workspace-two", HOME_B),
        ('c', "workspace-one", HOME_C),
    ] {
        let data = tempfile::tempdir().unwrap();
        let existed_before = root.join(name).exists();
        let mut child = Command::new(fixture_executable())
            .args(host_args())
            .env_clear()
            .env("TEAMARNESS_FIXTURE_ROOT", &root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        initialize_for(
            &mut child,
            &data.path().canonicalize().unwrap(),
            &identity.to_string().repeat(64),
            workspace,
            true,
        )
        .await;
        assert_eq!(
            root.join(name).exists(),
            existed_before,
            "SDK initialize must not create a home before activation"
        );
        assert_eq!(activate_fixture(&mut child).await["kind"], "response");
        let home = root.join(name);
        let metadata =
            std::fs::metadata(&home).expect("Installation home created at SDK activation");
        assert_eq!(metadata.permissions().mode() & 0o7777, 0o700);
        homes.push((metadata.dev(), metadata.ino()));
        children.push((child, data));
    }
    for (child, _data) in children {
        health_and_shutdown(child).await;
    }
    assert_eq!(homes[0], homes[1]);
    assert_ne!(homes[0], homes[2]);
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);
}
#[test]
fn copied_fixture_tmx_uses_only_explicit_child_home_and_shared_cli_logic() {
    use teamarness_generated_package_backend::client_store::{ClientStore, Connection};
    let root = private_root();
    let root = root.path().canonicalize().unwrap();
    let home = root.join(HOME_B);
    std::fs::create_dir(&home).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
    ClientStore::for_home(&home)
        .unwrap()
        .publish(&Connection {
            format_version: 1,
            workspace_id: "workspace-private".into(),
            workspace_name: "Private fixture".into(),
            port: 43127,
            installation_id: "b".repeat(64),
            continuity_id: "continuity".into(),
            origin_nonce: "a".repeat(64),
            token_id: "AAAAAAAAAAAAAAAAAAAAAA".into(),
            generation: 1,
            bearer: format!("eg1.{}.{}", "AAAAAAAAAAAAAAAAAAAAAA", "A".repeat(43)),
            operation_id: "operation".into(),
        })
        .unwrap();
    // Publish the exact fixture bytes through the real digest-checking managed-copy path.
    use sha2::{Digest, Sha256};
    use teamarness_generated_package_backend::managed_install::{
        Admitted, Approval, ManagedInstaller,
    };
    // Admit a private owner-only copy, as the Host admits Backend bytes: Cargo's output
    // and its directories follow the build umask (group-writable under Ubuntu's 0002).
    let source = root.join("admitted-fixture");
    std::fs::copy(fixture_executable(), &source).unwrap();
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o700)).unwrap();
    let digest: String = Sha256::digest(std::fs::read(&source).unwrap())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let installer = ManagedInstaller::for_home(&home).unwrap();
    let plan = installer
        .preview(
            &source,
            Admitted::new(&"b".repeat(64), "1.1.0", &digest).unwrap(),
        )
        .unwrap();
    installer.apply(&plan, Approval::Explicit).unwrap();
    let copy = home.join(".local/bin/tmx");
    assert_eq!(
        std::fs::read(&copy).unwrap(),
        std::fs::read(&source).unwrap()
    );
    let output = std::process::Command::new(&copy)
        .arg("workspaces")
        .env_clear()
        .env("TEAMARNESS_FIXTURE_HOME", &home)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"workspaces":[{"id":"workspace-private","name":"Private fixture","port":43127}]})
    );
    assert!(output.stderr.is_empty());
    for args in [
        vec![],
        vec!["--help"],
        vec!["--version"],
        vec!["--licenses"],
        vec!["--skill"],
    ] {
        let output = std::process::Command::new(&copy)
            .args(args)
            .env_clear()
            .env("TEAMARNESS_FIXTURE_HOME", &home)
            .output()
            .unwrap();
        assert!(output.status.success());
    }
    // The skill names the canonical invoked copy, never the managed home path it was copied from.
    let moved = root.join("agent's fixture dir");
    std::fs::create_dir(&moved).unwrap();
    // A separate `cp` process: this test process never holds a write descriptor to an
    // executable it runs, which a parallel test's spawning child could inherit (ETXTBSY).
    let status = std::process::Command::new("/bin/cp")
        .arg("--")
        .args([&copy, &moved.join("tmx")])
        .env_clear()
        .status()
        .unwrap();
    assert!(status.success());
    let expected = include_str!("../resources/teamarness/SKILL.md").replace(
        "{{COMMAND}}",
        &format!(
            "'{}'\\''s fixture dir/tmx'",
            root.join("agent").to_str().unwrap()
        ),
    );
    for args in [vec!["--skill"], vec!["--tmx-cli", "--skill"]] {
        let output = std::process::Command::new(moved.join("tmx"))
            .args(&args)
            .env_clear()
            .env("TEAMARNESS_FIXTURE_HOME", &home)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
    }
    let refused = std::process::Command::new(moved.join("tmx"))
        .args(["--skill", "extra"])
        .env_clear()
        .env("TEAMARNESS_FIXTURE_HOME", &home)
        .output()
        .unwrap();
    assert_eq!(refused.status.code(), Some(2));
    assert!(refused.stdout.starts_with(b"{\"error\":"));
    for missing_or_invalid in [
        None,
        Some(root.join("missing")),
        Some(root.join("..")),
        Some(std::path::PathBuf::from("relative")),
    ] {
        let mut cmd = std::process::Command::new(&copy);
        cmd.arg("workspaces")
            .env_clear()
            .env("TEAMARNESS_FIXTURE_ROOT", &root);
        if let Some(home) = missing_or_invalid {
            cmd.env("TEAMARNESS_FIXTURE_HOME", home);
        }
        let output = cmd.output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
    let output = std::process::Command::new(&copy)
        .args(host_args())
        .env_clear()
        .env("TEAMARNESS_FIXTURE_HOME", &home)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.starts_with(b"{\"error\":"));
    // Picker selection precedes fixture root/home handling and never opens a runtime.
    let picker = std::process::Command::new(fixture_executable())
        .arg("--tmx-picker")
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(picker.status.code(), Some(2));
    assert_eq!(
        picker.stdout,
        b"{\"code\":\"picker_request_invalid\",\"outcome\":\"error\",\"version\":1}\n"
    );
}

#[tokio::test]
async fn fixture_refuses_unsafe_children_and_replaced_pinned_root_without_repair() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for mode in ["symlink", "public", "file", "replaced-root"] {
        let owner = private_root();
        let parent = owner.path().canonicalize().unwrap();
        let root = parent.join("root");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let data = tempfile::tempdir().unwrap();
        let untouched = private_root();
        let home = root.join(HOME_B);
        match mode {
            "symlink" => symlink(untouched.path(), &home).unwrap(),
            "public" => {
                std::fs::create_dir(&home).unwrap();
                std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            "file" => std::fs::write(&home, b"preserve").unwrap(),
            _ => {}
        }
        let mut child = Command::new(fixture_executable())
            .args(host_args())
            .env_clear()
            .env("TEAMARNESS_FIXTURE_ROOT", &root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        initialize_for(
            &mut child,
            &data.path().canonicalize().unwrap(),
            &"b".repeat(64),
            "private",
            true,
        )
        .await;
        if mode == "replaced-root" {
            std::fs::rename(&root, parent.join("old-root")).unwrap();
            std::fs::create_dir(&root).unwrap();
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let terminal = activate_fixture(&mut child).await;
        assert_eq!(terminal["kind"], "error");
        assert_eq!(terminal["code"], "external_gateway.fixture_home_invalid");
        health_and_shutdown(child).await;
        assert_eq!(std::fs::read_dir(untouched.path()).unwrap().count(), 0);
        match mode {
            "symlink" => assert!(std::fs::symlink_metadata(&home).unwrap().is_symlink()),
            "public" => assert_eq!(
                home.metadata().unwrap().permissions().mode() & 0o7777,
                0o755
            ),
            "file" => assert_eq!(std::fs::read(&home).unwrap(), b"preserve"),
            _ => {
                assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
                assert_eq!(
                    std::fs::read_dir(parent.join("old-root")).unwrap().count(),
                    0
                );
            }
        }
    }
}
