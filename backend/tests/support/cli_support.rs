use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use teamarness_generated_package_backend::{
    cli::{Outcome, args, execute},
    client_store::{ClientStore, Connection},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
pub const MANIFEST: &str = r#"{"version":1,"readOnly":false,"workspace":{"id":"workspace-a","name":"Acme"},"routes":{"contracts":"/v1/contracts","onboarding":"/v1/onboarding","call":"/v1/call","subscribe":"/v1/subscribe"},"supportedShapes":[{"kind":"query","delivery":"response"},{"kind":"command","delivery":"response"},{"kind":"query","delivery":"stream"},{"kind":"event","delivery":"stream"}],"limits":{"outstandingSdkInvocations":12,"portValueBytes":262144,"metadataBytes":1048576,"toolWaitSeconds":25,"streams":8,"streamHeartbeatSeconds":15,"streamWriteSeconds":5,"streamFrameAllowanceBytes":4096}}"#;
pub fn connection(id: &str, name: &str) -> Connection {
    Connection {
        format_version: 1,
        workspace_id: id.into(),
        workspace_name: name.into(),
        port: 43127,
        installation_id: "installation-a".into(),
        continuity_id: "continuity-a".into(),
        origin_nonce: "a".repeat(64),
        token_id: "AAAAAAAAAAAAAAAAAAAAAA".into(),
        generation: 1,
        bearer: format!("eg1.{}.{}", "AAAAAAAAAAAAAAAAAAAAAA", "A".repeat(43)),
        operation_id: "operation-a".into(),
    }
}
/// An owner-only injected home, independent of the process umask (Ubuntu's default
/// 0002 would otherwise create a group-writable home the client store refuses).
pub fn private_home() -> tempfile::TempDir {
    use std::os::unix::fs::PermissionsExt;
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}
pub fn response(body: &[u8]) -> Vec<u8> {
    let mut raw=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).into_bytes();
    raw.extend_from_slice(body);
    raw
}
pub struct Fixture {
    pub home: tempfile::TempDir,
    pub count: Arc<AtomicUsize>,
    pub requests: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    pub async fn new(replies: Vec<Vec<u8>>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let home = private_home();
        let store = ClientStore::for_home(&home.path().canonicalize().unwrap()).unwrap();
        let mut c = connection("workspace-a", "Acme");
        c.port = listener.local_addr().unwrap().port();
        store.publish(&c).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(std::sync::Mutex::new(vec![]));
        let (n, r) = (count.clone(), requests.clone());
        let server = tokio::spawn(async move {
            for reply in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                n.fetch_add(1, Ordering::SeqCst);
                let mut bytes = vec![];
                let mut chunk = [0; 8192];
                loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                        let h = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let length = h
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .and_then(|n| n.parse::<usize>().ok())
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                r.lock().unwrap().push(bytes);
                let _ = socket.write_all(&reply).await;
                let _ = socket.shutdown().await;
            }
        });
        Self {
            home,
            count,
            requests,
            server,
        }
    }
    pub async fn run(&self, words: &[&str]) -> Outcome {
        let words = words.iter().map(|s| (*s).into()).collect::<Vec<String>>();
        let args = match args::parse(&words, &mut &b""[..]) {
            Ok(a) => a,
            Err(e) => return e.into(),
        };
        execute(
            args,
            &self.home.path().canonicalize().unwrap(),
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap_or_else(Into::into)
    }
}
