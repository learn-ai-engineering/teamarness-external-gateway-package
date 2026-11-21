#[path = "support/cli_support.rs"]
mod support;
use teamarness_generated_package_backend::cli::args::parse;

fn words(extra: &[&str]) -> Vec<String> {
    [
        "--workspace",
        "Acme",
        "subscribe",
        "dev.new.provider",
        "changes",
        "--major",
        "7",
        "--json",
        "null",
    ]
    .into_iter()
    .chain(extra.iter().copied())
    .map(String::from)
    .collect()
}
#[test]
fn subscription_requires_one_explicit_lifetime_and_positive_bounded_numbers() {
    for flags in [
        vec!["--until-cancelled"],
        vec!["--for", "1ms"],
        vec!["--for", "2s"],
        vec!["--for", "3m"],
        vec!["--for", "4h", "--max-events", "4294967295"],
    ] {
        assert!(parse(&words(&flags), &mut &b""[..]).is_ok(), "{flags:?}");
    }
    for flags in [
        vec![],
        vec!["--for", "0s"],
        vec!["--for", "1"],
        vec!["--for", "-1s"],
        vec!["--for", "1.5s"],
        vec!["--for", "18446744073709551615h"],
        vec!["--for", "18446744073709551615ms"],
        vec!["--for", "1s", "--until-cancelled"],
        vec!["--until-cancelled", "--until-cancelled"],
        vec!["--until-cancelled", "--max-events", "0"],
        vec!["--until-cancelled", "--max-events", "4294967296"],
        vec!["--until-cancelled", "--max-events", "-1"],
    ] {
        assert!(parse(&words(&flags), &mut &b""[..]).is_err(), "{flags:?}");
    }
}

#[tokio::test]
async fn subscribe_child_preserves_raw_values_and_drains_count_reaching_value() {
    use support::*;
    let ready = "{\"type\":\"ready\",\"contract\":\"dev.new.provider\",\"major\":7,\"port\":\"changes\",\"kind\":\"query\",\"delivery\":\"stream\"}\n";
    let values = "{\"type\":\"heartbeat\"}\n{ \"type\":\"value\", \"value\":123456789012345678901234567890 }\n{\"type\":\"value\",\"value\":[null,true,1.2300e+04]}\n";
    let body = format!("{ready}{values}{{\"type\":\"end\",\"reason\":\"completed\"}}\n");
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let f = Fixture::new(vec![response(MANIFEST.as_bytes()), reply.into_bytes()]).await;
    let output = spawn_captured(
        tokio::process::Command::new(fixture_executable())
            .args(words(&["--until-cancelled", "--max-events", "2"]))
            .env(
                "TEAMARNESS_FIXTURE_HOME",
                f.home.path().canonicalize().unwrap(),
            ),
    )
    .wait_with_output()
    .await
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        output.stdout,
        format!("{ready}{values}{{\"type\":\"client_end\",\"reason\":\"event_limit\"}}\n")
            .as_bytes()
    );
    assert_eq!(f.count.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(f.requests.lock().unwrap()[1].starts_with(b"POST /v1/subscribe HTTP/1.1\r\n"));
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
                    "cli-fixture"
                ])
                .status()
                .unwrap()
                .success()
        );
        let meta = std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .output()
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&meta.stdout).unwrap();
        std::path::Path::new(value["target_directory"].as_str().unwrap())
            .join("debug/examples/cli-fixture")
    })
    .clone()
}

const READY: &str = "{\"type\":\"ready\",\"contract\":\"dev.new.provider\",\"major\":7,\"port\":\"changes\",\"kind\":\"event\",\"delivery\":\"stream\"}\n";
const END: &str = "{\"type\":\"end\",\"reason\":\"completed\"}\n";
fn ndjson(body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
}
// Concurrent macOS capture creation intermittently left EOF pending after an
// already-exited sibling. Serialize only spawn, never child/socket lifetimes.
fn spawn_child(command: &mut tokio::process::Command) -> tokio::process::Child {
    static SPAWN: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _spawn = SPAWN.lock().unwrap();
    command.spawn().unwrap()
}
fn spawn_captured(command: &mut tokio::process::Command) -> tokio::process::Child {
    spawn_child(
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped()),
    )
}
async fn collect_child(mut child: tokio::process::Child, seconds: u64) -> std::process::Output {
    use tokio::io::AsyncReadExt;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let (mut stdout, mut stderr) = (vec![], vec![]);
    let work = async {
        let read_stdout = async {
            if let Some(pipe) = &mut stdout_pipe {
                pipe.read_to_end(&mut stdout).await.unwrap();
            }
        };
        let read_stderr = async {
            if let Some(pipe) = &mut stderr_pipe {
                pipe.read_to_end(&mut stderr).await.unwrap();
            }
        };
        let (status, (), ()) = tokio::join!(child.wait(), read_stdout, read_stderr);
        status.unwrap()
    };
    let status = match tokio::time::timeout(std::time::Duration::from_secs(seconds), work).await {
        Ok(status) => status,
        Err(_) => {
            let observed = child.try_wait().unwrap();
            if child.id().is_some() {
                let _ = child.kill().await;
                let _ = child.wait().await;
            }
            panic!(
                "fixture child/output timeout: status={observed:?} stdout={:?} stderr={:?}",
                String::from_utf8_lossy(&stdout),
                String::from_utf8_lossy(&stderr)
            );
        }
    };
    std::process::Output {
        status,
        stdout,
        stderr,
    }
}
async fn fixture_output(f: &support::Fixture, flags: &[&str]) -> std::process::Output {
    let child = spawn_child(
        tokio::process::Command::new(fixture_executable())
            .args(words(flags))
            .env(
                "TEAMARNESS_FIXTURE_HOME",
                f.home.path().canonicalize().unwrap(),
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true),
    );
    collect_child(child, 10).await
}

#[tokio::test]
async fn opening_refusal_and_pre_ready_eof_malformed_or_mismatch_are_ordinary_json() {
    use support::*;
    let refusal = r#"{"error":{"source":"host","code":"port.unconfirmed","retryable":false,"outcomeUncertain":false,"details":{"why":"original"}}}"#;
    let refused = String::from_utf8(response(refusal.as_bytes()))
        .unwrap()
        .replacen("200 OK", "503 Refused", 1)
        .into_bytes();
    for (reply, original) in [
        (refused, Some(refusal)),
        (ndjson(""), None),
        (ndjson("not json\n"), None),
        (ndjson(&READY.replace("changes", "other")), None),
        (ndjson(&READY.replace("event", "command")), None),
        (ndjson("{\"type\":\"value\",\"value\":0}\n"), None),
    ] {
        let f = Fixture::new(vec![response(MANIFEST.as_bytes()), reply]).await;
        let output = fixture_output(&f, &["--until-cancelled"]).await;
        assert_eq!(output.status.code(), Some(1));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(value.get("type").is_none());
        if let Some(original) = original {
            assert_eq!(output.stdout, format!("{original}\n").as_bytes());
        } else {
            assert_eq!(value["error"]["source"], "cli");
        }
        assert_eq!(f.count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
#[tokio::test]
async fn post_ready_eof_malformed_duplicate_order_depth_and_frame_overflow_are_client_errors() {
    use support::*;
    let invalid = [
        String::new(),
        "bad\n".into(),
        READY.into(),
        "{\"type\":\"heartbeat\",\"extra\":1}\n".into(),
        "{\"type\":\"end\",\"reason\":\"missing\"}\n".into(),
        format!(
            "{{\"type\":\"value\",\"value\":{}0{}}}\n",
            "[".repeat(64),
            "]".repeat(64)
        ),
        format!(
            "{{\"type\":\"value\",\"value\":\"{}\"}}\n",
            "x".repeat(266240)
        ),
    ];
    for invalid in invalid {
        let f = Fixture::new(vec![
            response(MANIFEST.as_bytes()),
            ndjson(&format!("{READY}{invalid}")),
        ])
        .await;
        let output = fixture_output(&f, &["--until-cancelled"]).await;
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.starts_with(READY.as_bytes()));
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout[READY.len()..]).unwrap();
        assert_eq!(value["type"], "client_error");
        assert_eq!(value["error"]["code"], "cli.stream_interrupted");
        assert_eq!(value["error"]["outcomeUncertain"], false);
    }
}
#[tokio::test]
async fn completed_and_error_gateway_ends_are_original_and_determine_exit() {
    use support::*;
    let error = "{ \"type\":\"end\", \"reason\":\"error\", \"error\":{\"source\":\"provider\",\"code\":\"provider.refused\",\"retryable\":false,\"outcomeUncertain\":false,\"details\":{\"version\":123456789012345678901234567890,\"decimal\":1.2300e+04}}}\n";
    for (ending, exit) in [(END, 0), (error, 1)] {
        let f = Fixture::new(vec![
            response(MANIFEST.as_bytes()),
            ndjson(&format!("{READY}{ending}")),
        ])
        .await;
        let output = fixture_output(&f, &["--until-cancelled"]).await;
        assert_eq!(output.status.code(), Some(exit));
        assert_eq!(output.stdout, format!("{READY}{ending}").as_bytes());
        // The ordinary path still works; streaming does not replace it.
        let f = Fixture::new(vec![response(MANIFEST.as_bytes())]).await;
        assert_eq!(f.run(&["--workspace", "Acme", "status"]).await.exit, 0);
    }
}

#[derive(Clone, Copy)]
enum Traffic {
    Idle,
    Flood,
    Opening,
    Heartbeats,
}
struct LiveFixture {
    home: tempfile::TempDir,
    closed: tokio::sync::oneshot::Receiver<()>,
    opened: tokio::sync::oneshot::Receiver<()>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for LiveFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl LiveFixture {
    // Actual loopback NDJSON, with chunks split independently from line framing.
    async fn new(traffic: Traffic) -> Self {
        use support::*;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let home = private_home();
        let mut c = connection("workspace-a", "Acme");
        c.port = listener.local_addr().unwrap().port();
        teamarness_generated_package_backend::client_store::ClientStore::for_home(
            &home.path().canonicalize().unwrap(),
        )
        .unwrap()
        .publish(&c)
        .unwrap();
        let (close, closed) = tokio::sync::oneshot::channel();
        let (open, opened) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            for n in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![];
                let mut byte = [0];
                loop {
                    socket.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
                if n == 0 {
                    socket
                        .write_all(&response(MANIFEST.as_bytes()))
                        .await
                        .unwrap();
                    socket.shutdown().await.unwrap();
                    continue;
                }
                assert!(request.starts_with(b"POST /v1/subscribe HTTP/1.1\r\n"));
                let request_text = String::from_utf8(request).unwrap();
                let length: usize = request_text
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                let mut body = vec![0; length];
                socket.read_exact(&mut body).await.unwrap();
                assert_eq!(body, b"{\"contract\":\"dev.new.provider\",\"major\":7,\"port\":\"changes\",\"request\":null}");
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/x-ndjson\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").await.unwrap();
                let _ = open.send(());
                if !matches!(traffic, Traffic::Opening) {
                    socket
                        .write_all(format!("{:x}\r\n{READY}\r\n", READY.len()).as_bytes())
                        .await
                        .unwrap();
                }
                if matches!(traffic, Traffic::Flood) {
                    let value = format!(
                        "{{\"type\":\"value\",\"value\":\"{}\"}}\n",
                        "x".repeat(262000)
                    );
                    let chunk = format!("{:x}\r\n{value}\r\n", value.len());
                    loop {
                        if socket.write_all(chunk.as_bytes()).await.is_err() {
                            break;
                        }
                    }
                } else if matches!(traffic, Traffic::Heartbeats) {
                    let mut heartbeat = tokio::time::interval_at(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(15),
                        std::time::Duration::from_secs(15),
                    );
                    loop {
                        let mut byte = [0];
                        tokio::select! {
                            _ = socket.read(&mut byte) => break,
                            _ = heartbeat.tick() => {
                                let frame = "{\"type\":\"heartbeat\"}\n";
                                if socket.write_all(format!("{:x}\r\n{frame}\r\n", frame.len()).as_bytes()).await.is_err() { break; }
                            }
                        }
                    }
                } else {
                    let mut rest = vec![];
                    let _ = socket.read_to_end(&mut rest).await;
                }
                let _ = close.send(());
                // No hidden automatic reconnect after any termination.
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(150), listener.accept())
                        .await
                        .is_err()
                );
                break;
            }
        });
        Self {
            home,
            closed,
            opened,
            server,
        }
    }
    fn child(&self, flags: &[&str]) -> tokio::process::Child {
        spawn_child(
            tokio::process::Command::new(fixture_executable())
                .args(words(flags))
                .env(
                    "TEAMARNESS_FIXTURE_HOME",
                    self.home.path().canonicalize().unwrap(),
                )
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true),
        )
    }
    async fn assert_closed(&mut self) {
        tokio::time::timeout(std::time::Duration::from_secs(3), &mut self.closed)
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut self.server)
            .await
            .unwrap()
            .unwrap();
    }
}
#[tokio::test]
async fn sigint_after_ready_delivers_one_local_terminal_closes_socket_and_never_reconnects() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    let mut f = LiveFixture::new(Traffic::Idle).await;
    let mut child = f.child(&["--until-cancelled"]);
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        output.read_line(&mut ready),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(ready, READY);
    let pid = rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::INT).unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(7), child.wait())
        .await
        .unwrap()
        .unwrap();
    let mut rest = String::new();
    output.read_to_string(&mut rest).await.unwrap();
    assert_eq!(status.code(), Some(130));
    assert_eq!(rest, "{\"type\":\"client_end\",\"reason\":\"cancelled\"}\n");
    f.assert_closed().await;
}
#[tokio::test]
async fn explicit_duration_closes_idle_socket_without_gateway_completion() {
    let mut f = LiveFixture::new(Traffic::Idle).await;
    let child = f.child(&["--for", "50ms"]);
    let output = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(
        output.stdout,
        format!("{READY}{{\"type\":\"client_end\",\"reason\":\"duration_limit\"}}\n").as_bytes()
    );
    f.assert_closed().await;
}
#[tokio::test]
async fn actual_unread_pipe_times_out_and_closes_network_without_terminal_write() {
    let mut f = LiveFixture::new(Traffic::Flood).await;
    let mut child = f.child(&["--until-cancelled", "--max-events", "1"]);
    let started = std::time::Instant::now();
    // A separate ordinary CLI process remains usable while this observation is blocked.
    let ordinary = support::Fixture::new(vec![
        support::response(support::MANIFEST.as_bytes()),
        support::response(b"{\"result\":true}"),
    ])
    .await;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        spawn_captured(
            tokio::process::Command::new(fixture_executable())
                .args([
                    "--workspace",
                    "Acme",
                    "call",
                    "dev.new.provider",
                    "get",
                    "--major",
                    "7",
                    "--json",
                    "{}",
                ])
                .env(
                    "TEAMARNESS_FIXTURE_HOME",
                    ordinary.home.path().canonicalize().unwrap(),
                ),
        )
        .wait_with_output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(result.status.success());
    assert_eq!(result.stdout, b"{\"result\":true}\n");
    // Keep stdout open but deliberately unread, exercising the OS pipe, not an adapter.
    let status = tokio::time::timeout(std::time::Duration::from_secs(9), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(1));
    assert!(started.elapsed() >= std::time::Duration::from_secs(5));
    use tokio::io::AsyncReadExt;
    let mut bytes = vec![];
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut bytes)
        .await
        .unwrap();
    assert!(bytes.len() < 266240); // No full count-reaching value/terminal was delivered.
    assert!(!String::from_utf8_lossy(&bytes).contains("client_end"));
    f.assert_closed().await;
}
#[tokio::test]
async fn broken_stdout_exits_nonzero_closes_network_and_never_writes_again() {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut f = LiveFixture::new(Traffic::Flood).await;
    let mut child = f.child(&["--until-cancelled"]);
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    output.read_line(&mut ready).await.unwrap();
    assert_eq!(ready, READY);
    drop(output);
    let status = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(1));
    f.assert_closed().await;
}
#[tokio::test]
async fn sigint_before_ready_is_ordinary_json_and_closes_opening_socket() {
    let mut f = LiveFixture::new(Traffic::Opening).await;
    let child = f.child(&["--until-cancelled"]);
    // Exact transport seam, not a startup sleep: signal registration precedes sockets.
    tokio::time::timeout(std::time::Duration::from_secs(5), &mut f.opened)
        .await
        .unwrap()
        .unwrap();
    let pid = rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::INT).unwrap();
    let output = tokio::time::timeout(std::time::Duration::from_secs(7), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.status.code(), Some(130));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value.get("type").is_none());
    assert_eq!(value["error"]["code"], "cli.cancelled");
    f.assert_closed().await;
}
#[tokio::test]
async fn real_child_continuous_observation_survives_thirty_seconds_with_sparse_heartbeats() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    let mut f = LiveFixture::new(Traffic::Heartbeats).await;
    let mut child = f.child(&["--until-cancelled", "--max-events", "1"]);
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    output.read_line(&mut ready).await.unwrap();
    assert_eq!(ready, READY);
    let start = std::time::Instant::now();
    for _ in 0..2 {
        let mut heartbeat = String::new();
        tokio::time::timeout(
            std::time::Duration::from_secs(18),
            output.read_line(&mut heartbeat),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(heartbeat, "{\"type\":\"heartbeat\"}\n");
    }
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert!(start.elapsed() > std::time::Duration::from_secs(30));
    assert!(child.try_wait().unwrap().is_none());
    let pid = rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::INT).unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(7), child.wait())
        .await
        .unwrap()
        .unwrap();
    let mut rest = String::new();
    output.read_to_string(&mut rest).await.unwrap();
    assert_eq!(status.code(), Some(130));
    assert_eq!(rest, "{\"type\":\"client_end\",\"reason\":\"cancelled\"}\n");
    f.assert_closed().await;
}
#[tokio::test]
async fn invalid_subscription_lifetime_flags_exit_two_before_any_socket() {
    use support::*;
    let f = Fixture::new(vec![response(MANIFEST.as_bytes())]).await;
    for flags in [
        vec![],
        vec!["--for", "0ms"],
        vec!["--for", "18446744073709551615h"],
        vec!["--for", "1s", "--until-cancelled"],
        vec!["--until-cancelled", "--max-events", "4294967296"],
    ] {
        let args = words(&flags);
        let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(f.run(&refs).await.exit, 2);
    }
    assert_eq!(f.count.load(std::sync::atomic::Ordering::SeqCst), 0);
}
#[tokio::test]
async fn escaped_control_names_and_value_whitespace_remain_original_bytes() {
    use support::*;
    let ready = r#"{ "t\u0079pe":"ready", "contract":"dev.new.provider", "major":7, "port":"chan\u0067es", "kind":"event", "delivery":"stream" }"#;
    let value = r#"{ "type":"value", "value": [123456789012345678901234567890, 1.2300e+04] }"#;
    let body = format!("{ready}\n{value}\n{END}");
    let f = Fixture::new(vec![response(MANIFEST.as_bytes()), ndjson(&body)]).await;
    let output = fixture_output(&f, &["--until-cancelled"]).await;
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, body.as_bytes());
}
#[tokio::test]
async fn regular_stdout_refuses_before_network_and_preserves_existing_file_bytes() {
    use support::*;
    let f = Fixture::new(vec![
        response(MANIFEST.as_bytes()),
        ndjson(&format!("{READY}{END}")),
    ])
    .await;
    let path = f.home.path().join("events.ndjson");
    std::fs::write(&path, b"existing sentinel\n").unwrap();
    let file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    let child = spawn_child(
        tokio::process::Command::new(fixture_executable())
            .args(words(&["--until-cancelled"]))
            .env(
                "TEAMARNESS_FIXTURE_HOME",
                f.home.path().canonicalize().unwrap(),
            )
            .stdout(file)
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true),
    );
    let output = collect_child(child, 5).await;
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(std::fs::read(&path).unwrap(), b"existing sentinel\n");
    assert_eq!(f.count.load(std::sync::atomic::Ordering::SeqCst), 0);
    let stderr = String::from_utf8(output.stderr).unwrap();
    let error: serde_json::Value = serde_json::from_str(stderr.lines().next().unwrap()).unwrap();
    assert_eq!(error["error"]["code"], "cli.stream_stdout_unsupported");
    assert!(stderr.contains("tee"));
}
/// macOS BSD `script` takes the command as trailing arguments.
#[cfg(target_os = "macos")]
fn pty_command(program: &std::path::Path, args: &[String]) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("/usr/bin/script");
    command.args(["-q", "/dev/null"]).arg(program).args(args);
    command
}
/// util-linux `script` takes one shell command string; every word is single-quoted and
/// the interpreting shell is fixed, so neither SHELL nor word content changes the argv.
#[cfg(not(target_os = "macos"))]
fn pty_command(program: &std::path::Path, args: &[String]) -> tokio::process::Command {
    let quote = |word: &str| format!("'{}'", word.replace('\'', "'\\''"));
    let line = std::iter::once(quote(program.to_str().expect("UTF-8 fixture path")))
        .chain(args.iter().map(|a| quote(a)))
        .collect::<Vec<_>>()
        .join(" ");
    let mut command = tokio::process::Command::new("/usr/bin/script");
    command
        .args(["-q", "-e", "-c", &line, "/dev/null"])
        .env("SHELL", "/bin/sh");
    command
}
#[tokio::test]
async fn real_terminal_stdout_supports_readiness_registered_streaming() {
    use support::*;
    let f = Fixture::new(vec![
        response(MANIFEST.as_bytes()),
        ndjson(&format!("{READY}{END}")),
    ])
    .await;
    // script supplies a genuine PTY; it is a test OS adapter, not a shipped helper.
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(7),
        spawn_captured(
            pty_command(&fixture_executable(), &words(&["--until-cancelled"]))
                .env(
                    "TEAMARNESS_FIXTURE_HOME",
                    f.home.path().canonicalize().unwrap(),
                )
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true),
        )
        .wait_with_output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let text = String::from_utf8(output.stdout)
        .unwrap()
        .replace("\r\n", "\n");
    assert!(text.contains(READY));
    assert!(text.contains(END));
    assert_eq!(f.count.load(std::sync::atomic::Ordering::SeqCst), 2);
}
#[tokio::test]
async fn sigint_during_real_pipe_backpressure_drains_one_value_then_one_cancel_terminal() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
    let mut f = LiveFixture::new(Traffic::Flood).await;
    let mut child = f.child(&["--until-cancelled"]);
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    output.read_line(&mut ready).await.unwrap();
    assert_eq!(ready, READY);
    // A 262-KiB value cannot fit in the unread OS pipe plus BufReader's read buffer.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let pid = rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap();
    rustix::process::kill_process(pid, rustix::process::Signal::INT).unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let mut rest = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(4),
        output.read_to_string(&mut rest),
    )
    .await
    .unwrap()
    .unwrap();
    let status = tokio::time::timeout(std::time::Duration::from_secs(2), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(130));
    let lines = rest
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["type"], "value");
    assert_eq!(lines[0]["value"].as_str().unwrap().len(), 262000);
    assert_eq!(
        lines[1],
        serde_json::json!({"type":"client_end","reason":"cancelled"})
    );
    f.assert_closed().await;
}
