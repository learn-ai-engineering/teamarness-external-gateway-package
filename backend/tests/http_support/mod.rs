#![allow(dead_code)]
//! Verifies: TEAM.SYS.PKG-R03, R05, R13, R14, R19. Only the Host capability is scripted.
#[path = "../support/environment.rs"]
pub mod backend_environment;
#[path = "../membership_support/mod.rs"]
pub mod membership_support;
use base64::{Engine, engine::general_purpose::STANDARD};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use teamarness_generated_package_backend::State;
use teamarness_package_sdk::BackendError;
use teamarness_package_sdk::testing::{Harness, HarnessBuilder, ScriptedCapability};

#[derive(Clone)]
pub struct Provider {
    pub document: Value,
    pub provider: String,
    pub major: u16,
    pub enabled: bool,
    pub invocations: usize,
    pub cleanup_fails: bool,
    pub result: Value,
    pub guide: String,
    pub failure: Option<BackendError>,
    pub describes: usize,
}
impl Provider {
    pub fn new() -> Self {
        let request = json!({"type":"object","properties":{},"additionalProperties":false});
        let ports = json!([
            {"name":"runtime.guide","kind":"query","description":"Guide","request":request,"response":{"type":"object","properties":{"markdown":{"type":"string","maxLength":1000}},"required":["markdown"],"additionalProperties":false}},
            {"name":"read.item","kind":"query","description":"Read","request":request,"response":{"$ref":"#/$defs/Result"}},
            {"name":"write.item","kind":"command","description":"Write","request":request,"response":request},
            {"name":"watch.item","kind":"query","description":"Stream","request":request,"event":request},
            {"name":"items.changed","kind":"event","description":"Event","event":request}
        ]);
        Self {
            document: json!({"schemaVersion":1,"$defs":{"Result":{"type":"object","properties":{"value":{"type":"integer"}},"required":["value"],"additionalProperties":false}},"contracts":[{"identity":"dev.test.reads","major":1,"minor":0,"description":"Unknown provider","ports":ports}]}),
            provider: "dev.test.provider".into(),
            major: 1,
            enabled: false,
            invocations: 0,
            cleanup_fails: false,
            result: json!({"value":42}),
            guide: "Unknown provider guide".into(),
            failure: None,
            describes: 0,
        }
    }
    pub fn call(&mut self, method: &str, p: Value) -> Result<Value, BackendError> {
        assert_eq!(p["access"], "person");
        let contract = self.document["contracts"][0]["identity"].clone();
        let bytes = serde_json::to_vec(&self.document).unwrap();
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        match method {
            "available" => Ok(
                json!({"contracts":[{"contract":contract,"major":self.major,"minor":0,"description":"Reads","available":true,"enabled":self.enabled,"enabledMajor":self.enabled.then_some(self.major),"metadataSha256":digest,"provider":{"identity":self.provider,"version":"1.0.0","displayName":"Unknown","description":null},"guideEligibility":"eligible","routeRevision":"a".repeat(64)}]}),
            ),
            "describe" => {
                assert_eq!(p["routeRevision"], "a".repeat(64));
                assert_eq!(p["metadataSha256"], digest);
                self.describes += 1;
                let offset = p["offset"].as_u64().unwrap() as usize;
                let end = (offset + 65536).min(bytes.len());
                let mut chunk = json!({"metadataSha256":digest,"totalBytes":bytes.len(),"offset":offset,"data":STANDARD.encode(&bytes[offset..end])});
                if end < bytes.len() {
                    chunk["nextOffset"] = end.into();
                }
                Ok(chunk)
            }
            "enable" => {
                assert_eq!(p["routeRevision"], "a".repeat(64));
                self.enabled = true;
                Ok(json!({"access":"person","contract":contract,"major":self.major,"enabled":true}))
            }
            "disable" => {
                if self.cleanup_fails {
                    return Err(BackendError::new("port_unavailable", false, json!({})));
                }
                self.enabled = false;
                Ok(json!({"access":"person","contract":p["contract"],"enabled":false}))
            }
            "invoke" => {
                assert_eq!(p["routeRevision"], "a".repeat(64));
                assert_eq!(
                    p["kind"],
                    if p["port"] == "write.item" {
                        "command"
                    } else {
                        "query"
                    }
                );
                self.invocations += 1;
                if let Some(e) = &self.failure {
                    return Err(e.clone());
                }
                let value = if p["port"] == "runtime.guide" {
                    json!({"markdown":self.guide})
                } else {
                    self.result.clone()
                };
                Ok(json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":value}))
            }
            _ => panic!("unexpected method"),
        }
    }
}

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
pub async fn start(provider: Arc<Mutex<Provider>>, directory: &std::path::Path) -> Harness {
    start_router(provider, directory, backend_environment::router()).await
}
pub async fn start_router(
    provider: Arc<Mutex<Provider>>,
    directory: &std::path::Path,
    r: teamarness_package_sdk::Router<State>,
) -> Harness {
    let script = ScriptedCapability::new().calls(move |m, p| provider.lock().unwrap().call(m, p));
    let mut h = HarnessBuilder::new(r.into_backend())
        .data_directory(directory.canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", script, 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    h
}
pub async fn select(h: &mut Harness) {
    let tools = h
        .request(
            "external-gateway.catalog.page",
            json!({"contract":"dev.test.reads"}),
        )
        .await
        .unwrap();
    let fingerprint = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["port"] == "read.item")
        .unwrap()["fingerprint"]
        .clone();
    h.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":"read.item","fingerprint":fingerprint,"enabled":true})).await.unwrap();
}
pub async fn enable(h: &mut Harness) -> (u16, String) {
    let t = h
        .request(
            "external-gateway.token.create",
            json!({"label":"socket test"}),
        )
        .await
        .unwrap();
    h.request("external-gateway.listener.set", json!({"enabled":true}))
        .await
        .unwrap();
    let s = h
        .request("external-gateway.status.get", json!({}))
        .await
        .unwrap();
    assert_eq!(
        s["running"], true,
        "enable must publish a running listener only after saving its port"
    );
    (
        s["port"].as_u64().unwrap() as u16,
        t["bearer"].as_str().unwrap().to_owned(),
    )
}
pub async fn raw(port: u16, bytes: &[u8]) -> Vec<u8> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(bytes).await.unwrap();
    let mut out = vec![];
    tokio::time::timeout(
        std::time::Duration::from_secs(30),
        stream.read_to_end(&mut out),
    )
    .await
    .unwrap()
    .unwrap();
    out
}
pub async fn get(port: u16, bearer: &str, path: &str) -> (u16, Value) {
    response(&raw(port,format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\n\r\n").as_bytes()).await)
}
pub fn response(bytes: &[u8]) -> (u16, Value) {
    let text = std::str::from_utf8(bytes).unwrap();
    let (head, body) = text.split_once("\r\n\r\n").expect("HTTP response");
    assert!(
        head.to_ascii_lowercase()
            .contains("cache-control: no-store")
    );
    assert!(
        head.to_ascii_lowercase()
            .contains("x-content-type-options: nosniff")
    );
    (
        head.split_whitespace().nth(1).unwrap().parse().unwrap(),
        serde_json::from_str(body).unwrap(),
    )
}
pub async fn post(port: u16, bearer: &str, value: Value) -> (u16, Value) {
    let body = serde_json::to_vec(&value).unwrap();
    let mut bytes=format!("POST /v1/call HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {bearer}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",body.len()).into_bytes();
    bytes.extend(body);
    response(&raw(port, &bytes).await)
}
pub fn call(request: Value) -> Value {
    json!({"contract":"dev.test.reads","major":1,"port":"read.item","request":request})
}
/// The only paused dependency is the SDK's actual scripted capability. Never sleep to schedule a race.
pub struct Pause {
    pub entered: tokio::sync::Notify,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl Pause {
    pub fn wait(&self) {
        self.release
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("test releases capability barrier");
    }
}
pub struct Release(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}
pub fn pause() -> (Arc<Pause>, Release) {
    let (send, recv) = std::sync::mpsc::channel();
    (
        Arc::new(Pause {
            entered: tokio::sync::Notify::new(),
            release: Mutex::new(recv),
        }),
        Release(Some(send)),
    )
}
pub type CapabilityPause = Arc<Mutex<Option<(String, Arc<Pause>)>>>;
pub async fn scripted(
    provider: Arc<Mutex<Provider>>,
    directory: &std::path::Path,
    armed: CapabilityPause,
) -> (
    Harness,
    Arc<teamarness_generated_package_backend::policy::Policy>,
) {
    let (send, recv) = tokio::sync::oneshot::channel();
    let send = Arc::new(Mutex::new(Some(send)));
    let r = backend_environment::router().request(
        "test.capture",
        move |ctx: teamarness_package_sdk::Context<State>, _: teamarness_package_sdk::Empty| {
            let send = send.clone();
            async move {
                send.lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(ctx.state().policy.clone())
                    .ok();
                Ok(json!({}))
            }
        },
    );
    let script = ScriptedCapability::new().calls(move |method, payload| {
        let pause = {
            let mut armed = armed.lock().unwrap();
            if armed.as_ref().is_some_and(|(m, _)| m == method) {
                armed.take().map(|(_, p)| p)
            } else {
                None
            }
        };
        if let Some(p) = pause {
            p.entered.notify_one();
            p.wait();
        }
        provider.lock().unwrap().call(method, payload)
    });
    let mut h = HarnessBuilder::new(r.into_backend())
        .data_directory(directory.canonicalize().unwrap())
        .installation(&"b".repeat(64), "Local")
        .capability("teamarness.membership", membership_support::present())
        .capability_version("teamarness.package-ports", script, 3, 0)
        .start()
        .await;
    h.activate().await.unwrap();
    h.request("test.capture", json!({})).await.unwrap();
    (h, recv.await.unwrap())
}
