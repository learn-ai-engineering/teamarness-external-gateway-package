//! Recoverable output coordination. Policy alone admits/revokes verifier authority.
use crate::{
    client_store::{ClientStore, Connection},
    config::{ManagedAssociation, ManagedDisposition, StorageFault, WriteStage},
    error,
    managed_install::{
        self as install, Admitted, Approval, CommandPlan, Disposition, InstallFault,
        ManagedInstaller, Pin,
    },
    policy::{Policy, SetupSnapshot},
    skill_install::{SkillInstaller, SkillPlan},
    tokens::{self, TokenRecord},
};
use rustix::fs::{self as sys};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use teamarness_package_sdk::{BackendError, Initialization};
use tokio::sync::watch;
type Result<T> = std::result::Result<T, BackendError>;
const TEN_MINUTES: u64 = 600;
const MAX_JOURNAL: u64 = 65536;
fn disk<T>(r: std::result::Result<T, impl std::fmt::Debug>) -> Result<T> {
    r.map_err(|_| error("setup_storage_failed"))
}
fn output(e: install::InstallError) -> BackendError {
    BackendError::new(e.code(), false, json!({}))
}
/// External OS seams only; owned stores, Policy and installers are never substituted.
#[derive(Clone)]
pub struct BackendEnvironment {
    home: Option<PathBuf>,
    source: Option<(PathBuf, Admitted)>,
    pub storage_fault: Option<Arc<dyn StorageFault>>,
    pub client_fault: Option<Arc<dyn StorageFault>>,
    pub install_fault: Option<Arc<dyn InstallFault>>,
    pub setup_fault: Option<Arc<dyn SetupFault>>,
    pub clock: Arc<dyn Clock>,
}
impl Default for BackendEnvironment {
    fn default() -> Self {
        Self {
            home: None,
            source: None,
            storage_fault: None,
            client_fault: None,
            install_fault: None,
            setup_fault: None,
            clock: Arc::new(Monotonic(Instant::now())),
        }
    }
}
impl BackendEnvironment {
    /// Library/test driver constructor only. There is no production argument or UI path override.
    pub fn injected(home: PathBuf) -> Self {
        Self {
            home: Some(home),
            ..Self::default()
        }
    }
    pub fn with_admitted_source(mut self, source: PathBuf, admitted: Admitted) -> Self {
        self.source = Some((source, admitted));
        self
    }
    pub(crate) fn executable(
        &self,
        init: &Initialization,
    ) -> Result<(
        PathBuf,
        std::result::Result<Admitted, install::InstallError>,
    )> {
        Ok(match &self.source {
            Some((p, a)) => (p.clone(), Ok(a.clone())),
            None => (
                std::env::current_exe().map_err(|_| error("reload_required"))?,
                Admitted::from_initialization(init),
            ),
        })
    }
    async fn resolve_home(&self) -> Result<PathBuf> {
        match &self.home {
            Some(p) => Ok(p.clone()),
            None => crate::cli::home::resolve()
                .await
                .map_err(|_| error("home_unavailable")),
        }
    }
}
pub trait Clock: Send + Sync {
    fn seconds(&self) -> u64;
}
struct Monotonic(Instant);
impl Clock for Monotonic {
    fn seconds(&self) -> u64 {
        self.0.elapsed().as_secs()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Journal,
    Receipt,
    JournalCleanup,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transition {
    Prepared,
    Outputs,
    Committed,
    Published,
}
/// Faults run at real file boundaries or completed durable transitions; never receive credentials.
pub trait SetupFault: Send + Sync {
    fn before(&self, _effect: Effect, _stage: WriteStage) -> std::io::Result<()> {
        Ok(())
    }
    fn checkpoint(&self, _transition: Transition) -> std::io::Result<()> {
        Ok(())
    }
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Intent {
    Connect,
    Repair,
    Rotate,
    Disconnect,
    InstallSkill,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preview {
    pub intent: Intent,
    pub selection_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Apply {
    pub plan_id: String,
}
#[derive(Clone)]
struct Plan {
    selection_id: Option<String>,
    id: String,
    operation: String,
    expires: u64,
    intent: Intent,
    revision: String,
    connection: Option<Connection>,
    command: Option<CommandPlan>,
    skill: Option<SkillPlan>,
    selected: Option<Pin>,
    credential_change: bool,
    resume: Option<Journal>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Receipt {
    operation_id: Option<String>,
    intent: Option<Intent>,
    phase: String,
    command: String,
    skill: String,
    access: String,
    error: Option<String>,
    command_path: Option<String>,
    skill_path: Option<String>,
}
impl Default for Receipt {
    fn default() -> Self {
        Self {
            operation_id: None,
            intent: None,
            phase: "idle".into(),
            command: "unchanged".into(),
            skill: "not-requested".into(),
            access: "unchanged".into(),
            error: None,
            command_path: None,
            skill_path: None,
        }
    }
}
fn identifier(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Receipt {
    fn validate(&self) -> Result<()> {
        if ![
            "idle",
            "accepted",
            "prepared",
            "outputs",
            "committed",
            "complete",
            "repair-required",
            "conflict",
            "cleanup-pending",
        ]
        .contains(&self.phase.as_str())
            || !["unchanged", "pending", "installed"].contains(&self.command.as_str())
            || !["not-requested", "pending", "installed"].contains(&self.skill.as_str())
            || ![
                "unchanged",
                "pending",
                "configured",
                "revoked",
                "repair-required",
            ]
            .contains(&self.access.as_str())
            || self.operation_id.as_ref().is_some_and(|s| !identifier(s))
            || self.operation_id.is_some() != self.intent.is_some()
            || self.error.as_ref().is_some_and(|s| {
                s.len() > 128
                    || !s.starts_with("external_gateway.")
                    || !s
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b == b'_' || b == b'.')
            })
            || [&self.command_path, &self.skill_path].iter().any(|p| {
                p.as_ref().is_some_and(|s| {
                    !s.starts_with('/') || s.len() > 4096 || s.chars().any(char::is_control)
                })
            })
        {
            return Err(error("setup_state_invalid"));
        }
        Ok(())
    }
    fn without_selection(&self) -> Self {
        let mut receipt = self.clone();
        receipt.skill_path = None;
        receipt
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Journal {
    format_version: u32,
    plan_id: String,
    operation: String,
    intent: Intent,
    revision: String,
    policy_digest: String,
    connection: Option<Connection>,
    old_connection: Option<Connection>,
    association: Option<ManagedAssociation>,
    record: Option<TokenRecord>,
    admission: Admitted,
    selected: Option<Pin>,
    command_done: bool,
    skill_done: bool,
    policy_done: bool,
    receipt: Receipt,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Completed {
    format_version: u32,
    plan_id: String,
    receipt: Receipt,
}
impl Completed {
    fn validate(&self) -> Result<()> {
        self.receipt.validate()?;
        if self.format_version != 1
            || !identifier(&self.plan_id)
            || self.receipt.operation_id.is_none()
            || self.receipt.skill_path.is_some()
        {
            return Err(error("setup_state_invalid"));
        }
        Ok(())
    }
}
struct Memory {
    plans: Vec<Plan>,
    accepted: Option<(String, String)>,
    pending: Option<Journal>,
    selections: BTreeMap<String, Pin>,
    configured_port: Option<u16>,
    admission: bool,
    invalid: bool,
}
pub struct SetupCoordinator {
    policy: Arc<Policy>,
    home: PathBuf,
    source: PathBuf,
    admitted: std::result::Result<Admitted, install::InstallError>,
    environment: BackendEnvironment,
    workspace: String,
    name: String,
    installation: String,
    continuity: String,
    journal_name: String,
    receipt_name: String,
    memory: Mutex<Memory>,
    status: watch::Sender<Receipt>,
    stopped: AtomicBool,
    task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl SetupCoordinator {
    pub async fn open(
        policy: Arc<Policy>,
        init: &Initialization,
        environment: BackendEnvironment,
    ) -> Result<Arc<Self>> {
        let home = environment.resolve_home().await?;
        let (source, admitted) = environment.executable(init)?;
        let key = crate::catalog::digest(
            format!(
                "{}\0{}\0{}",
                init.workspace().id,
                init.installation().identity,
                init.data_directory().continuity_id()
            )
            .as_bytes(),
        );
        let (status, _) = watch::channel(Receipt::default());
        let this = Arc::new(Self {
            policy,
            home,
            source,
            admitted,
            environment,
            workspace: init.workspace().id.clone(),
            name: init.workspace().name.clone(),
            installation: init.installation().identity.clone(),
            continuity: init.data_directory().continuity_id().into(),
            journal_name: format!("{key}.operation"),
            receipt_name: format!("{key}.receipt"),
            memory: Mutex::new(Memory {
                plans: vec![],
                accepted: None,
                pending: None,
                selections: BTreeMap::new(),
                configured_port: None,
                admission: false,
                invalid: false,
            }),
            status,
            stopped: AtomicBool::new(false),
            task: Default::default(),
        });
        let runner = this.clone();
        let handle = tokio::runtime::Handle::current();
        let recovery = tokio::task::spawn_blocking(move || runner.recover(&handle))
            .await
            .map_err(|_| error("setup_storage_failed"))?;
        if let Err(e) = recovery {
            let mut m = this.memory.lock().expect("setup memory");
            m.invalid = m.pending.is_none();
            drop(m);
            this.report_error(e);
        }
        this.refresh().await;
        Ok(this)
    }
    fn installer(&self) -> Result<ManagedInstaller> {
        let mut i = ManagedInstaller::for_home(&self.home).map_err(output)?;
        if let Some(f) = &self.environment.install_fault {
            i = i.with_fault(f.clone());
        }
        Ok(i)
    }
    fn store(&self) -> Result<ClientStore> {
        let mut s = ClientStore::for_home(&self.home).map_err(|_| error("client_store_invalid"))?;
        if let Some(f) = &self.environment.client_fault {
            s = s.with_fault(f.clone());
        }
        Ok(s)
    }
    fn admitted(&self) -> Result<Admitted> {
        self.admitted.clone().map_err(output)
    }
    fn state_path(&self) -> PathBuf {
        self.home
            .join(".local/share/teamarness/external-gateway/setup")
    }
    fn state(&self, create: bool) -> Result<Option<File>> {
        let pin = Pin::capture(&self.state_path()).map_err(output)?;
        if !create && !pin.complete {
            return Ok(None);
        }
        install::open_dir(&self.state_path(), create, true)
            .map(Some)
            .map_err(output)
    }
    fn read<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        let Some(dir) = self.state(false)? else {
            return Ok(None);
        };
        let Some(bytes) = install::read_at(&dir, name, 0o600, MAX_JOURNAL).map_err(output)? else {
            return Ok(None);
        };
        let value = teamarness_package_contract::json::strict_json(&bytes)
            .map_err(|_| error("setup_state_invalid"))?;
        serde_json::from_value(value)
            .map(Some)
            .map_err(|_| error("setup_state_invalid"))
    }
    fn boundary(&self, effect: Effect, stage: WriteStage) -> Result<()> {
        if let Some(f) = &self.environment.setup_fault {
            disk(f.before(effect, stage))?;
        }
        Ok(())
    }
    fn checkpoint(&self, t: Transition) -> Result<()> {
        if let Some(f) = &self.environment.setup_fault {
            disk(f.checkpoint(t))?;
        }
        Ok(())
    }
    fn write<T: Serialize>(&self, name: &str, value: &T, effect: Effect) -> Result<()> {
        let dir = self
            .state(true)?
            .ok_or_else(|| error("setup_storage_failed"))?;
        // Recognized private temporary state is discarded, never promoted after interruption.
        let temp = format!("{name}.tmp");
        install::discard_temp(&dir, &temp, 0o600).map_err(output)?;
        let bytes = disk(serde_json::to_vec(value))?;
        if bytes.len() > MAX_JOURNAL as usize {
            return Err(error("too_large"));
        }
        let mut file = install::create(&dir, &temp).map_err(output)?;
        self.boundary(effect, WriteStage::Write)?;
        disk(file.write_all(&bytes))?;
        self.boundary(effect, WriteStage::FileSync)?;
        disk(file.sync_all())?;
        self.boundary(effect, WriteStage::Rename)?;
        disk(sys::renameat(&dir, &temp, &dir, name))?;
        self.boundary(effect, WriteStage::DirectorySync)?;
        disk(dir.sync_all())
    }
    fn remove_journal(&self) -> Result<()> {
        let Some(dir) = self.state(false)? else {
            return Ok(());
        };
        // Read validates private file shape before unlink. No pathname from journal controls deletion.
        let _: Option<Journal> = self.read(&self.journal_name)?;
        self.boundary(Effect::JournalCleanup, WriteStage::Rename)?;
        install::discard_temp(&dir, &format!("{}.tmp", self.journal_name), 0o600)
            .map_err(output)?;
        match sys::unlinkat(&dir, &self.journal_name, sys::AtFlags::empty()) {
            Ok(()) => {}
            Err(rustix::io::Errno::NOENT) => {}
            Err(_) => return Err(error("setup_storage_failed")),
        }
        self.boundary(Effect::JournalCleanup, WriteStage::DirectorySync)?;
        disk(dir.sync_all())
    }
    fn report_error(&self, e: BackendError) {
        let mut r = self
            .memory
            .lock()
            .expect("setup memory")
            .pending
            .as_ref()
            .map(|j| j.receipt.clone())
            .unwrap_or_else(|| self.status.borrow().clone());
        r.phase = if e.code() == "external_gateway.conflict" {
            "conflict"
        } else {
            "repair-required"
        }
        .into();
        if r.access == "pending" {
            r.access = "repair-required".into();
        }
        r.error = Some(e.code().into());
        self.publish(r);
    }
    fn publish(&self, r: Receipt) {
        self.status.send_replace(r);
    }
    pub(crate) fn receipt_watch(&self) -> watch::Receiver<Receipt> {
        self.status.subscribe()
    }
    pub async fn status(&self) -> Value {
        let mut r = self.status.borrow().clone();
        if r.access == "configured" {
            match self.policy.setup_snapshot().await {
                Ok(s)
                    if s.config.managed.as_ref().is_some_and(|m| {
                        Some(&m.operation_id) == r.operation_id.as_ref()
                            && m.disposition == ManagedDisposition::Current
                    }) && s.running
                        && s.config.port
                            == self.memory.lock().expect("setup memory").configured_port => {}
                _ => {
                    r.access = "repair-required".into();
                }
            }
        }
        serde_json::to_value(r).expect("receipt")
    }
    /// Phase-5 picker registers a canonical physical selection here after its cancellation race.
    /// Only an opaque ID crosses the UI boundary; this never grants overwrite authority.
    pub async fn register_selection(self: &Arc<Self>, path: PathBuf) -> Result<String> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(error("unavailable"));
        }
        let pin = tokio::task::spawn_blocking(move || {
            let p = Pin::capture(&path).map_err(output)?;
            p.open().map_err(output)?;
            Ok::<_, BackendError>(p)
        })
        .await
        .map_err(|_| error("output_unsafe"))??;
        let id = random_id()?;
        let mut m = self.memory.lock().expect("setup memory");
        if self.stopped.load(Ordering::SeqCst) {
            return Err(error("unavailable"));
        }
        if m.selections.len() >= 4 {
            return Err(error("busy"));
        }
        m.selections.insert(id.clone(), pin);
        Ok(id)
    }
    pub fn invalidate_selection(&self, id: &str) {
        let mut m = self.memory.lock().expect("setup memory");
        m.selections.remove(id);
        m.plans.retain(|p| p.selection_id.as_deref() != Some(id));
    }
    pub async fn preview(self: &Arc<Self>, request: Preview) -> Result<Value> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(error("unavailable"));
        }
        let selected = {
            let mut m = self.memory.lock().expect("setup memory");
            m.plans
                .retain(|p| p.expires > self.environment.clock.seconds());
            if m.invalid {
                return Err(error("setup_state_invalid"));
            }
            if m.admission || m.plans.len() >= 4 {
                return Err(error("busy"));
            }
            if m.pending.is_some() && request.intent != Intent::Repair {
                return Err(error("setup_repair_required"));
            }
            request
                .selection_id
                .as_ref()
                .map(|id| {
                    m.selections
                        .get(id)
                        .cloned()
                        .ok_or_else(|| error("selection_invalid"))
                })
                .transpose()?
        };
        if request.intent == Intent::InstallSkill && selected.is_none() {
            return Err(error("selection_required"));
        }
        if request.intent == Intent::Disconnect && selected.is_some() {
            return Err(error("bad_request"));
        }
        let snapshot = self.policy.setup_snapshot().await?;
        let this = self.clone();
        let mut plan =
            tokio::task::spawn_blocking(move || this.make_plan(request.intent, selected, snapshot))
                .await
                .map_err(|_| error("setup_storage_failed"))??;
        let result = json!({"planId":plan.id,"intent":plan.intent,"commandPath":self.home.join(".local/bin/tmx"),"skillPath":plan.skill.as_ref().map(|s|s.destination()),"credentialChange":plan.credential_change,"commandChange":plan.command.as_ref().map(|c|c.summary().map_err(output)).transpose()?,"skillChange":plan.skill.as_ref().map(|s|if s.unchanged(){"unchanged"}else{"publish"}).unwrap_or("not-requested")});
        let mut m = self.memory.lock().expect("setup memory");
        if m.admission || m.plans.len() >= 4 {
            return Err(error("busy"));
        }
        if self.stopped.load(Ordering::SeqCst)
            || request
                .selection_id
                .as_ref()
                .is_some_and(|id| !m.selections.contains_key(id))
        {
            return Err(error("selection_invalid"));
        }
        plan.selection_id = request.selection_id;
        m.plans.push(plan);
        Ok(result)
    }
    fn make_plan(
        &self,
        intent: Intent,
        selected: Option<Pin>,
        snapshot: SetupSnapshot,
    ) -> Result<Plan> {
        let pending = self.memory.lock().expect("setup memory").pending.clone();
        let disconnect = intent == Intent::Disconnect
            || pending
                .as_ref()
                .is_some_and(|j| j.intent == Intent::Disconnect);
        if !disconnect
            && intent != Intent::InstallSkill
            && !pending
                .as_ref()
                .is_some_and(|j| j.intent == Intent::InstallSkill)
            && (!snapshot.running || snapshot.config.port.is_none())
        {
            return Err(error("listener_required"));
        }
        if let Some(p) = &selected {
            p.check().map_err(output)?;
        }
        let installer = self.installer()?;
        let admission = self.admitted()?;
        let command = if disconnect {
            None
        } else {
            Some(
                installer
                    .preview(&self.source, admission.clone())
                    .map_err(output)?,
            )
        };
        let connections = self
            .store()?
            .connections()
            .map_err(|_| error("client_store_invalid"))?;
        let connection = connections
            .into_iter()
            .find(|c| c.workspace_id == self.workspace);
        if connection.as_ref().is_some_and(|c| {
            c.installation_id != self.installation
                || c.continuity_id != self.continuity
                || snapshot
                    .config
                    .managed
                    .as_ref()
                    .is_none_or(|m| m.origin_nonce != c.origin_nonce)
        }) {
            return Err(error("connection_origin_conflict"));
        }
        let managed = snapshot.config.managed.as_ref();
        let current = managed.is_some_and(|m| m.disposition == ManagedDisposition::Current);
        if pending.is_some() && selected.is_some() {
            return Err(error("setup_repair_required"));
        }
        if matches!(intent, Intent::Rotate) && !current {
            return Err(error("managed_setup_required"));
        }
        if intent == Intent::Repair && pending.is_none() && !current {
            return Err(error("credential_consent_required"));
        }
        if intent == Intent::Connect && current {
            return Err(error("managed_setup_required"));
        }
        if intent == Intent::Disconnect && managed.is_none() {
            return Err(error("not_found"));
        }
        let credential_change = matches!(intent, Intent::Connect | Intent::Rotate)
            || pending.as_ref().is_some_and(|j| j.record.is_some());
        if credential_change
            && managed.is_none()
            && snapshot.config.tokens.len() >= crate::config::MAX_TOKENS
        {
            return Err(error("too_large"));
        }
        if intent == Intent::Repair
            && pending.is_none()
            && !connection.as_ref().is_some_and(|c| {
                managed.is_some_and(|m| m.token_id == c.token_id && m.generation == c.generation)
                    && snapshot
                        .config
                        .tokens
                        .iter()
                        .any(|t| tokens::verify(t, &c.bearer))
            })
        {
            return Err(error("credential_consent_required"));
        }
        let selected = pending
            .as_ref()
            .and_then(|j| j.selected.clone())
            .or(selected);
        let skill = selected
            .as_ref()
            .map(|p| {
                SkillInstaller::new(&installer)
                    .preview(&p.path, admission.clone())
                    .map_err(output)
            })
            .transpose()?;
        let id = random_id()?;
        let operation = pending
            .as_ref()
            .map(|j| j.operation.clone())
            .unwrap_or_else(|| id.clone());
        Ok(Plan {
            selection_id: None,
            id,
            operation,
            expires: self.environment.clock.seconds() + TEN_MINUTES,
            intent,
            revision: snapshot.revision,
            connection,
            command,
            skill,
            selected,
            credential_change,
            resume: pending,
        })
    }
    pub async fn apply(self: &Arc<Self>, request: Apply) -> Result<Value> {
        let plan = {
            let mut m = self.memory.lock().expect("setup memory");
            if let Some((id, op)) = &m.accepted
                && id == &request.plan_id
            {
                return Ok(json!({"operationId":op}));
            }
            if self.stopped.load(Ordering::SeqCst) {
                return Err(error("unavailable"));
            }
            if m.admission {
                return Err(error("busy"));
            }
            let p = m
                .plans
                .iter()
                .find(|p| p.id == request.plan_id && p.expires > self.environment.clock.seconds())
                .cloned()
                .ok_or_else(|| error("setup_preview_stale"))?;
            m.admission = true;
            p
        };
        // Reserve one admission before the first await. Cancellation cannot leave an orphaned reservation.
        let this = self.clone();
        let (send, recv) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result = this.accept_and_run(plan, send).await;
            if let Err(e) = result {
                let runner = this.clone();
                let rt = tokio::runtime::Handle::current();
                let recovered = tokio::task::spawn_blocking(move || runner.recover(&rt)).await;
                if !matches!(recovered, Ok(Ok(()))) || this.status.borrow().phase != "complete" {
                    this.report_error(e);
                }
            }
            this.memory.lock().expect("setup memory").admission = false;
        });
        *self.task.lock().expect("setup task") = Some(task);
        recv.await.map_err(|_| error("setup_storage_failed"))?
    }
    async fn accept_and_run(
        self: &Arc<Self>,
        plan: Plan,
        send: tokio::sync::oneshot::Sender<Result<Value>>,
    ) -> Result<()> {
        let this = self.clone();
        let checked = plan.clone();
        let snap = match self.policy.setup_snapshot().await {
            Ok(s) => s,
            Err(e) => {
                let _ = send.send(Err(e));
                return Ok(());
            }
        };
        let check = tokio::task::spawn_blocking(move || {
            if snap.revision != checked.revision
                || (checked.intent != Intent::Disconnect
                    && checked.intent != Intent::InstallSkill
                    && !checked.resume.as_ref().is_some_and(|j| {
                        matches!(j.intent, Intent::Disconnect | Intent::InstallSkill)
                    })
                    && !snap.running)
            {
                return Err(error("setup_preview_stale"));
            }
            let i = this.installer()?;
            if let Some(c) = &checked.command {
                c.recheck(&i).map_err(output)?;
            }
            if let Some(s) = &checked.skill {
                s.recheck(&i, this.admitted()?).map_err(output)?;
            }
            if this
                .store()?
                .connections()
                .map_err(|_| error("client_store_invalid"))?
                .into_iter()
                .find(|c| c.workspace_id == this.workspace)
                != checked.connection
            {
                return Err(error("setup_preview_stale"));
            }
            Ok(())
        })
        .await
        .map_err(|_| error("setup_storage_failed"))?;
        if let Err(e) = check {
            let _ = send.send(Err(e));
            return Ok(());
        }
        {
            let mut m = self.memory.lock().expect("setup memory");
            if self.stopped.load(Ordering::SeqCst)
                || plan
                    .selection_id
                    .as_ref()
                    .is_some_and(|id| !m.selections.contains_key(id))
            {
                let _ = send.send(Err(error("setup_preview_stale")));
                return Ok(());
            }
            m.accepted = Some((plan.id.clone(), plan.operation.clone()));
            m.plans.clear();
        }
        let receipt = Receipt {
            operation_id: Some(plan.operation.clone()),
            intent: Some(plan.intent),
            phase: "accepted".into(),
            command: if plan.intent == Intent::Disconnect {
                "unchanged"
            } else {
                "pending"
            }
            .into(),
            skill: if plan.skill.is_some() {
                "pending"
            } else {
                "not-requested"
            }
            .into(),
            access: if plan.intent == Intent::InstallSkill {
                "unchanged"
            } else {
                "pending"
            }
            .into(),
            command_path: Some(self.home.join(".local/bin/tmx").to_string_lossy().into()),
            skill_path: plan
                .skill
                .as_ref()
                .map(|s| s.destination().to_string_lossy().into()),
            error: None,
        };
        self.publish(receipt.clone());
        let _ = send.send(Ok(json!({"operationId":plan.operation})));
        let this = self.clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || this.run(plan, receipt, &handle))
            .await
            .map_err(|_| error("setup_storage_failed"))?
    }
    fn run(&self, plan: Plan, receipt: Receipt, rt: &tokio::runtime::Handle) -> Result<()> {
        let installer = self.installer()?;
        if let Some(c) = &plan.command {
            c.recheck(&installer).map_err(output)?;
        }
        if let Some(s) = &plan.skill {
            s.recheck(&installer, self.admitted()?).map_err(output)?;
        }
        let store = self.store()?;
        let mutation = disk(store.lock())?;
        if mutation
            .connections()
            .map_err(|_| error("client_store_invalid"))?
            .into_iter()
            .find(|c| c.workspace_id == self.workspace)
            != plan.connection
        {
            return Err(error("setup_preview_stale"));
        }
        let disk_pending = self.read::<Journal>(&self.journal_name)?;
        if let Some(j) = &disk_pending {
            self.validate_journal(j)?;
        }
        if serde_json::to_vec(&disk_pending).map_err(|_| error("setup_state_invalid"))?
            != serde_json::to_vec(&plan.resume).map_err(|_| error("setup_state_invalid"))?
        {
            return Err(error("setup_preview_stale"));
        }
        if let Some(c) = self.read::<Completed>(&self.receipt_name)? {
            c.validate()?;
        }
        let snapshot = rt.block_on(self.policy.setup_snapshot())?;
        if snapshot.revision != plan.revision {
            return Err(error("setup_preview_stale"));
        }
        let mut j = if let Some(j) = plan.resume.clone() {
            self.validate_journal(&j)?;
            if j.admission != self.admitted()? {
                return Err(error("reload_required"));
            }
            if j.policy_digest != snapshot.durable_revision && !self.current(&j, rt) {
                return Err(error("conflict"));
            }
            j
        } else {
            let origin = rt.block_on(self.policy.setup_origin())?;
            let old = snapshot.config.managed.as_ref();
            if old.is_some_and(|m| m.origin_nonce != origin) {
                return Err(error("connection_origin_conflict"));
            }
            let issued = if plan.credential_change {
                let generation = old
                    .map(|m| {
                        m.generation
                            .checked_add(1)
                            .ok_or_else(|| error("too_large"))
                    })
                    .transpose()?
                    .unwrap_or(1);
                Some(
                    tokens::issue(
                        "Managed CLI",
                        generation,
                        old.map(|m| m.token_id.as_str()),
                        &tokens::OsEntropy,
                    )
                    .map_err(|_| error("unavailable"))?,
                )
            } else {
                None
            };
            let record = issued.as_ref().map(|(r, _)| r.clone()).or_else(|| {
                (plan.intent == Intent::Repair)
                    .then(|| {
                        snapshot
                            .config
                            .tokens
                            .iter()
                            .find(|r| {
                                old.is_some_and(|m| {
                                    m.token_id == r.id && m.generation == r.generation
                                })
                            })
                            .cloned()
                    })
                    .flatten()
            });
            let association = if let Some(r) = &record {
                Some(ManagedAssociation {
                    token_id: r.id.clone(),
                    generation: r.generation,
                    operation_id: plan.operation.clone(),
                    origin_nonce: origin.clone(),
                    disposition: ManagedDisposition::Current,
                })
            } else if plan.intent == Intent::Disconnect {
                old.map(|m| ManagedAssociation {
                    operation_id: plan.operation.clone(),
                    disposition: ManagedDisposition::Disconnected,
                    ..m.clone()
                })
            } else {
                old.cloned()
            };
            let connection = if let Some((r, bearer)) = issued {
                Some(Connection {
                    format_version: 1,
                    workspace_id: self.workspace.clone(),
                    workspace_name: self.name.clone(),
                    port: snapshot
                        .config
                        .port
                        .ok_or_else(|| error("listener_required"))?,
                    installation_id: self.installation.clone(),
                    continuity_id: self.continuity.clone(),
                    origin_nonce: origin,
                    token_id: r.id,
                    generation: r.generation,
                    bearer,
                    operation_id: plan.operation.clone(),
                })
            } else {
                plan.connection.clone().map(|mut c| {
                    if let Some(port) = snapshot.config.port {
                        c.port = port;
                    }
                    c.workspace_name = self.name.clone();
                    if plan.intent == Intent::Repair {
                        c.operation_id = plan.operation.clone();
                    }
                    c
                })
            };
            Journal {
                format_version: 1,
                plan_id: plan.id.clone(),
                operation: plan.operation.clone(),
                intent: plan.intent,
                revision: plan.revision.clone(),
                policy_digest: snapshot.durable_revision.clone(),
                connection,
                old_connection: plan.connection.clone(),
                association,
                record,
                admission: self.admitted()?,
                selected: plan.selected.clone(),
                command_done: false,
                skill_done: false,
                policy_done: false,
                receipt,
            }
        };
        j.plan_id = plan.id.clone();
        j.revision = plan.revision.clone();
        j.receipt.phase = "prepared".into();
        self.save_journal(&j)?;
        self.checkpoint(Transition::Prepared)?;
        if j.intent != Intent::Disconnect {
            let command = plan
                .command
                .as_ref()
                .ok_or_else(|| error("conflict"))?
                .after_preparation(&installer)
                .map_err(output)?;
            installer
                .apply(&command, Approval::Explicit)
                .map_err(output)?;
            j.command_done = true;
            j.receipt.command = "installed".into();
            self.save_journal(&j)?;
            if let Some(s) = &plan.skill {
                let s = s
                    .after_preparation(&installer, self.admitted()?)
                    .map_err(output)?;
                SkillInstaller::new(&installer).apply(&s).map_err(output)?;
                j.skill_done = true;
                j.receipt.skill = "installed".into();
                self.save_journal(&j)?;
            }
        }
        if j.intent != Intent::Disconnect {
            self.check_outputs(&j, &installer)?;
        }
        j.receipt.phase = "outputs".into();
        self.save_journal(&j)?;
        self.checkpoint(Transition::Outputs)?;
        if j.intent != Intent::Disconnect {
            self.check_outputs(&j, &installer)?;
        }
        if j.intent != Intent::InstallSkill
            && (j.record.is_some() || j.intent == Intent::Disconnect)
        {
            let association = j.association.clone().ok_or_else(|| error("conflict"))?;
            rt.block_on(self.policy.commit_managed(
                j.revision.clone(),
                association,
                j.record.clone(),
            ))?;
        }
        j.policy_done = true;
        j.receipt.phase = "committed".into();
        self.save_journal(&j)?;
        self.checkpoint(Transition::Committed)?;
        self.finish_publication(&mut j, &mutation, rt)
    }
    fn save_journal(&self, j: &Journal) -> Result<()> {
        self.write(&self.journal_name, j, Effect::Journal)?;
        self.memory.lock().expect("setup memory").pending = Some(j.clone());
        self.publish(j.receipt.clone());
        Ok(())
    }
    fn current(&self, j: &Journal, rt: &tokio::runtime::Handle) -> bool {
        match (&j.association, &j.record) {
            (Some(a), Some(r)) => rt.block_on(self.policy.managed_current(a, r)),
            (Some(a), None) => rt
                .block_on(self.policy.setup_snapshot())
                .is_ok_and(|s| s.config.managed.as_ref() == Some(a)),
            _ => j.intent == Intent::InstallSkill,
        }
    }
    fn finish_publication(
        &self,
        j: &mut Journal,
        mutation: &crate::client_store::ClientMutation<'_>,
        rt: &tokio::runtime::Handle,
    ) -> Result<()> {
        if j.intent == Intent::Disconnect {
            if !self.current(j, rt) {
                return Err(error("conflict"));
            }
            j.receipt.access = "revoked".into();
            if let Some(c) = &j.old_connection
                && mutation.remove_matching(c).is_err()
            {
                j.receipt.phase = "cleanup-pending".into();
                j.receipt.error = Some("external_gateway.local_cleanup_pending".into());
                self.save_journal(j)?;
                return Ok(());
            }
        } else if j.intent != Intent::InstallSkill {
            if !self.current(j, rt) {
                return Err(error("conflict"));
            }
            let c = j.connection.as_ref().ok_or_else(|| error("conflict"))?;
            disk(mutation.publish(c, None))?;
            self.checkpoint(Transition::Published)?;
            if !self.current(j, rt) {
                return Err(error("conflict"));
            }
            j.receipt.access = "configured".into();
        }
        j.receipt.phase = "complete".into();
        j.receipt.error = None;
        self.write(
            &self.receipt_name,
            &Completed {
                format_version: 1,
                plan_id: j.plan_id.clone(),
                receipt: j.receipt.without_selection(),
            },
            Effect::Receipt,
        )?;
        self.remove_journal()?;
        {
            let mut m = self.memory.lock().expect("setup memory");
            m.pending = None;
            m.configured_port = j.connection.as_ref().map(|c| c.port);
        }
        self.publish(j.receipt.clone());
        Ok(())
    }
    fn validate_journal(&self, j: &Journal) -> Result<()> {
        j.receipt.validate()?;
        if j.format_version != 1
            || !identifier(&j.operation)
            || !identifier(&j.plan_id)
            || !identifier(&j.revision)
            || !identifier(&j.policy_digest)
            || j.admission.installation_identity() != self.installation
            || j.receipt.command_path.as_deref() != self.home.join(".local/bin/tmx").to_str()
            || j.receipt.skill_path.as_deref()
                != j.selected
                    .as_ref()
                    .and_then(|p| p.path.join("teamarness").to_str().map(str::to_owned))
                    .as_deref()
            || j.receipt.operation_id.as_ref() != Some(&j.operation)
            || j.receipt.intent != Some(j.intent)
        {
            return Err(error("setup_state_invalid"));
        }
        if matches!(j.intent, Intent::Connect | Intent::Rotate | Intent::Repair)
            && (j.record.is_none() || j.connection.is_none() || j.association.is_none())
            || (j.intent == Intent::Disconnect
                && (j.record.is_some()
                    || j.association.as_ref().is_none_or(|m| {
                        m.disposition != ManagedDisposition::Disconnected
                            || m.operation_id != j.operation
                    })))
            || (j.intent == Intent::InstallSkill && (j.record.is_some() || j.selected.is_none()))
        {
            return Err(error("setup_state_invalid"));
        }
        if let Some(old) = &j.old_connection {
            old.validate().map_err(|_| error("setup_state_invalid"))?;
            if old.workspace_id != self.workspace
                || old.installation_id != self.installation
                || old.continuity_id != self.continuity
                || j.association.as_ref().is_none_or(|m| {
                    m.origin_nonce != old.origin_nonce
                        || m.token_id != old.token_id
                        || (m.generation != old.generation
                            && old.generation.checked_add(1) != Some(m.generation))
                })
            {
                return Err(error("connection_origin_conflict"));
            }
        }
        if let Some(c) = &j.connection {
            c.validate().map_err(|_| error("setup_state_invalid"))?;
            if j.association.as_ref().is_none_or(|m| {
                m.token_id != c.token_id
                    || m.generation != c.generation
                    || m.origin_nonce != c.origin_nonce
            }) {
                return Err(error("setup_state_invalid"));
            }
            if c.workspace_id != self.workspace
                || c.installation_id != self.installation
                || c.continuity_id != self.continuity
            {
                return Err(error("connection_origin_conflict"));
            }
            if let Some(r) = &j.record
                && (j.association.as_ref().is_none_or(|m| {
                    m.disposition != ManagedDisposition::Current || m.operation_id != j.operation
                }) || !tokens::verify(r, &c.bearer)
                    || c.generation != r.generation
                    || c.token_id != r.id
                    || c.operation_id != j.operation)
            {
                return Err(error("setup_state_invalid"));
            }
        }
        j.admission.validate().map_err(output)?;
        Ok(())
    }
    fn recover(&self, rt: &tokio::runtime::Handle) -> Result<()> {
        // Opening an unused home is read-only. No ordinary user's paths are ever used by fixtures.
        self.store()?;
        let journal = self.read::<Journal>(&self.journal_name)?;
        if let Some(mut j) = journal {
            self.validate_journal(&j)?;
            if let Some(a) = &j.association
                && rt.block_on(self.policy.read_setup_origin())?.as_ref() != Some(&a.origin_nonce)
            {
                return Err(error("connection_origin_conflict"));
            }
            {
                let mut m = self.memory.lock().expect("setup memory");
                m.accepted = Some((j.plan_id.clone(), j.operation.clone()));
                m.pending = Some(j.clone());
            }
            let store = self.store()?;
            let mutation = disk(store.lock())?;
            let installer = self.installer()?;
            if j.intent != Intent::Disconnect {
                installer.recover_command().map_err(output)?;
                if j.selected.is_some() {
                    SkillInstaller::new(&installer).recover().map_err(output)?;
                }
                if installer
                    .preview(&self.source, j.admission.clone())
                    .is_ok_and(|p| p.disposition() == Disposition::Unchanged)
                {
                    j.command_done = true;
                    j.receipt.command = "installed".into();
                }
                if let Some(selected) = &j.selected
                    && SkillInstaller::new(&installer)
                        .preview(&selected.path, j.admission.clone())
                        .is_ok_and(|p| p.unchanged())
                {
                    j.skill_done = true;
                    j.receipt.skill = "installed".into();
                }
            }
            if self.current(&j, rt) {
                if j.intent != Intent::Disconnect {
                    self.check_outputs(&j, &installer)?;
                }
                self.finish_publication(&mut j, &mutation, rt)?;
            } else {
                let s = rt.block_on(self.policy.setup_snapshot())?;
                if s.durable_revision != j.policy_digest {
                    j.receipt.phase = "conflict".into();
                    j.receipt.access = "repair-required".into();
                    j.receipt.error = Some("external_gateway.setup_superseded".into());
                    self.write(
                        &self.receipt_name,
                        &Completed {
                            format_version: 1,
                            plan_id: j.plan_id.clone(),
                            receipt: j.receipt.without_selection(),
                        },
                        Effect::Receipt,
                    )?;
                    self.remove_journal()?;
                    self.memory.lock().expect("setup memory").pending = None;
                } else {
                    j.receipt.phase = "repair-required".into();
                    j.receipt.error =
                        Some("external_gateway.setup_explicit_repair_required".into());
                    self.save_journal(&j)?;
                }
                self.publish(j.receipt);
            }
        } else if let Some(c) = self.read::<Completed>(&self.receipt_name)? {
            c.validate()?;
            let port = self
                .store()?
                .connections()
                .map_err(|_| error("client_store_invalid"))?
                .iter()
                .find(|r| {
                    r.workspace_id == self.workspace
                        && Some(&r.operation_id) == c.receipt.operation_id.as_ref()
                })
                .map(|r| r.port);
            {
                let mut m = self.memory.lock().expect("setup memory");
                m.accepted = Some((
                    c.plan_id,
                    c.receipt.operation_id.clone().expect("checked operation"),
                ));
                m.configured_port = port;
                m.pending = None;
            }
            self.publish(c.receipt);
        } else {
            self.memory.lock().expect("setup memory").pending = None;
        }
        Ok(())
    }
    fn check_outputs(&self, j: &Journal, installer: &ManagedInstaller) -> Result<()> {
        // Receipt recognition plus exact proposed bytes; merely existing output is not enough.
        let p = installer
            .preview(&installer.command_path(), j.admission.clone())
            .map_err(output)?;
        if p.disposition() != Disposition::Unchanged {
            return Err(error("output_repair_required"));
        }
        if let Some(selected) = &j.selected {
            selected.check().map_err(output)?;
            if !SkillInstaller::new(installer)
                .preview(&selected.path, j.admission.clone())
                .map_err(output)?
                .unchanged()
            {
                return Err(error("output_repair_required"));
            }
        }
        Ok(())
    }
    /// Initial consent is required: absent output is never installed by activation/reload.
    async fn refresh(self: &Arc<Self>) {
        if self.memory.lock().expect("setup memory").pending.is_some() {
            return;
        }
        let this = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            let installer = this.installer()?;
            let p = installer
                .preview(&this.source, this.admitted()?)
                .map_err(output)?;
            if p.disposition() != Disposition::Install {
                installer.apply(&p, Approval::Automatic).map_err(output)?;
            }
            Ok::<_, BackendError>(())
        })
        .await;
        if let Ok(Err(e)) = result {
            let mut r = self.status.borrow().clone();
            r.error = Some(e.code().into());
            self.publish(r);
        }
    }
    pub fn close(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let mut m = self.memory.lock().expect("setup memory");
        m.selections.clear();
        m.plans.clear();
    }
    pub async fn stop(&self) {
        self.close();
        let task = self.task.lock().expect("setup task").take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
    pub fn resume(&self) {
        self.stopped.store(false, Ordering::SeqCst);
    }
}
fn random_id() -> Result<String> {
    let mut b = [0; 32];
    getrandom::fill(&mut b).map_err(|_| error("unavailable"))?;
    Ok(install::hex(&b))
}
