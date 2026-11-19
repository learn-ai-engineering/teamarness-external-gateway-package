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
        "tmx 1.1.0 (CLI 1)\n"
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
        vec!["--tmx-picker", "extra"],
        vec!["--tmx-cli", "subscribe"],
        vec!["--tmx-cli", "--workspace"],
    ] {
        let out = Command::new(exe).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2));
        assert!(out.stdout.starts_with(b"{\"error\":"));
    }
    let helper = Command::new(exe).arg("--tmx-picker").output().unwrap();
    assert_eq!(helper.status.code(), Some(2));
    assert_eq!(
        helper.stdout,
        b"{\"code\":\"picker_request_invalid\",\"outcome\":\"error\",\"version\":1}\n"
    );
    assert!(helper.stderr.is_empty());
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
fn linux_getent_parser_accepts_only_one_exact_effective_user_entry() {
    // Pure parser; runs on every platform and never consults the real account database.
    use teamarness_generated_package_backend::cli::home::parse_getent;
    assert_eq!(
        parse_getent(
            b"fixture:x:501:20:Fixture:/home/fixture user:/bin/sh\n",
            501
        )
        .unwrap(),
        std::path::PathBuf::from("/home/fixture user")
    );
    assert_eq!(
        parse_getent(b"root:x:0:0::/:/bin/sh", 0).unwrap(),
        std::path::PathBuf::from("/")
    );
    for (bytes, uid) in [
        (
            b"fixture:x:502:20:Fixture:/home/fixture:/bin/sh\n".as_slice(),
            501,
        ),
        (b"fixture:x:0501:20:Fixture:/home/fixture:/bin/sh\n", 501),
        (b"fixture:x:+501:20:Fixture:/home/fixture:/bin/sh\n", 501),
        (
            b"fixture:x:501:20:Fixture:/home/a:/bin/sh\nfixture:x:501:20:Fixture:/home/b:/bin/sh\n",
            501,
        ),
        (b"fixture:x:501:20:Fixture:/home/a:/bin/sh\n\n", 501),
        (b"\nfixture:x:501:20:Fixture:/home/a:/bin/sh\n", 501),
        (b"fixture:x:501:20:Fixture:/home/fixture\n", 501),
        (
            b"fixture:x:501:20:Fixture:/home/fixture:/bin/sh:extra\n",
            501,
        ),
        (b"fixture:x:501:20:Fixture:home/fixture:/bin/sh\n", 501),
        (b"fixture:x:501:20:Fixture::/bin/sh\n", 501),
        (b"fixture:x:501:20:Fixture:/home/a\x00b:/bin/sh\n", 501),
        (b"fixture:x:501:20:Fixture:/home/a\tb:/bin/sh\n", 501),
        (b"fixture:x:501:20:Fixture:/home/a\x7f:/bin/sh\n", 501),
        (b"", 501),
        (b"\n", 501),
        (b"fixture:x:501:20:\xff:/home/fixture:/bin/sh\n", 501),
        // The macOS dscacheutil shape is never accepted by the Linux parser.
        (b"name: fixture\ndir: /home/fixture\n", 501),
    ] {
        assert_eq!(
            parse_getent(bytes, uid).unwrap_err().code,
            "cli.home_unavailable",
            "{bytes:?}"
        );
    }
}
#[tokio::test]
async fn linux_getent_lookup_rejects_exit_overflow_and_timeout_without_fallback() {
    use teamarness_generated_package_backend::cli::home::{lookup_with, parse_getent};
    let parser = |bytes: &[u8]| parse_getent(bytes, 501);
    let ok = lookup_with(
        tokio::process::Command::new("/bin/sh")
            .args(["-c", "printf 'f:x:501:20::/home/f:/bin/sh\\n'"])
            .env_clear(),
        parser,
    )
    .await
    .unwrap();
    assert_eq!(ok, std::path::PathBuf::from("/home/f"));
    for script in [
        "printf 'f:x:501:20::/home/f:/bin/sh\\n'; exit 2",
        "exec /bin/sleep 10",
        "/usr/bin/yes x | /usr/bin/head -c 16385",
        "/usr/bin/yes x | /usr/bin/head -c 4097 >&2; printf 'f:x:501:20::/home/f:/bin/sh\\n'",
    ] {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            lookup_with(
                tokio::process::Command::new("/bin/sh")
                    .args(["-c", script])
                    .env_clear(),
                parser,
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().code, "cli.home_unavailable", "{script}");
    }
}
#[test]
fn notices_cover_linux_only_rustix_backend_and_state_both_targets() {
    let text = teamarness_generated_package_backend::cli::third_party_notices();
    assert!(text.contains(
        "Component list: locked normal production Backend dependencies for both supported targets, aarch64-apple-darwin and x86_64-unknown-linux-gnu"
    ));
    assert!(text.contains("A given native binary may not contain every listed component."));
    let entry = "linux-raw-sys 0.12.1\n\
License: Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT\n\
Repository: https://github.com/sunfishcode/linux-raw-sys\n\
Authors (upstream metadata): Dan Gohman <dev@sunfishcode.online>\n\
Notice source: https://crates.io/crates/linux-raw-sys/0.12.1; registry source checksum in Cargo.lock; COPYRIGHT, LICENSE-APACHE, LICENSE-Apache-2.0_WITH_LLVM-exception, LICENSE-MIT; Linux-only rustix backend (x86_64-unknown-linux-gnu)\n\
Literal notice SHA-256: 3290ae0fbc9ddb77d2239121d710f0bb9d31b3b4744e6d97fe01e652b4c1870b, a60eea817514531668d7e00765731449fe14d059d3249e0bc93b36de45f759f2, 268872b9816f90fd8e85db5a28d33f8150ebb8dd016653fb39ef1f94f2686bc5, 23f18e03dc49df91622fe2a76176497404e46ced8a715d9d2b67a7446571cca3\n";
    assert!(text.contains(entry));
    // Its COPYRIGHT names linux-raw-sys, so it is stored literally rather than shared with rustix.
    let copyright = text
        .split_once(
            "\n===== SHA-256 3290ae0fbc9ddb77d2239121d710f0bb9d31b3b4744e6d97fe01e652b4c1870b =====\n",
        )
        .unwrap()
        .1;
    assert!(copyright.starts_with(
        "Short version for non-lawyers:\n\n`linux-raw-sys` is triple-licensed under Apache 2.0 with the LLVM Exception,\n"
    ));
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

#[test]
fn licenses_are_offline_embedded_and_reject_other_arguments() {
    let exe = env!("CARGO_BIN_EXE_teamarness-generated-package-backend");
    let output = Command::new(exe)
        .args(["--tmx-cli", "--licenses"])
        .env_clear()
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(output.stdout.starts_with(b"Third-party notices"));
    use sha2::{Digest, Sha256};
    let original = &output.stdout[..output.stdout.len() - 1]; // CLI's one framing newline is not notice data.
    let hash = Sha256::digest(original)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    assert_eq!(
        hash,
        "6bee8926c87f2657779ce17a234f958c7c57440e60bf97e572e659f38dfc4fe7"
    );
    // Independent original registry notice, not an expectation decoded by the CLI.
    let rfd = include_bytes!("fixtures/notices/rfd-0.16.1-LICENSE");
    assert!(output.stdout.windows(rfd.len()).any(|b| b == rfd));
    // Original CRLF and trailing-space notices survive source escaping byte-for-byte.
    assert!(
        output
            .stdout
            .windows(b"                              Apache License\r\n".len())
            .any(|b| b == b"                              Apache License\r\n")
    );
    assert!(
        output
            .stdout
            .windows(b"SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE. \n".len())
            .any(|b| b == b"SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE. \n")
    );
    let text = String::from_utf8(output.stdout).unwrap();
    for required in [
        "rfd 0.16.1",
        "objc2 0.6.5",
        "rustix 1.1.4",
        "Copyright (c) 2022 Bartłomiej Maryńczak",
        "LLVM Exceptions to the Apache 2.0 License",
        "frameworks-0.3.2",
        "7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54",
    ] {
        assert!(text.contains(required), "{required}");
    }
    for extra in ["--workspace", "workspaces", "--version"] {
        assert_eq!(
            Command::new(exe)
                .args(["--tmx-cli", "--licenses", extra])
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
    }
}

/// Independent POSIX single-quoting of the expected canonical command word.
fn expected_skill(command: &std::path::Path) -> String {
    let word = format!("'{}'", command.to_str().unwrap().replace('\'', "'\\''"));
    include_str!("../resources/teamarness/SKILL.md").replace("{{COMMAND}}", &word)
}
fn assert_portable_skill(stdout: &[u8], command: &std::path::Path) {
    let text = std::str::from_utf8(stdout).unwrap();
    assert_eq!(text, expected_skill(command));
    for forbidden in ["{{", "eg1.", ".local/share", "Bearer"] {
        assert!(!text.contains(forbidden), "{forbidden}");
    }
}
#[test]
fn skill_is_offline_rendered_for_the_canonical_invoked_executable() {
    let exe = env!("CARGO_BIN_EXE_teamarness-generated-package-backend");
    let dir = tempfile::tempdir().unwrap();
    // Temporary directories are reached through a symlinked /var on macOS; output is canonical.
    let folder = dir.path().join("it's a dir");
    std::fs::create_dir(&folder).unwrap();
    let copy = folder.join("tmx");
    std::fs::copy(exe, &copy).unwrap();
    let canonical = copy.canonicalize().unwrap();
    assert_ne!(canonical, copy);
    let links = dir.path().join("link dir");
    std::fs::create_dir(&links).unwrap();
    let link = links.join("tmx");
    std::os::unix::fs::symlink(&copy, &link).unwrap();
    for (path, args) in [
        (copy.clone(), vec!["--skill"]),
        (copy.clone(), vec!["--tmx-cli", "--skill"]),
        (link, vec!["--skill"]),
    ] {
        let output = Command::new(&path)
            .args(args)
            .current_dir(dir.path())
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        assert_portable_skill(&output.stdout, &canonical);
        assert!(
            String::from_utf8(output.stdout)
                .unwrap()
                .contains("it'\\''s a dir/tmx' workspaces\n")
        );
    }
    let output = Command::new(exe)
        .args(["--tmx-cli", "--skill"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    assert_portable_skill(
        &output.stdout,
        &std::path::Path::new(exe).canonicalize().unwrap(),
    );
}
#[test]
fn skill_refuses_extra_arguments_and_unquotable_executable_paths() {
    let exe = env!("CARGO_BIN_EXE_teamarness-generated-package-backend");
    for args in [
        vec!["--skill", "extra"],
        vec!["--skill", "--version"],
        vec!["--version", "--skill"],
        vec!["--skill", "--skill"],
        vec!["--skill", "--licenses"],
        vec!["--workspace", "a", "--skill"],
        vec!["workspaces", "--skill"],
    ] {
        let output = Command::new(exe)
            .arg("--tmx-cli")
            .args(&args)
            .env_clear()
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert_eq!(
            output.stdout,
            br#"{"error":{"code":"cli.invalid_arguments","details":{},"outcomeUncertain":false,"retryable":false,"source":"cli"}}
"#,
            "{args:?}"
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("control\u{1}dir");
    std::fs::create_dir(&folder).unwrap();
    let copy = folder.join("tmx");
    std::fs::copy(exe, &copy).unwrap();
    let output = Command::new(&copy)
        .arg("--skill")
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        output.stdout,
        br#"{"error":{"code":"cli.skill_unavailable","details":{},"outcomeUncertain":false,"retryable":false,"source":"cli"}}
"#
    );
    assert!(!output.stderr.is_empty());
}
#[test]
fn help_lists_offline_skill_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_teamarness-generated-package-backend"))
        .args(["--tmx-cli", "--help"])
        .output()
        .unwrap();
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(
        help.contains(
            "\n  --skill             Offline portable skill text for manual installation\n"
        )
    );
}

#[test]
fn embedded_literal_notice_checksums_are_intact() {
    use sha2::{Digest, Sha256};
    let texts = teamarness_generated_package_backend::cli::third_party_notices();
    let mut count = 0;
    for section in texts.split("\n===== SHA-256 ").skip(1) {
        let (expected, literal) = section.split_once(" =====\n").unwrap();
        let hash = |s: &str| {
            Sha256::digest(s.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        assert!(
            hash(literal) == expected
                || literal
                    .strip_suffix('\n')
                    .is_some_and(|s| hash(s) == expected),
            "{expected}"
        );
        count += 1;
    }
    assert_eq!(count, 83);
}
