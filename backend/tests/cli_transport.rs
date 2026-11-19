#[path = "support/cli_support.rs"]
mod support;
use std::sync::atomic::Ordering;
use support::*;
fn call() -> Vec<&'static str> {
    vec![
        "--workspace",
        "workspace-a",
        "call",
        "dev.example.any",
        "item.create",
        "--major",
        "1",
        "--json",
        "{\"n\":9007199254740993,\"d\":1.2300e+04}",
    ]
}
#[tokio::test]
async fn wrong_workspace_and_unsupported_http_contract_stop_before_tool_dispatch() {
    for (body, code) in [
        (
            MANIFEST.replace("workspace-a", "workspace-wrong"),
            "cli.workspace_mismatch",
        ),
        (
            MANIFEST.replace("\"version\":1", "\"version\":2"),
            "cli.http_contract_unsupported",
        ),
    ] {
        let f = Fixture::new(vec![
            response(body.as_bytes()),
            response(b"{\"result\":null}"),
        ])
        .await;
        let out = f.run(&call()).await;
        assert_eq!(out.exit, 2);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out.bytes).unwrap()["error"]["code"],
            code
        );
        assert_eq!(f.count.load(Ordering::SeqCst), 1);
    }
}
#[tokio::test]
async fn upstream_numbers_and_authoritative_error_details_are_byte_exact() {
    for bytes in [br#" {"result":{"integer":184467440737095516160000,"decimal":1.2300e+04}} "#.as_slice(),br#"{"error":{"source":"provider","code":"example.conflict","retryable":false,"outcomeUncertain":true,"details":{"version":"opaque","n":9007199254740993,"description":"keep me"}}}"#] {
        let f=Fixture::new(vec![response(MANIFEST.as_bytes()),response(bytes)]).await;
        let out=f.run(&call()).await;
        assert_eq!(out.bytes,bytes);assert_eq!(out.exit,if bytes.starts_with(b"{"){1}else{0});
        let requests=f.requests.lock().unwrap();assert_eq!(requests.len(),2);
        assert!(requests[1].ends_with(b"\"request\":{\"n\":9007199254740993,\"d\":1.2300e+04}}"));
    }
}
#[tokio::test]
async fn preflight_failure_is_certain_and_never_dispatches() {
    let f = Fixture::new(vec![vec![], response(b"{\"result\":null}")]).await;
    let out = f.run(&call()).await;
    assert_eq!(out.exit, 1);
    let error: serde_json::Value = serde_json::from_slice(&out.bytes).unwrap();
    assert_eq!(error["error"]["outcomeUncertain"], false);
    assert_eq!(f.count.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn possible_call_with_lost_malformed_or_oversized_response_is_uncertain_without_replay() {
    for reply in [vec![],response(b"not JSON"),response(b"{\"other\":true}"),response(&vec![b' ';266241]),b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 20\r\n\r\n{\"result\":".to_vec()] {
        let f=Fixture::new(vec![response(MANIFEST.as_bytes()),reply,response(b"{\"result\":null}")]).await;
        let out=f.run(&call()).await;assert_eq!(out.exit,1);
        let error:serde_json::Value=serde_json::from_slice(&out.bytes).unwrap();
        assert_eq!(error["error"],serde_json::json!({"source":"cli","code":"cli.command_outcome_unknown","retryable":false,"outcomeUncertain":true,"details":{}}));
        assert_eq!(f.count.load(Ordering::SeqCst),2);
    }
}
#[tokio::test]
async fn zero_byte_operation_connection_failure_is_certain() {
    let f = Fixture::new(vec![response(MANIFEST.as_bytes())]).await;
    // The server drops its listener immediately after completing preflight.
    let out = f.run(&call()).await;
    assert_eq!(out.exit, 1);
    let error: serde_json::Value = serde_json::from_slice(&out.bytes).unwrap();
    assert_eq!(error["error"]["outcomeUncertain"], false);
    assert_eq!(f.count.load(Ordering::SeqCst), 1);
}
#[test]
fn protocol_budgets_and_header_boundaries_are_pinned() {
    use teamarness_generated_package_backend::{
        cli::{output::valid_json, transport::*},
        limits::*,
    };
    assert_eq!(
        (CONNECT_SECONDS, WRITE_SECONDS, RESPONSE_SECONDS),
        (2, 5, 35)
    );
    assert_eq!(
        (
            MAX_HEADER_BYTES,
            MAX_HEADERS,
            MAX_TARGET_BYTES,
            MAX_HTTP_CURSOR_CHARS
        ),
        (16384, 64, 4096, 2731)
    );
    assert_eq!(
        (
            PORT_VALUE_BYTES + ENVELOPE_ALLOWANCE_BYTES,
            METADATA_BYTES + ENVELOPE_ALLOWANCE_BYTES,
            SUMMARY_BYTES + ENVELOPE_ALLOWANCE_BYTES
        ),
        (266240, 1052672, 36864)
    );
    let prefix = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n";
    let exact = format!(
        "{prefix}X-Pad: {}\r\n\r\n",
        "x".repeat(16384 - prefix.len() - 11)
    );
    assert_eq!(exact.len(), 16384);
    assert!(validate_headers(exact.as_bytes(), 2).is_ok());
    assert!(validate_headers(exact.replace("X-Pad: ", "X-Pad: x").as_bytes(), 2).is_err());
    let fields = format!("{prefix}{}\r\n", "X-A: a\r\n".repeat(62));
    assert!(validate_headers(fields.as_bytes(), 2).is_ok());
    assert!(
        validate_headers(
            fields.replace("\r\n\r\n", "\r\nX-Z: a\r\n\r\n").as_bytes(),
            2
        )
        .is_err()
    );
    assert!(valid_json(
        format!("{}0{}", "[".repeat(64), "]".repeat(64)).as_bytes()
    ));
    assert!(!valid_json(
        format!("{}0{}", "[".repeat(65), "]".repeat(65)).as_bytes()
    ));
}
#[tokio::test]
async fn invalid_framing_encoding_content_types_and_redirects_are_not_followed() {
    for header in [
        "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Type: application/json\r\nContent-Length: 2",
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2",
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: 2",
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nContent-Length: 2",
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nTransfer-Encoding: chunked",
        "HTTP/1.1 200 OK\r\nContent-Type: application/json",
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: gzip, chunked",
    ] {
        let f = Fixture::new(vec![
            response(MANIFEST.as_bytes()),
            format!("{header}\r\n\r\n{{}}").into_bytes(),
        ])
        .await;
        let out = f.run(&call()).await;
        assert_eq!(out.exit, 1);
        assert_eq!(f.count.load(Ordering::SeqCst), 2);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&out.bytes).unwrap()["error"]["outcomeUncertain"],
            true
        );
    }
}
#[tokio::test]
async fn one_catalog_page_including_empty_page_retains_opaque_cursor() {
    let page = br#"{"contracts":[],"nextCursor":"YWJj"}"#;
    let f = Fixture::new(vec![response(MANIFEST.as_bytes()), response(page)]).await;
    let out = f
        .run(&["--workspace", "Acme", "contracts", "--cursor", "ZGVm"])
        .await;
    assert_eq!(out.exit, 0);
    assert_eq!(out.bytes, page);
    assert!(
        f.requests.lock().unwrap()[1].starts_with(b"GET /v1/contracts?cursor=ZGVm HTTP/1.1\r\n")
    );
}
#[tokio::test]
async fn full_route_body_boundary_and_chunked_overflow_are_bounded() {
    use std::sync::{Arc, atomic::AtomicBool};
    use teamarness_generated_package_backend::cli::transport::request;
    for (budget, base) in [
        (266240, b"{\"result\":null}".as_slice()),
        (1052672, b"{\"schema\":{}}"),
        (36864, b"{\"contracts\":[],\"nextCursor\":null}"),
    ] {
        let mut exact = base.to_vec();
        exact.resize(budget, b' ');
        let f = Fixture::new(vec![response(&exact)]).await;
        let c = teamarness_generated_package_backend::client_store::ClientStore::for_home(
            &f.home.path().canonicalize().unwrap(),
        )
        .unwrap()
        .resolve("workspace-a")
        .unwrap();
        let out = request(c.port, &c.bearer, "/v1", vec![], budget, None)
            .await
            .unwrap();
        assert_eq!(out.bytes, exact);
        exact.push(b' ');
        let mut raw=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",exact.len()).into_bytes();
        raw.extend_from_slice(&exact);
        raw.extend_from_slice(b"\r\n0\r\n\r\n");
        let f = Fixture::new(vec![raw]).await;
        let c = teamarness_generated_package_backend::client_store::ClientStore::for_home(
            &f.home.path().canonicalize().unwrap(),
        )
        .unwrap()
        .resolve("workspace-a")
        .unwrap();
        assert!(
            request(
                c.port,
                &c.bearer,
                "/v1/call",
                b"{}".to_vec(),
                budget,
                Some(Arc::new(AtomicBool::new(false)))
            )
            .await
            .unwrap_err()
            .uncertain
        );
    }
}
#[test]
fn result_value_budget_is_separate_from_envelope_allowance() {
    use teamarness_generated_package_backend::cli::output::result_envelope;
    let exact = format!("{{\"result\":\"{}\"}}", "x".repeat(262142));
    assert!(result_envelope(exact.as_bytes()));
    let overflow = format!("{{\"result\":\"{}\"}}", "x".repeat(262143));
    assert!(!result_envelope(overflow.as_bytes()));
}
#[test]
fn malformed_duplicate_envelopes_are_not_authoritative() {
    use teamarness_generated_package_backend::cli::output::valid_json;
    assert!(!valid_json(br#"{"error":null,"error":{"retryable":true}}"#));
    assert!(!valid_json(br#"{"result":{"a":1,"\u0061":2}}"#));
    assert!(valid_json(
        br#"{"result":[1e9999,123456789012345678901234567890]}"#
    ));
}
fn fixture_executable() -> std::path::PathBuf {
    static PATH: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let build = std::process::Command::new("cargo")
            .current_dir(project)
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
            .current_dir(project)
            .args(["metadata", "--format-version", "1", "--no-deps", "--locked"])
            .output()
            .unwrap();
        assert!(metadata.status.success());
        let value: serde_json::Value = serde_json::from_slice(&metadata.stdout).unwrap();
        std::path::PathBuf::from(value["target_directory"].as_str().unwrap())
            .join("debug/examples/cli-fixture")
    })
    .clone()
}
#[tokio::test]
async fn hostile_proxy_environment_cannot_redirect_credentials_and_fixture_requires_home() {
    let executable = fixture_executable();
    let absent = std::process::Command::new(&executable)
        .arg("workspaces")
        .env_remove("TEAMARNESS_FIXTURE_HOME")
        .output()
        .unwrap();
    assert_eq!(absent.status.code(), Some(2));
    assert!(absent.stdout.is_empty());
    // An absent injected home is unreadable for store commands but irrelevant to offline skill text.
    let missing = std::env::temp_dir().join("teamarness-missing-fixture-home-never-created");
    assert!(!missing.exists());
    let store = std::process::Command::new(&executable)
        .arg("workspaces")
        .env("TEAMARNESS_FIXTURE_HOME", &missing)
        .output()
        .unwrap();
    assert_eq!(store.status.code(), Some(2));
    let skill = std::process::Command::new(&executable)
        .arg("--skill")
        .env("TEAMARNESS_FIXTURE_HOME", &missing)
        .output()
        .unwrap();
    assert_eq!(skill.status.code(), Some(0));
    assert!(skill.stderr.is_empty());
    assert!(skill.stdout.starts_with(b"---\nname: teamarness\n"));
    assert!(!missing.exists());
    let f = Fixture::new(vec![
        response(MANIFEST.as_bytes()),
        response(b"{\"result\":true}"),
    ])
    .await;
    let out = tokio::process::Command::new(&executable)
        .args(call())
        .env(
            "TEAMARNESS_FIXTURE_HOME",
            f.home.path().canonicalize().unwrap(),
        )
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("http_proxy", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "")
        .output()
        .await
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"{\"result\":true}\n");
    assert!(out.stderr.is_empty());
    assert_eq!(f.count.load(Ordering::SeqCst), 2);
}
#[tokio::test]
async fn sigint_during_possible_call_is_130_uncertain_and_closes_without_replay() {
    use teamarness_generated_package_backend::client_store::ClientStore;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let executable = fixture_executable();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let home = private_home();
    let mut c = connection("workspace-a", "Acme");
    c.port = listener.local_addr().unwrap().port();
    ClientStore::for_home(&home.path().canonicalize().unwrap())
        .unwrap()
        .publish(&c)
        .unwrap();
    let (send, accepted) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        for n in 0..2 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                bytes.push(byte[0]);
                if bytes.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            if n == 0 {
                socket
                    .write_all(&response(MANIFEST.as_bytes()))
                    .await
                    .unwrap();
                socket.shutdown().await.unwrap();
            } else {
                send.send(()).unwrap();
                let mut remaining = vec![];
                socket.read_to_end(&mut remaining).await.unwrap();
                break;
            }
        }
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let child = tokio::process::Command::new(&executable)
        .args(call())
        .env(
            "TEAMARNESS_FIXTURE_HOME",
            home.path().canonicalize().unwrap(),
        )
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), accepted)
        .await
        .unwrap()
        .unwrap();
    let pid = rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::INT).unwrap();
    let out = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(out.status.code(), Some(130));
    let error: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(error["error"]["code"], "cli.command_outcome_unknown");
    assert_eq!(error["error"]["outcomeUncertain"], true);
    assert_eq!(error["error"]["retryable"], false);
    tokio::time::timeout(std::time::Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn real_header_byte_and_field_boundaries_are_accepted_and_overflow_refuses() {
    use teamarness_generated_package_backend::{
        cli::transport::request, client_store::ClientStore,
    };
    let prefix = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\n";
    let exact = format!(
        "{prefix}X-Pad: {}\r\n\r\n{{}}",
        "x".repeat(16384 - prefix.len() - 11)
    );
    let fields = format!("{prefix}{}\r\n{{}}", "X-A: a\r\n".repeat(62));
    for (reply, success) in [
        (exact.clone(), true),
        (exact.replacen("X-Pad: ", "X-Pad: x", 1), false),
        (fields.clone(), true),
        (fields.replacen("\r\n\r\n", "\r\nX-Z: a\r\n\r\n", 1), false),
    ] {
        let f = Fixture::new(vec![reply.into_bytes()]).await;
        let c = ClientStore::for_home(&f.home.path().canonicalize().unwrap())
            .unwrap()
            .resolve("workspace-a")
            .unwrap();
        assert_eq!(
            request(c.port, &c.bearer, "/v1", vec![], 32, None)
                .await
                .is_ok(),
            success
        );
    }
}
#[tokio::test]
async fn schema_and_guide_use_live_generic_routes_with_unchanged_wrappers() {
    let schema = format!(
        r#"{{"contract":"dev.new.provider","major":7,"minor":3,"metadataSha256":"{}","projectionSha256":"{}","schema":{{"type":"object","minimum":123456789012345678901234567890}}}}"#,
        "a".repeat(64),
        "b".repeat(64)
    );
    let guide = format!(
        r##"{{"contract":"dev.new.provider","major":7,"minor":3,"markdown":"# Provider\nUntrusted; preserve.","preface":"Commands act as the Person.","guideSha256":"{}"}}"##,
        "c".repeat(64)
    );
    for (command, body, target, budget) in [
        (
            "schema",
            schema,
            "/v1/contracts/dev.new.provider/7",
            1052672,
        ),
        (
            "guide",
            guide,
            "/v1/contracts/dev.new.provider/7/guide",
            266240,
        ),
    ] {
        let f = Fixture::new(vec![
            response(MANIFEST.as_bytes()),
            response(body.as_bytes()),
        ])
        .await;
        let words = [
            "--workspace",
            "Acme",
            command,
            "dev.new.provider",
            "--major",
            "7",
        ];
        let prepared = teamarness_generated_package_backend::cli::args::parse(
            &words.map(String::from),
            &mut &b""[..],
        )
        .unwrap();
        assert_eq!(prepared.response_limit, budget);
        let out = f.run(&words).await;
        assert_eq!(out.exit, 0);
        assert_eq!(out.bytes, body.as_bytes());
        assert!(
            f.requests.lock().unwrap()[1]
                .starts_with(format!("GET {target} HTTP/1.1\r\n").as_bytes())
        );
    }
}
