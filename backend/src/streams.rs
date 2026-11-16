//! Connection-owned subscriptions: separate capacity, bounded handoff and cancellation.
use crate::{error, limits::*, policy::Policy};
use bytes::Bytes;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use teamarness_package_sdk::{BackendError, CancellationToken, CapabilitySubscription};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinSet,
};

#[derive(Clone)]
pub(crate) struct Grant {
    pub token: String,
    pub generation: u64,
    pub contract: String,
    pub major: u16,
    pub provider: String,
    pub port: String,
    pub fingerprint: String,
    pub cancel: CancellationToken,
    pub finished: Arc<AtomicBool>,
}
pub(crate) type Registry = BTreeMap<u64, Grant>;
pub(crate) struct Streams {
    slots: Arc<Semaphore>,
    tasks: Mutex<JoinSet<()>>,
    next: AtomicU64,
    cancellations: Mutex<Vec<(CancellationToken, std::sync::Weak<AtomicBool>)>>,
}
impl Default for Streams {
    fn default() -> Self {
        Self {
            slots: Arc::new(Semaphore::new(MAX_STREAMS)),
            tasks: Mutex::new(JoinSet::new()),
            next: AtomicU64::new(0),
            cancellations: Mutex::new(vec![]),
        }
    }
}
impl Streams {
    pub fn reserve(&self) -> Result<(u64, OwnedSemaphorePermit), BackendError> {
        Ok((
            self.next.fetch_add(1, Ordering::Relaxed),
            self.slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| error("busy"))?,
        ))
    }
    pub fn own(
        &self,
        cancel: &CancellationToken,
        finished: &Arc<AtomicBool>,
        work: impl std::future::Future<Output = ()> + Send + 'static,
    ) {
        let mut signals = self.cancellations.lock().expect("stream signals");
        signals.retain(|(_, f)| f.upgrade().is_some_and(|f| !f.load(Ordering::SeqCst)));
        signals.push((cancel.clone(), Arc::downgrade(finished)));
        let mut tasks = self.tasks.lock().expect("stream tasks");
        while tasks.try_join_next().is_some() {}
        if !self.slots.is_closed() {
            tasks.spawn(work);
        }
    }
    pub fn cancel_all(&self) {
        for (cancel, _) in self.cancellations.lock().expect("stream signals").iter() {
            cancel.cancel();
        }
    }
    pub fn retire(&self) {
        self.cancel_all();
        self.slots.close();
        self.tasks.lock().expect("stream tasks").abort_all();
    }
    pub async fn dispose(&self) {
        self.retire();
        let mut tasks = std::mem::take(&mut *self.tasks.lock().expect("stream tasks"));
        while tasks.join_next().await.is_some() {}
    }
}
pub(crate) struct Finish(pub Arc<AtomicBool>);
impl Drop for Finish {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

pub struct Output {
    pub(crate) receiver: mpsc::Receiver<Bytes>,
    pub(crate) cancel: CancellationToken,
}
impl Drop for Output {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) fn channel(cancel: CancellationToken) -> (mpsc::Sender<Bytes>, Output) {
    let (sender, receiver) = mpsc::channel(1);
    (sender, Output { receiver, cancel })
}
pub(crate) fn encode(value: Value) -> Bytes {
    let mut bytes = serde_json::to_vec(&value).expect("JSON frame");
    bytes.push(b'\n');
    Bytes::from(bytes)
}
pub(crate) async fn forward(
    policy: Arc<Policy>,
    grant: Grant,
    mut subscription: CapabilitySubscription,
    sender: mpsc::Sender<Bytes>,
) {
    let mut heartbeat =
        tokio::time::Instant::now() + Duration::from_secs(STREAM_HEARTBEAT_SECONDS as u64);
    loop {
        // Reserve before polling the SDK. One waiting frame plus one writer-owned frame.
        let permit = tokio::select! {biased;
            _=grant.cancel.cancelled()=>break,
            p=sender.reserve()=>match p {Ok(p)=>p,Err(_)=>break},
        };
        let next = tokio::select! {biased;
            _=grant.cancel.cancelled()=>break,
            _=sender.closed()=>break,
            v=subscription.recv_with_credit()=>Some(v),
            _=tokio::time::sleep_until(heartbeat)=>None,
        };
        let (value, credit, terminal) = match next {
            None => (json!({"type":"heartbeat"}), None, false),
            Some(Some(Ok((value, credit)))) => {
                if serde_json::to_vec(&value).map_or(true, |b| b.len() > PORT_VALUE_BYTES) {
                    (
                        json!({"type":"end","reason":"error","error":crate::http::error_value(error("too_large"))}),
                        None,
                        true,
                    )
                } else {
                    (json!({"type":"value","value":value}), Some(credit), false)
                }
            }
            Some(Some(Err(e))) => (
                json!({"type":"end","reason":"error","error":crate::http::stream_error_value(e)}),
                None,
                true,
            ),
            Some(None) => (json!({"type":"end","reason":"completed"}), None, true),
        };
        let frame = encode(value);
        if frame.len() > PORT_VALUE_BYTES + STREAM_FRAME_ALLOWANCE_BYTES {
            break;
        }
        // This owner covers the local output admission, never credit/socket/cleanup waits.
        if !policy.admit_output(&grant, permit, frame).await {
            break;
        }
        if let Some(credit) = credit {
            tokio::select! {biased;_=grant.cancel.cancelled()=>break,_=sender.closed()=>break,result=credit.release()=>{if result.is_err(){break}}}
        }
        if terminal {
            break;
        }
        heartbeat =
            tokio::time::Instant::now() + Duration::from_secs(STREAM_HEARTBEAT_SECONDS as u64);
    }
    // Drop is the bounded safety net if a writer cannot accept close.
    let _ = tokio::time::timeout(
        Duration::from_secs(LISTENER_DRAIN_TIMEOUT as u64),
        subscription.close(),
    )
    .await;
}
pub(crate) type Opening = oneshot::Sender<Result<Output, BackendError>>;
