use std::process::Command;
#[test]
fn help_and_version_are_offline() {
    let exe = env!("CARGO_BIN_EXE_teamarness-generated-package-backend");
    let help = Command::new(exe)
        .args(["--tmx-cli", "--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("tmx workspaces")
    );
    let version = Command::new(exe)
        .args(["--tmx-cli", "--version"])
        .output()
        .unwrap();
    assert!(version.status.success());
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        "tmx 1.0.0 (CLI 1)\n"
    );
}
#[path = "support/cli_support.rs"]
mod support;
use std::sync::atomic::Ordering;
use support::*;
#[tokio::test]
async fn target_required_even_for_one_workspace() {
    let f = Fixture::new(vec![response(MANIFEST.as_bytes())]).await;
    let result = f.run(&["status"]).await;
    assert_eq!(result.exit, 2);
    assert_eq!(result.bytes,br#"{"error":{"code":"cli.workspace_required","details":{},"outcomeUncertain":false,"retryable":false,"source":"cli"}}"#);
    let list = f.run(&["workspaces"]).await;
    assert_eq!(list.exit, 0);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&list.bytes).unwrap()["workspaces"][0]["id"],
        "workspace-a"
    );
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn id_wins_and_exact_names_must_be_unique() {
    use teamarness_generated_package_backend::client_store::ClientStore;
    let f = Fixture::new(vec![response(MANIFEST.as_bytes())]).await;
    let store = ClientStore::for_home(&f.home.path().canonicalize().unwrap()).unwrap();
    store.publish(&connection("workspace-b", "Acme")).unwrap();
    store
        .publish(&connection("workspace-c", "workspace-a"))
        .unwrap();
    let ambiguous = f.run(&["--workspace", "Acme", "status"]).await;
    assert_eq!(ambiguous.exit, 2);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&ambiguous.bytes).unwrap()["error"]["details"],
        serde_json::json!({"candidateIds":["workspace-a","workspace-b"]})
    );
    assert_eq!(f.run(&["--workspace", "acme", "status"]).await.exit, 2);
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.run(&["--workspace", "workspace-a", "status"]).await.exit,
        0
    );
    assert_eq!(f.count.load(Ordering::SeqCst), 1);
    assert_eq!(f.requests.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn json_sources_are_explicit_and_exclusive() {
    let f = Fixture::new(vec![response(MANIFEST.as_bytes())]).await;
    for words in [
        vec!["call", "dev.example.any", "item.create", "--major", "1"],
        vec![
            "call",
            "dev.example.any",
            "item.create",
            "--major",
            "1",
            "--json",
            "{}",
            "--input",
            "-",
        ],
        vec![
            "call",
            "dev.example.any",
            "item.create",
            "--major",
            "1",
            "--json",
            "{} {}",
        ],
        vec!["call", "dev.example.any", "item.create", "--json", "{}"],
        vec![
            "call",
            "dev.example.any",
            "item.create",
            "--major",
            "0",
            "--json",
            "{}",
        ],
        vec!["contracts", "--workspace", "Acme"],
    ] {
        let mut args = vec!["--workspace", "workspace-a"];
        args.extend(words);
        assert_eq!(f.run(&args).await.exit, 2);
    }
    let huge = " ".repeat(262145);
    assert_eq!(
        f.run(&[
            "--workspace",
            "Acme",
            "call",
            "dev.example.any",
            "item.create",
            "--major",
            "1",
            "--json",
            &huge
        ])
        .await
        .exit,
        2
    );
    assert_eq!(f.count.load(Ordering::SeqCst), 0);
    use teamarness_generated_package_backend::cli::args::parse;
    let file = f.home.path().join("request.json");
    std::fs::write(&file, b"[9007199254740993,1.2300e+04]").unwrap();
    for input in [file.to_str().unwrap(), "-"] {
        let args = [
            "--workspace",
            "Acme",
            "call",
            "dev.example.any",
            "item.create",
            "--major",
            "1",
            "--input",
            input,
        ]
        .map(String::from);
        let prepared = parse(&args, &mut &b"[9007199254740993,1.2300e+04]"[..]).unwrap();
        assert!(
            prepared
                .body
                .ends_with(b"\"request\":[9007199254740993,1.2300e+04]}")
        );
    }
}
#[test]
fn native_entry_modes_never_fall_back_to_host_stdio() {
    let exe = env!("CARGO_BIN_EXE_teamarness-generated-package-backend");
    for args in [
        vec!["bad"],
        vec!["--tmx-picker"],
        vec!["--tmx-cli", "subscribe"],
        vec!["--tmx-cli", "--workspace"],
    ] {
        let out = Command::new(exe).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.starts_with(b"{\"error\":"));
    }
    let dir = tempfile::tempdir().unwrap();
    let copy = dir.path().join("tmx");
    std::fs::copy(exe, &copy).unwrap();
    let out = Command::new(copy)
        .current_dir(dir.path())
        .env_clear()
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.starts_with("tmx —".as_bytes()));
}
#[tokio::test]
async fn os_home_adapter_rejects_errors_ambiguity_overflow_and_timeout_without_fallback() {
    use teamarness_generated_package_backend::cli::home::{lookup, parse};
    assert_eq!(
        parse(b"name: fixture\ndir: /private/tmp/fixture home\n").unwrap(),
        std::path::PathBuf::from("/private/tmp/fixture home")
    );
    for bytes in [
        b"dir: relative\n".as_slice(),
        b"dir: /a\ndir: /b\n",
        b"name: nobody\n",
        b"dir: /a\x00\n",
    ] {
        assert!(parse(bytes).is_err());
    }
    for script in [
        "printf 'dir: /private/tmp/fixture\\n'; exit 1",
        "printf 'dir: /a\\ndir: /b\\n'",
        "exec /bin/sleep 10",
        "/usr/bin/yes x | /usr/bin/head -c 16385",
    ] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            lookup(tokio::process::Command::new("/bin/sh").args(["-c", script])),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().code, "cli.home_unavailable");
    }
}
#[test]
fn cursor_target_and_input_depth_boundaries_reject_before_sockets() {
    use teamarness_generated_package_backend::cli::args::parse;
    let cursor = "A".repeat(2731);
    let words = ["--workspace", "Acme", "contracts", "--cursor", &cursor].map(String::from);
    assert!(parse(&words, &mut &b""[..]).is_ok());
    let words = [
        "--workspace",
        "Acme",
        "contracts",
        "--cursor",
        &(cursor + "A"),
    ]
    .map(String::from);
    assert!(parse(&words, &mut &b""[..]).is_err());
    let json = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    let words = [
        "--workspace",
        "Acme",
        "call",
        "dev.example.any",
        "item.create",
        "--major",
        "1",
        "--json",
        &json,
    ]
    .map(String::from);
    assert!(parse(&words, &mut &b""[..]).is_err());
    for flag in ["--url", "--endpoint", "--home", "--credential-path"] {
        let words = [
            "--workspace",
            "Acme",
            "status",
            flag,
            "/private/tmp/forbidden",
        ]
        .map(String::from);
        assert!(parse(&words, &mut &b""[..]).is_err());
    }
}
#[tokio::test]
async fn manifest_version_is_shared_with_cli_and_sdk_initialization() {
    use teamarness_package_sdk::{Router, testing::HarnessBuilder};
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../teamarness.package.json")).unwrap();
    let version = manifest["version"].as_str().unwrap();
    assert_eq!(
        teamarness_generated_package_backend::PACKAGE_VERSION,
        version
    );
    let router = Router::new(|activation| {
        let admitted = activation.initialization().package().version.clone();
        async move {
            assert_eq!(
                admitted,
                teamarness_generated_package_backend::PACKAGE_VERSION
            );
            Ok(())
        }
    });
    let data = tempfile::tempdir().unwrap();
    let mut h = HarnessBuilder::new(router.into_backend())
        .data_directory(data.path().canonicalize().unwrap())
        .package_version(version)
        .start()
        .await;
    h.activate().await.unwrap();
    h.shutdown().await.unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_teamarness-generated-package-backend"))
        .args(["--tmx-cli", "--version"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("tmx {version} (CLI 1)\n")
    );
}
