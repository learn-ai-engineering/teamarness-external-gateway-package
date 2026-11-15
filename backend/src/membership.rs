//! One activation-owned membership stream; roster authority is never inferred from event digests.
use crate::{error, policy::Policy};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use teamarness_package_sdk::{
    BackendError, CapabilityClient, CapabilitySubscription, Initialization,
    capabilities::membership::{IDENTITY, Membership, MembershipSnapshot},
};
use tokio::{sync::Mutex, task::JoinHandle};

pub struct Monitor {
    membership: Membership,
    client: CapabilityClient,
    grant: String,
    workspace: String,
    installation: String,
    task: Mutex<Option<JoinHandle<()>>>,
}
impl Monitor {
    pub fn new(init: &Initialization, client: CapabilityClient) -> Result<Self, BackendError> {
        let grant = init
            .capability(IDENTITY)
            .ok_or_else(|| error("unavailable"))?;
        if grant.major != 1 || !digest(&init.installation().identity) {
            return Err(error("unavailable"));
        }
        Ok(Self {
            membership: Membership::from_initialization(init, client.clone())?,
            client,
            grant: grant.grant.clone(),
            workspace: init.workspace().id.clone(),
            installation: init.installation().identity.clone(),
            task: Mutex::new(None),
        })
    }
    pub async fn start(&self, policy: &Arc<Policy>) {
        let mut task = self.task.lock().await;
        // Resume replaces a terminated stream only; a live watcher remains authoritative
        // throughout manual suspension. There is no automatic reconnect or polling loop.
        if task.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        if let Some(old) = task.take() {
            let _ = old.await;
        }
        let observer = Observer {
            membership: self.membership.clone(),
            client: self.client.clone(),
            grant: self.grant.clone(),
            workspace: self.workspace.clone(),
            installation: self.installation.clone(),
        };
        let policy = policy.clone();
        let (ready, initialized) = tokio::sync::oneshot::channel();
        *task = Some(tokio::spawn(async move {
            let mut stream = None;
            tokio::select! {
                biased;
                _ = policy.retired.cancelled() => {},
                _ = observer.run(&policy, &mut stream, ready) => {},
            }
            if let Some(mut stream) = stream {
                stream.close().await;
            }
        }));
        let _ = initialized.await;
    }
    pub async fn join(&self) {
        if let Some(task) = self.task.lock().await.take() {
            let _ = task.await;
        }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().take() {
            task.abort();
        }
    }
}
struct Observer {
    membership: Membership,
    client: CapabilityClient,
    grant: String,
    workspace: String,
    installation: String,
}
impl Observer {
    async fn current(&self, minimum: u64) -> Result<MembershipSnapshot, BackendError> {
        // Keep the sole snapshot future owned until its SDK terminal (including its
        // existing deadline). Dropping a shorter observer timeout would abandon SDK work.
        let snapshot = self.membership.current().await?;
        if snapshot.workspace_id != self.workspace
            || snapshot.epoch < minimum
            || snapshot.members.iter().any(|m| !digest(&m.identity))
        {
            return Err(error("unavailable"));
        }
        let mut identities = std::collections::BTreeSet::new();
        if snapshot
            .members
            .iter()
            .any(|m| !identities.insert(&m.identity))
        {
            return Err(error("unavailable"));
        }
        Ok(snapshot)
    }
    async fn run(
        &self,
        policy: &Arc<Policy>,
        stream: &mut Option<CapabilitySubscription>,
        ready: tokio::sync::oneshot::Sender<()>,
    ) {
        policy.membership_unavailable().await;
        let Ok(first) = self.current(0).await else {
            return;
        };
        if !self.present(&first) {
            policy.membership_observed(false).await;
        }
        let Ok(opened) = self
            .client
            .subscribe(
                &self.grant,
                json!({"method":"subscribe","afterEpoch":first.epoch.to_string()}),
            )
            .await
        else {
            return;
        };
        *stream = Some(opened);
        let stream = stream.as_mut().expect("opened membership stream");
        let mut minimum = first.epoch;
        let mut after_event = first.epoch;
        let mut observation = self.current(minimum).await;
        let mut ready = Some(ready);
        loop {
            // Prefer every already-buffered event/terminal over publishing a snapshot.
            let incoming = tokio::select! {
                biased;
                event = stream.recv_with_credit() => Some(event),
                _ = std::future::ready(()) => None,
            };
            let incoming = match incoming {
                Some(event) => event,
                None => {
                    match &observation {
                        Ok(snapshot) => {
                            minimum = minimum.max(snapshot.epoch);
                            policy.membership_observed(self.present(snapshot)).await;
                        }
                        Err(_) => policy.membership_unavailable().await,
                    }
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(());
                    }
                    stream.recv_with_credit().await
                }
            };
            // Initialization need not drain an indefinitely busy event source before
            // returning: it may finish closed, and the owned watcher reconciles later.
            if let Some(ready) = ready.take() {
                let _ = ready.send(());
            }
            policy.membership_unavailable().await;
            let Some(Ok((value, credit))) = incoming else {
                return;
            };
            let parsed = serde_json::from_value::<Transition>(value)
                .ok()
                .and_then(|event| event.epoch_after(after_event));
            if let Some(epoch) = parsed {
                after_event = epoch;
                minimum = minimum.max(epoch);
                observation = self.current(minimum).await;
            } else {
                observation = Err(error("unavailable"));
            }
            if credit.release().await.is_err() {
                return;
            }
        }
    }
    fn present(&self, snapshot: &MembershipSnapshot) -> bool {
        snapshot
            .members
            .iter()
            .any(|m| m.identity == self.installation)
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Transition {
    epoch: String,
    membership_version: String,
    cause: String,
    snapshot_digest: String,
}
impl Transition {
    fn epoch_after(&self, previous: u64) -> Option<u64> {
        let epoch: u64 = self.epoch.parse().ok()?;
        (epoch > previous
            && epoch.to_string() == self.epoch
            && digest(&self.membership_version)
            && digest(&self.cause)
            && digest(&self.snapshot_digest))
        .then_some(epoch)
    }
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
