//! Single shared durable authorization owner. Cancelled observers never release write ownership.
use crate::{
    catalog::{Catalog, Observed, excluded},
    config::{Config, ContractSelection, FileStore, MAX_TOKENS, ToolSelection},
    error,
    tokens::{self, Entropy},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use teamarness_package_sdk::BackendError;
use tokio::sync::{Mutex, watch};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolChange {
    pub contract: String,
    pub major: u16,
    pub provider: String,
    pub port: String,
    pub fingerprint: String,
    pub enabled: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractChoice {
    pub contract: String,
    pub major: u16,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageChange {
    pub provider: String,
    pub enabled: bool,
    pub contracts: Vec<ContractChoice>,
}
pub enum Change {
    Listener(bool),
    Port(u16),
    Package(PackageChange),
    Tool(ToolChange),
    Create(String),
    Regenerate(String),
    Revoke(String),
    DisableAll,
}
struct Inner {
    config: Config,
    storage_failed: bool,
    warning: Option<&'static str>,
}
pub struct Policy {
    inner: Arc<Mutex<Inner>>,
    store: Arc<FileStore>,
    catalog: Catalog,
    entropy: Arc<dyn Entropy>,
    status: watch::Sender<Value>,
    active: AtomicBool,
}
impl Policy {
    pub async fn open(
        store: FileStore,
        catalog: Catalog,
        entropy: Arc<dyn Entropy>,
    ) -> Result<Arc<Self>, BackendError> {
        let store = Arc::new(store);
        let reader = store.clone();
        let config = tokio::task::spawn_blocking(move || reader.read())
            .await
            .map_err(|_| error("storage_failed"))?
            .map_err(|_| error("storage_failed"))?;
        let inner = Inner {
            config,
            storage_failed: false,
            warning: None,
        };
        let (status, _) = watch::channel(status_value(&inner));
        Ok(Arc::new(Self {
            inner: Arc::new(Mutex::new(inner)),
            store,
            catalog,
            entropy,
            status,
            active: AtomicBool::new(true),
        }))
    }
    pub fn status(&self) -> Value {
        self.status.borrow().clone()
    }
    pub fn watch(&self) -> watch::Receiver<Value> {
        self.status.subscribe()
    }
    pub async fn suspend(&self) {
        self.active.store(false, Ordering::SeqCst);
        let _owner = self.inner.lock().await;
    }
    pub fn resume(&self) {
        self.active.store(true, Ordering::SeqCst);
    }
    /// No mutation queue: one owned transaction, independent of the observing UI request.
    pub async fn change(self: &Arc<Self>, change: Change) -> Result<Value, BackendError> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(error("unavailable"));
        }
        let mut owner = self
            .inner
            .clone()
            .try_lock_owned()
            .map_err(|_| error("busy"))?;
        let this = self.clone();
        tokio::spawn(async move {
            let result = this.apply(&mut owner, change).await;
            this.status.send_replace(status_value(&owner));
            result
        })
        .await
        .map_err(|_| error("storage_failed"))?
    }
    async fn persist(&self, inner: &mut Inner, next: Config) -> Result<(), BackendError> {
        next.validate().map_err(|_| error("too_large"))?;
        let denied = denial_intersection(&inner.config, &next);
        let store = self.store.clone();
        let to_write = next.clone();
        // Gate ownership lives in the spawned transaction, not its UI waiter.
        let result = tokio::task::spawn_blocking(move || store.write(&to_write)).await;
        match result {
            Ok(Ok(())) => {
                inner.config = next;
                inner.storage_failed = false;
                Ok(())
            }
            _ => {
                inner.storage_failed = true;
                // Reread uncertain completion before any subsequent authorization. Remain denied
                // even after successful reread until an explicit durable user action succeeds.
                let store = self.store.clone();
                let _reread = tokio::task::spawn_blocking(move || store.read()).await;
                inner.config = denied;
                Err(error("storage_failed"))
            }
        }
    }
    async fn apply(&self, inner: &mut Inner, change: Change) -> Result<Value, BackendError> {
        let mut next = inner.config.clone();
        let mut cleanup = vec![];
        let result = match change {
            Change::Listener(enabled) => {
                next.desired_enabled = enabled;
                ok()
            }
            Change::Port(port) => {
                if port == 0 {
                    return Err(error("bad_request"));
                }
                next.port = Some(port);
                ok()
            }
            Change::Create(label) => {
                if next.tokens.len() >= MAX_TOKENS {
                    return Err(error("too_large"));
                }
                let (record, bearer) = tokens::issue(&label, 1, None, self.entropy.as_ref())
                    .map_err(|_| error("unavailable"))?;
                if next.tokens.iter().any(|t| t.id == record.id) {
                    return Err(error("unavailable"));
                }
                let id = record.id.clone();
                next.tokens.push(record);
                json!({"id":id,"bearer":bearer})
            }
            Change::Regenerate(id) => {
                let old = next
                    .tokens
                    .iter_mut()
                    .find(|t| t.id == id)
                    .ok_or_else(|| error("not_found"))?;
                let generation = old
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| error("unavailable"))?;
                let (record, bearer) =
                    tokens::issue(&old.label, generation, Some(&id), self.entropy.as_ref())
                        .map_err(|_| error("unavailable"))?;
                *old = record;
                json!({"id":id,"bearer":bearer})
            }
            Change::Revoke(id) => {
                next.tokens.retain(|t| t.id != id);
                ok()
            }
            Change::DisableAll => {
                next.desired_enabled = false;
                next.tokens.clear();
                cleanup.extend(next.contracts.keys().cloned());
                next.contracts.clear();
                ok()
            }
            Change::Package(change) => {
                if !change.enabled {
                    cleanup.extend(
                        next.contracts
                            .iter()
                            .filter(|(_, c)| c.provider == change.provider)
                            .map(|(id, _)| id.clone()),
                    );
                    next.contracts.retain(|_, c| c.provider != change.provider);
                } else {
                    if change.contracts.is_empty() || change.contracts.len() > 256 {
                        return Err(error("bad_request"));
                    }
                    // Check capacity before any Host effect. Compile one bounded document at a time.
                    for c in &change.contracts {
                        next.contracts
                            .entry(c.contract.clone())
                            .and_modify(|stored| {
                                if stored.provider != change.provider || stored.major != c.major {
                                    stored.tools.clear();
                                }
                                stored.provider = change.provider.clone();
                                stored.major = c.major;
                                stored.enabled = true;
                            })
                            .or_insert(ContractSelection {
                                major: c.major,
                                provider: change.provider.clone(),
                                enabled: true,
                                tools: BTreeMap::new(),
                            });
                    }
                    next.validate().map_err(|_| error("too_large"))?;
                    tokio::time::timeout(
                        std::time::Duration::from_secs(crate::limits::CATALOG_READ_TIMEOUT as u64),
                        async {
                            for choice in &change.contracts {
                                let observed = self.catalog.observe(&choice.contract).await?;
                                exact(&observed, choice.major, &change.provider)?;
                                self.catalog.enable(&observed).await?;
                            }
                            Ok::<(), BackendError>(())
                        },
                    )
                    .await
                    .map_err(|_| error("timeout"))??;
                }
                ok()
            }
            Change::Tool(change) => {
                if change.enabled {
                    if change.port == "runtime.guide" {
                        return Err(error("forbidden"));
                    }
                    let observed = self.catalog.observe(&change.contract).await?;
                    exact(&observed, change.major, &change.provider)?;
                    if !observed.ordinary(&change.port)
                        || observed.fingerprint(&change.port)? != change.fingerprint
                    {
                        return Err(error("forbidden"));
                    }
                    let c = next.contracts.entry(change.contract.clone()).or_insert(
                        ContractSelection {
                            major: change.major,
                            provider: change.provider.clone(),
                            enabled: true,
                            tools: BTreeMap::new(),
                        },
                    );
                    if c.major != change.major || c.provider != change.provider {
                        c.tools.clear();
                    }
                    c.major = change.major;
                    c.provider = change.provider;
                    c.enabled = true;
                    c.tools.insert(
                        change.port,
                        ToolSelection {
                            fingerprint: change.fingerprint,
                        },
                    );
                    next.validate().map_err(|_| error("too_large"))?;
                    self.catalog.enable(&observed).await?;
                } else if let Some(c) = next.contracts.get_mut(&change.contract) {
                    c.tools.remove(&change.port);
                    if c.tools.is_empty() {
                        c.enabled = false;
                        cleanup.push(change.contract);
                    }
                }
                ok()
            }
        };
        self.persist(inner, next).await?;
        for contract in cleanup {
            if self.catalog.disable(&contract).await.is_err() {
                inner.warning = Some("host_cleanup_failed");
            }
        }
        if result.get("ok").is_some() {
            Ok(json!({"ok":true,"warning":inner.warning}))
        } else {
            Ok(result)
        }
    }
    /// Current exact policy and Host selection are checked on each use, never repaired on read.
    pub async fn query(
        self: &Arc<Self>,
        bearer: String,
        contract: String,
        major: u16,
        port: String,
        request: Value,
    ) -> Result<Value, BackendError> {
        if !self.active.load(Ordering::SeqCst) {
            return Err(error("unavailable"));
        }
        // A single admitted call in this phase. HTTP's bounded parallel work belongs to phase 4.
        let owner = self
            .inner
            .clone()
            .try_lock_owned()
            .map_err(|_| error("busy"))?;
        let this = self.clone();
        tokio::spawn(async move {
            if owner.storage_failed
                || !owner.config.desired_enabled
                || !owner
                    .config
                    .tokens
                    .iter()
                    .any(|t| tokens::verify(t, &bearer))
            {
                return Err(error("unauthorized"));
            }
            let observed = this.catalog.observe(&contract).await?;
            authorize(&owner.config, &observed, major, &port)?;
            if !this.active.load(Ordering::SeqCst) {
                return Err(error("unavailable"));
            }
            // The gate remains owned through SDK enqueue/response; a cancelled observer cannot
            // allow a mutation to acknowledge before this already-authorized read settles.
            this.catalog.query(&observed, &port, request).await
        })
        .await
        .map_err(|_| error("unavailable"))?
    }
    pub async fn catalog_page(
        &self,
        cursor: Option<&str>,
        contract: Option<&str>,
        offset: usize,
    ) -> Result<Value, BackendError> {
        let owner = self.inner.try_lock().map_err(|_| error("busy"))?;
        if let Some(contract) = contract {
            if cursor.is_some() || offset > 256 {
                return Err(error("bad_request"));
            }
            let observed = self.catalog.observe(contract).await?;
            let ports = &observed.document.contracts[0].ports;
            let mut tools = vec![];
            for port in ports.iter().skip(offset).take(32) {
                let fingerprint = observed.fingerprint(&port.name)?;
                let saved = owner.config.contracts.get(contract);
                let selected = saved.and_then(|c| c.tools.get(&port.name));
                let matches = saved.is_some_and(|c| {
                    c.major == observed.summary.major
                        && observed
                            .summary
                            .provider
                            .as_ref()
                            .is_some_and(|p| p.identity == c.provider)
                });
                tools.push(json!({"port":port.name,"fingerprint":fingerprint,"eligible":observed.ordinary(&port.name)&&port.name!="runtime.guide","selected":selected.is_some(),"reviewRequired":selected.is_some_and(|t|t.fingerprint!=fingerprint)||selected.is_some()&&!matches}));
            }
            return Ok(
                json!({"contracts":[],"nextCursor":null,"tools":tools,"nextOffset":(offset+32<ports.len()).then_some(offset+32)}),
            );
        }
        if offset != 0 {
            return Err(error("bad_request"));
        }
        let page = tokio::time::timeout(
            std::time::Duration::from_secs(crate::limits::CATALOG_READ_TIMEOUT as u64),
            self.catalog.page(cursor),
        )
        .await
        .map_err(|_| error("timeout"))??;
        let rows:Vec<Value>=page.contracts.iter().map(|r|json!({"contract":r.contract,"major":r.major,"provider":r.provider.as_ref().map(|p|&p.identity),"displayName":r.provider.as_ref().map(|p|&p.display_name),"available":r.available,"hostEnabled":r.enabled,"selected":owner.config.contracts.get(&r.contract).is_some_and(|c|c.enabled),"guideEligibility":r.guide_eligibility,"excluded":excluded(r)})).collect();
        Ok(json!({"contracts":rows,"nextCursor":page.next_cursor,"tools":[],"nextOffset":null}))
    }
}
fn ok() -> Value {
    json!({"ok":true,"warning":null})
}
fn exact(observed: &Observed, major: u16, provider: &str) -> Result<(), BackendError> {
    if observed.summary.major != major
        || observed
            .summary
            .provider
            .as_ref()
            .is_none_or(|p| p.identity != provider)
    {
        Err(error("catalog_changed"))
    } else {
        Ok(())
    }
}
fn authorize(
    config: &Config,
    observed: &Observed,
    major: u16,
    port: &str,
) -> Result<(), BackendError> {
    let c = config
        .contracts
        .get(&observed.summary.contract)
        .ok_or_else(|| error("forbidden"))?;
    exact(observed, c.major, &c.provider)?;
    if !c.enabled
        || c.major != major
        || !observed.summary.enabled
        || observed.summary.enabled_major != Some(major)
        || !observed.summary.available
        || !observed.ordinary(port)
    {
        return Err(error("forbidden"));
    }
    if port == "runtime.guide" {
        if !c.tools.iter().any(|(name, t)| {
            observed.ordinary(name) && observed.fingerprint(name).is_ok_and(|f| f == t.fingerprint)
        }) {
            return Err(error("forbidden"));
        }
    } else if c.tools.get(port).is_none_or(|t| {
        observed
            .fingerprint(port)
            .map_or(true, |f| f != t.fingerprint)
    }) {
        return Err(error("forbidden"));
    }
    Ok(())
}
fn status_value(inner: &Inner) -> Value {
    let tokens: Vec<Value> = inner
        .config
        .tokens
        .iter()
        .map(|t| json!({"id":t.id,"label":t.label,"generation":t.generation}))
        .collect();
    json!({"desiredEnabled":inner.config.desired_enabled,"port":inner.config.port,"running":false,"storageFailed":inner.storage_failed,"warning":inner.warning,"tokens":tokens,"selectedContracts":inner.config.contracts.len(),"selectedTools":inner.config.contracts.values().map(|c|c.tools.len()).sum::<usize>()})
}

// A failed write cannot grant a new token/tool or resurrect a requested revocation. A later
// successful user mutation commits this intersection, not a stale pre-revocation disk snapshot.
fn denial_intersection(old: &Config, next: &Config) -> Config {
    let mut safe = old.clone();
    safe.desired_enabled &= next.desired_enabled;
    safe.tokens.retain(|t| {
        next.tokens
            .iter()
            .any(|n| n.id == t.id && n.generation == t.generation && n.verifier == t.verifier)
    });
    safe.contracts.retain(|id, c| {
        let Some(n) = next.contracts.get(id) else {
            return false;
        };
        if n.major != c.major || n.provider != c.provider {
            return false;
        }
        c.enabled &= n.enabled;
        c.tools.retain(|name, t| {
            n.tools
                .get(name)
                .is_some_and(|n| n.fingerprint == t.fingerprint)
        });
        true
    });
    safe
}
