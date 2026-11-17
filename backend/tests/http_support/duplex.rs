//! Controlled Host capability peer over real production SDK duplex I/O.
//! Unlike ScriptedCapability, this seam can hold confirmation and address multiple streams.
#![allow(dead_code)]
use super::Provider;
use crate::http_support::backend_environment;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use teamarness_generated_package_backend::{State, policy::Policy};
use teamarness_package_sdk::{Context, Empty, Package};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
    sync::{Notify, mpsc, oneshot},
    task::JoinHandle,
};
type Answers = Arc<Mutex<BTreeMap<String, oneshot::Sender<Result<Value, Value>>>>>;
#[derive(Clone)]
pub struct Peer {
    writer: Arc<tokio::sync::Mutex<DuplexStream>>,
    streams: Arc<Mutex<BTreeMap<String, (u64, u64)>>>,
    credit: Arc<Notify>,
    answers: Answers,
    next: Arc<AtomicU64>,
    pub hold: Arc<AtomicBool>,
    pub membership: Arc<Mutex<Option<String>>>,
    pub present: Arc<AtomicBool>,
}
pub struct Fixture {
    pub peer: Peer,
    pub opens: mpsc::UnboundedReceiver<(String, Value)>,
    pub policy: Arc<Policy>,
    pump: JoinHandle<()>,
    service: JoinHandle<Result<(), teamarness_package_sdk::SdkError>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.pump.abort();
        self.service.abort();
    }
}
async fn read(s: &mut DuplexStream) -> Value {
    let n = s.read_u32().await.expect("SDK frame length");
    assert!(n <= 1048576);
    let mut b = vec![0; n as usize];
    s.read_exact(&mut b).await.expect("SDK frame");
    serde_json::from_slice(&b).unwrap()
}
impl Peer {
    async fn write(&self, v: Value) {
        let b = serde_json::to_vec(&v).unwrap();
        let mut s = self.writer.lock().await;
        s.write_u32(b.len() as u32).await.unwrap();
        s.write_all(&b).await.unwrap();
    }
    pub async fn request(&self, operation: &str, payload: Value) -> Result<Value, Value> {
        let id = format!("h:{}", self.next.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = oneshot::channel();
        self.answers.lock().unwrap().insert(id.clone(), tx);
        let deadline = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
            + Duration::from_secs(60))
        .as_millis()
        .to_string();
        self.write(json!({"kind":"request","id":id,"operation":operation,"payload":payload,"deadlineUnixMs":deadline})).await;
        tokio::time::timeout(Duration::from_secs(65), rx)
            .await
            .unwrap()
            .unwrap()
    }
    pub async fn ready(&self, id: &str) {
        self.write(json!({"kind":"stream_ready","streamId":id}))
            .await;
    }
    pub async fn close(&self, id: &str, code: &str) {
        self.write(json!({"kind":"stream_close","streamId":id,"code":code}))
            .await;
    }
    pub fn is_open(&self, id: &str) -> bool {
        self.streams.lock().unwrap().contains_key(id)
    }
    pub fn remaining_credit(&self, id: &str) -> u64 {
        self.streams.lock().unwrap().get(id).map_or(0, |s| s.0)
    }
    pub async fn send(&self, id: &str, value: Value) -> bool {
        self.send_payload(
            id,
            json!({"$teamarnessPortEnvelope":1,"$teamarnessPortValue":value}),
        )
        .await
    }
    pub async fn send_payload(&self, id: &str, value: Value) -> bool {
        loop {
            let notified = self.credit.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let seq = {
                let mut streams = self.streams.lock().unwrap();
                let Some(s) = streams.get_mut(id) else {
                    return false;
                };
                if s.0 > 0 {
                    s.0 -= 1;
                    s.1 += 1;
                    Some(s.1.to_string())
                } else {
                    None
                }
            };
            if let Some(seq) = seq {
                self.write(json!({"kind":"event","streamId":id,"sequence":seq,"payload":value}))
                    .await;
                return true;
            }
            notified.await;
        }
    }
    pub async fn closed(&self, id: &str) {
        tokio::time::timeout(Duration::from_secs(7), async {
            loop {
                let n = self.credit.notified();
                tokio::pin!(n);
                n.as_mut().enable();
                if !self.is_open(id) {
                    break;
                }
                n.await;
            }
        })
        .await
        .unwrap();
    }
}
impl Fixture {
    pub async fn start(
        directory: &std::path::Path,
        provider: Arc<Mutex<Provider>>,
        hold: bool,
        fault: Option<Arc<dyn teamarness_generated_package_backend::config::StorageFault>>,
    ) -> Self {
        let (mut host_read, sdk_write) = tokio::io::duplex(1 << 20);
        let (sdk_read, host_write) = tokio::io::duplex(1 << 20);
        let (capture, captured) = oneshot::channel();
        let capture = Arc::new(Mutex::new(Some(capture)));
        let router = if let Some(fault) = fault {
            backend_environment::router_with_storage_fault(
                Arc::new(teamarness_generated_package_backend::tokens::OsEntropy),
                Some(fault),
            )
        } else {
            backend_environment::router()
        };
        let router = router.request("test.capture", move |c: Context<State>, _: Empty| {
            let capture = capture.clone();
            async move {
                capture
                    .lock()
                    .unwrap()
                    .take()
                    .unwrap()
                    .send(c.state().policy.clone())
                    .ok();
                Ok(json!({}))
            }
        });
        let service = tokio::spawn(
            Package::from_backend(router.into_backend()).serve_io(sdk_read, sdk_write),
        );
        let peer = Peer {
            writer: Arc::new(tokio::sync::Mutex::new(host_write)),
            streams: Default::default(),
            credit: Default::default(),
            answers: Default::default(),
            next: Arc::new(AtomicU64::new(1)),
            hold: Arc::new(AtomicBool::new(hold)),
            membership: Default::default(),
            present: Arc::new(AtomicBool::new(true)),
        };
        peer.write(json!({"kind":"initialize","nonce":"bm9uY2U","package":{"identity":"dev.teamarness.external-gateway","version":"1.0.0","manifestDigest":"a".repeat(64),"backendDigest":"b".repeat(64)},"uiContractDigest":"c".repeat(64),"processGeneration":"1","workspace":{"id":"workspace1","name":"Workspace"},"installation":{"identity":"b".repeat(64),"displayName":"Local"},"dataDirectory":{"path":directory.canonicalize().unwrap(),"continuityId":"Y29udGludWl0eQ"},"protocol":{"major":1,"minor":1},"capabilities":[{"identity":"teamarness.membership","grant":"bWVtYmVyc2hpcA","major":1,"minor":0},{"identity":"teamarness.package-ports","grant":"cG9ydHM","major":3,"minor":0}],"limits":{"maxFrameBytes":1048576,"maxInFlightEachDirection":32,"maxWriterFrames":64,"maxWriterBytes":8388608,"initialStreamCredit":32}})).await;
        assert_eq!(read(&mut host_read).await["kind"], "initialized");
        let (tx, opens) = mpsc::unbounded_channel();
        let pump_peer = peer.clone();
        let pump = tokio::spawn(async move {
            loop {
                let frame = read(&mut host_read).await;
                let id = frame["id"].as_str().unwrap_or("").to_owned();
                match frame["kind"].as_str().unwrap() {
                    "request" => {
                        let p = &frame["payload"];
                        let result = if frame["grant"] == "bWVtYmVyc2hpcA" {
                            Ok(
                                json!({"workspaceId":"workspace1","epoch":if pump_peer.present.load(Ordering::SeqCst){"1"}else{"2"},"membershipVersion":"a".repeat(64),"members":if pump_peer.present.load(Ordering::SeqCst){vec![json!({"identity":"b".repeat(64),"role":"member"})]}else{vec![]}}),
                            )
                        } else {
                            let mut payload = p.clone();
                            let method = payload
                                .as_object_mut()
                                .unwrap()
                                .remove("method")
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned();
                            provider.lock().unwrap().call(&method, payload)
                        };
                        let response = match result {
                            Ok(value) => json!({"kind":"response","id":id,"payload":value}),
                            Err(e) => {
                                json!({"kind":"error","id":id,"code":e.code(),"retryable":e.retryable(),"outcomeUncertain":e.outcome_uncertain(),"details":e.details()})
                            }
                        };
                        pump_peer.write(response).await;
                    }
                    "response" | "error" => {
                        let answer = pump_peer.answers.lock().unwrap().remove(&id);
                        if let Some(answer) = answer {
                            let result = if frame["kind"] == "response" {
                                Ok(frame["payload"].clone())
                            } else {
                                Err(frame.clone())
                            };
                            let _ = answer.send(result);
                        }
                    }
                    "stream_open" => {
                        if frame["payload"]["grant"] == "cG9ydHM" {
                            assert_eq!(frame["requireReady"], true);
                            pump_peer
                                .streams
                                .lock()
                                .unwrap()
                                .insert(id.clone(), (frame["initialCredit"].as_u64().unwrap(), 0));
                            if !pump_peer.hold.load(Ordering::SeqCst) {
                                pump_peer.ready(&id).await;
                            }
                            tx.send((id, frame["payload"].clone())).unwrap();
                        } else {
                            pump_peer
                                .streams
                                .lock()
                                .unwrap()
                                .insert(id.clone(), (frame["initialCredit"].as_u64().unwrap(), 0));
                            *pump_peer.membership.lock().unwrap() = Some(id);
                        }
                    }
                    "stream_close" => {
                        pump_peer
                            .streams
                            .lock()
                            .unwrap()
                            .remove(frame["streamId"].as_str().unwrap());
                        pump_peer.credit.notify_waiters();
                    }
                    "credit" => {
                        if let Some(s) = pump_peer
                            .streams
                            .lock()
                            .unwrap()
                            .get_mut(frame["streamId"].as_str().unwrap())
                        {
                            s.0 += frame["amount"].as_u64().unwrap();
                        }
                        pump_peer.credit.notify_waiters();
                    }
                    "shutdown_ack" => break,
                    other => panic!("unexpected SDK frame {other}"),
                }
            }
        });
        peer.request("lifecycle.activate", json!({})).await.unwrap();
        peer.request("test.capture", json!({})).await.unwrap();
        let policy = captured.await.unwrap();
        Self {
            peer,
            opens,
            policy,
            pump,
            service,
        }
    }
    pub async fn select(&self, name: &str, enabled: bool) {
        let tools = self
            .peer
            .request(
                "external-gateway.catalog.page",
                json!({"contract":"dev.test.reads"}),
            )
            .await
            .unwrap();
        let t = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["port"] == name)
            .unwrap();
        self.peer.request("external-gateway.tool.set",json!({"contract":"dev.test.reads","major":1,"provider":"dev.test.provider","port":name,"fingerprint":t["fingerprint"],"enabled":enabled})).await.unwrap();
    }
    pub async fn enable(&self) -> (u16, String) {
        let token = self
            .peer
            .request(
                "external-gateway.token.create",
                json!({"label":"duplex socket"}),
            )
            .await
            .unwrap();
        self.peer
            .request("external-gateway.listener.set", json!({"enabled":true}))
            .await
            .unwrap();
        let status = self
            .peer
            .request("external-gateway.status.get", json!({}))
            .await
            .unwrap();
        (
            status["port"].as_u64().unwrap() as u16,
            token["bearer"].as_str().unwrap().to_owned(),
        )
    }
    pub async fn interrupt(&self) {
        self.peer.writer.lock().await.shutdown().await.unwrap();
        self.pump.abort();
    }
    pub async fn dispose(&self) {
        self.peer
            .request("lifecycle.dispose", json!({}))
            .await
            .unwrap();
    }
}
