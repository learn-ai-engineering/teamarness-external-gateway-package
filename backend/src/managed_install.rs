//! Recognized output publication, not credential authorization. Call from owned blocking work.
//! No output is adopted without its receipt or an exact old/new publication journal.
use rustix::fs::{self as sys, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const DIR: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
const MAX_BINARY: u64 = 512 * 1024 * 1024;
pub(crate) const MAX_STATE: usize = 16384;
pub(crate) const SKILL_RECEIPT: &str = ".teamarness-receipt.json";
const COMMAND_RECEIPT: &str = "command.receipt";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    Unsafe,
    Collision,
    InvalidState,
    Busy,
    StalePreview,
    ConsentRequired,
    ReloadRequired,
    RepairRequired,
    Io,
}
impl InstallError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Unsafe => "external_gateway.output_unsafe",
            Self::Collision => "external_gateway.output_collision",
            Self::InvalidState => "external_gateway.output_state_invalid",
            Self::Busy => "external_gateway.installer_busy",
            Self::StalePreview => "external_gateway.setup_preview_stale",
            Self::ConsentRequired => "external_gateway.output_consent_required",
            Self::ReloadRequired => "external_gateway.reload_required",
            Self::RepairRequired => "external_gateway.output_repair_required",
            Self::Io => "external_gateway.output_storage_failed",
        }
    }
}
pub(crate) type Result<T> = std::result::Result<T, InstallError>;
pub(crate) fn io<T>(r: std::result::Result<T, impl std::fmt::Debug>) -> Result<T> {
    r.map_err(|_| InstallError::Io)
}
pub(crate) fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn valid_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn text(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)
}
/// Values must come from this running generation's Host Initialization, never an on-disk manifest.
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Admitted {
    owner: String,
    version: String,
    digest: String,
}
impl Admitted {
    pub(crate) fn installation_identity(&self) -> &str {
        &self.owner
    }
    pub fn from_initialization(init: &teamarness_package_sdk::Initialization) -> Result<Self> {
        if init.package().identity != "dev.teamarness.external-gateway"
            || init.package().version != crate::PACKAGE_VERSION
        {
            return Err(InstallError::ReloadRequired);
        }
        Self::new(
            &init.installation().identity,
            &init.package().version,
            &init.package().backend_digest,
        )
    }
    /// Also the injected-provenance seam for artifact fixtures; no filesystem manifest lookup.
    pub fn new(owner: &str, version: &str, digest: &str) -> Result<Self> {
        let value = Self {
            owner: owner.into(),
            version: version.into(),
            digest: digest.strip_prefix("sha256:").unwrap_or(digest).into(),
        };
        value.validate()?;
        Ok(value)
    }
    /// Validate a complete executable candidate before any private helper spawn.
    /// The caller must retain ownership/revalidate immediately before spawning it.
    pub fn verify_source(&self, path: &Path) -> Result<File> {
        self.validate()?;
        source_file(path, self)
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if !text(&self.owner)
            || self.version.len() > 128
            || semver::Version::parse(&self.version).is_err()
            || !valid_digest(&self.digest)
        {
            return Err(InstallError::InvalidState);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Install,
    Unchanged,
    Refresh,
    ExplicitRepair,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approval {
    Automatic,
    Explicit,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recovery {
    NothingPending,
    Completed,
    RepairRequired,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boundary {
    JournalWrite,
    JournalSync,
    JournalRename,
    JournalDirectorySync,
    CopyWrite,
    CopiedVerify,
    FileSync,
    OutputRename,
    OutputDirectorySync,
    ReceiptWrite,
    ReceiptSync,
    ReceiptRename,
    ReceiptDirectorySync,
    JournalRemove,
    CompletionSync,
}
/// OS fault adapter. It receives no secrets and runs only in owned blocking work.
pub trait InstallFault: Send + Sync {
    fn before(&self, boundary: Boundary) -> std::io::Result<()>;
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Receipt {
    format_version: u32,
    package: String,
    admission: Admitted,
    installed_digest: String,
    cli_major: u16,
    store_major: u16,
    kind: Kind,
}
#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Command,
    Skill,
}
impl Receipt {
    pub(crate) fn new(admission: Admitted, installed_digest: String, kind: Kind) -> Self {
        Self {
            format_version: 1,
            package: "dev.teamarness.external-gateway".into(),
            admission,
            installed_digest,
            cli_major: 1,
            store_major: 1,
            kind,
        }
    }
    pub(crate) fn validate(&self, kind: Kind) -> Result<()> {
        self.admission.validate()?;
        if self.format_version != 1
            || self.package != "dev.teamarness.external-gateway"
            || !valid_digest(&self.installed_digest)
            || self.cli_major != 1
            || self.store_major != 1
            || self.kind != kind
            || (kind == Kind::Command && self.installed_digest != self.admission.digest)
        {
            return Err(InstallError::InvalidState);
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    dev: u64,
    ino: u64,
}
fn identity(f: &File) -> Result<Identity> {
    let s = io(sys::fstat(f))?;
    // Stat field widths are per target: st_dev is i32 on macOS and u64 on Linux. The
    // cast from its declared platform alias keeps the persisted macOS identity value
    // unchanged and is a no-op on Linux; `From` does not exist for both widths.
    Ok(Identity {
        dev: s.st_dev as u64,
        ino: s.st_ino,
    })
}
pub(crate) fn directory_safe(f: &File, final_dir: bool, private: bool) -> Result<()> {
    let s = io(sys::fstat(f))?;
    let uid = rustix::process::geteuid().as_raw();
    if FileType::from_raw_mode(s.st_mode) != FileType::Directory
        || (s.st_uid != uid && (final_dir || s.st_uid != 0))
        || (s.st_mode & 0o022 != 0 && !(s.st_uid == 0 && !final_dir && s.st_mode & 0o1000 != 0))
        || (private && s.st_mode & 0o7777 != 0o700)
    {
        return Err(InstallError::Unsafe);
    }
    Ok(())
}
fn file_safe(f: &File, mode: u32) -> Result<()> {
    let s = io(sys::fstat(f))?;
    if s.st_uid != rustix::process::geteuid().as_raw()
        || FileType::from_raw_mode(s.st_mode) != FileType::RegularFile
        || s.st_nlink != 1
        // st_mode is u16 on macOS and u32 on Linux; widening from its platform alias
        // is lossless on both, while `u32::from` is a same-type conversion on Linux.
        || (s.st_mode as u32) & 0o7777 != mode
    {
        return Err(InstallError::Unsafe);
    }
    Ok(())
}
#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pin {
    pub(crate) path: PathBuf,
    pub(crate) nodes: Vec<Identity>,
    pub(crate) complete: bool,
}
impl Pin {
    pub(crate) fn capture(path: &Path) -> Result<Self> {
        let raw = path.to_str().ok_or(InstallError::Unsafe)?;
        if !path.is_absolute()
            || path.components().collect::<PathBuf>().to_str() != Some(raw)
            || raw.len() > 4096
            || raw.chars().any(char::is_control)
            || path
                .components()
                .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
            || raw.split('/').any(|p| p == "." || p == "..")
        {
            return Err(InstallError::Unsafe);
        }
        let mut dir: File = io(sys::open("/", DIR, Mode::empty()))?.into();
        let mut nodes = vec![identity(&dir)?];
        let parts: Vec<_> = path
            .components()
            .filter_map(|c| {
                if let Component::Normal(n) = c {
                    Some(n)
                } else {
                    None
                }
            })
            .collect();
        for (i, name) in parts.iter().enumerate() {
            directory_safe(&dir, false, false)?;
            dir = match sys::openat(&dir, *name, DIR, Mode::empty()) {
                Ok(fd) => fd.into(),
                Err(rustix::io::Errno::NOENT) => {
                    return Ok(Self {
                        path: path.into(),
                        nodes,
                        complete: false,
                    });
                }
                Err(_) => return Err(InstallError::Unsafe),
            };
            directory_safe(&dir, i + 1 == parts.len(), false)?;
            nodes.push(identity(&dir)?);
        }
        Ok(Self {
            path: path.into(),
            nodes,
            complete: true,
        })
    }
    pub(crate) fn check(&self) -> Result<()> {
        if &Self::capture(&self.path)? != self {
            return Err(InstallError::StalePreview);
        }
        Ok(())
    }
    pub(crate) fn existing_check(&self) -> Result<()> {
        let now = Self::capture(&self.path)?;
        if !now.nodes.starts_with(&self.nodes) {
            return Err(InstallError::StalePreview);
        }
        Ok(())
    }
    pub(crate) fn open(&self) -> Result<File> {
        self.check()?;
        if !self.complete {
            return Err(InstallError::Unsafe);
        }
        open_dir(&self.path, false, false)
    }
}
pub(crate) fn open_dir(path: &Path, create: bool, private: bool) -> Result<File> {
    // Capture validates the complete lexical path and all existing ancestors first.
    Pin::capture(path)?;
    let mut dir: File = io(sys::open("/", DIR, Mode::empty()))?.into();
    for c in path.components() {
        if let Component::Normal(name) = c {
            if create {
                match sys::mkdirat(&dir, name, Mode::RWXU) {
                    Ok(()) => io(dir.sync_all())?,
                    Err(rustix::io::Errno::EXIST) => {}
                    Err(_) => return Err(InstallError::Io),
                }
            }
            dir = io(sys::openat(&dir, name, DIR, Mode::empty()))?.into();
            directory_safe(&dir, false, false)?;
        }
    }
    directory_safe(&dir, true, private)?;
    Ok(dir)
}
pub(crate) fn read_at(dir: &File, name: &str, mode: u32, max: u64) -> Result<Option<Vec<u8>>> {
    let f: File = match sys::openat(dir, name, READ, Mode::empty()) {
        Ok(fd) => fd.into(),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(_) => return Err(InstallError::Unsafe),
    };
    file_safe(&f, mode)?;
    let mut b = vec![];
    io(f.take(max + 1).read_to_end(&mut b))?;
    if b.len() as u64 > max {
        return Err(InstallError::InvalidState);
    }
    Ok(Some(b))
}
pub(crate) fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    if bytes.len() > MAX_STATE {
        return Err(InstallError::InvalidState);
    }
    let v = teamarness_package_contract::json::strict_json(bytes)
        .map_err(|_| InstallError::InvalidState)?;
    serde_json::from_value(v).map_err(|_| InstallError::InvalidState)
}
pub(crate) fn encode<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    let b = io(serde_json::to_vec(v))?;
    if b.len() > MAX_STATE {
        return Err(InstallError::InvalidState);
    }
    Ok(b)
}
pub(crate) fn hash_file(f: &mut File) -> Result<String> {
    io(f.seek(SeekFrom::Start(0)))?;
    let mut h = Sha256::new();
    let mut total = 0;
    let mut buf = [0u8; 65536];
    loop {
        let n = io(f.read(&mut buf))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > MAX_BINARY {
            return Err(InstallError::InvalidState);
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}
fn source_file(path: &Path, admission: &Admitted) -> Result<File> {
    let parent = Pin::capture(path.parent().ok_or(InstallError::Unsafe)?)?.open()?;
    let mut f: File = io(sys::openat(
        &parent,
        path.file_name().ok_or(InstallError::Unsafe)?,
        READ,
        Mode::empty(),
    ))?
    .into();
    let s = io(sys::fstat(&f))?;
    if FileType::from_raw_mode(s.st_mode) != FileType::RegularFile || s.st_mode & 0o022 != 0 {
        return Err(InstallError::Unsafe);
    }
    if hash_file(&mut f)? != admission.digest {
        return Err(InstallError::ReloadRequired);
    }
    Ok(f)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Observed {
    pub(crate) digest: Option<String>,
    pub(crate) receipt: Option<Vec<u8>>,
}
pub(crate) fn observed(output: &Path, receipt: &Path, kind: Kind) -> Result<Observed> {
    let out = Pin::capture(output.parent().ok_or(InstallError::Unsafe)?)?;
    let bytes = if out.complete {
        read_at(
            &out.open()?,
            output
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or(InstallError::Unsafe)?,
            if kind == Kind::Command { 0o755 } else { 0o600 },
            if kind == Kind::Command {
                MAX_BINARY
            } else {
                65536
            },
        )?
    } else {
        None
    };
    let rp = Pin::capture(receipt.parent().ok_or(InstallError::Unsafe)?)?;
    let receipt = if rp.complete {
        read_at(
            &rp.open()?,
            receipt
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or(InstallError::Unsafe)?,
            0o600,
            MAX_STATE as u64,
        )?
    } else {
        None
    };
    let result = Observed {
        digest: bytes.as_deref().map(digest),
        receipt,
    };
    match (&result.digest, &result.receipt) {
        (None, None) => {}
        (Some(hash), Some(b)) => {
            let r: Receipt = parse(b)?;
            r.validate(kind)?;
            if &r.installed_digest != hash {
                return Err(InstallError::Collision);
            }
        }
        _ => return Err(InstallError::Collision),
    }
    Ok(result)
}
#[derive(Clone, PartialEq, Eq)]
pub struct CommandPlan {
    source: PathBuf,
    output: PathBuf,
    admission: Admitted,
    parent: Pin,
    state: Pin,
    old: Observed,
    disposition: Disposition,
    pending: Option<Vec<u8>>,
}
impl CommandPlan {
    pub(crate) fn recheck(&self, installer: &ManagedInstaller) -> Result<()> {
        if installer.preview(&self.source, self.admission.clone())? != *self {
            return Err(InstallError::StalePreview);
        }
        Ok(())
    }
    pub(crate) fn after_preparation(&self, installer: &ManagedInstaller) -> Result<Self> {
        self.parent.existing_check()?;
        self.state.existing_check()?;
        let new = installer.preview(&self.source, self.admission.clone())?;
        if self.old != new.old || self.pending != new.pending || self.disposition != new.disposition
        {
            return Err(InstallError::StalePreview);
        }
        Ok(new)
    }
    pub fn disposition(&self) -> Disposition {
        self.disposition
    }
    pub fn command_path(&self) -> &Path {
        &self.output
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandVersion {
    installation: String,
    version: String,
    sha256: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandChange {
    disposition: &'static str,
    installed: Option<CommandVersion>,
    proposed: CommandVersion,
}
impl CommandPlan {
    /// Secret-free consent facts, not permission inferred from a token label or a filesystem manifest.
    pub fn summary(&self) -> Result<CommandChange> {
        let installed = self
            .old
            .receipt
            .as_ref()
            .map(|bytes| {
                let r: Receipt = parse(bytes)?;
                r.validate(Kind::Command)?;
                Ok(CommandVersion {
                    installation: r.admission.owner,
                    version: r.admission.version,
                    sha256: r.installed_digest,
                })
            })
            .transpose()?;
        Ok(CommandChange {
            disposition: match self.disposition {
                Disposition::Install => "install",
                Disposition::Unchanged => "unchanged",
                Disposition::Refresh => "refresh",
                Disposition::ExplicitRepair => "explicit-repair",
            },
            installed,
            proposed: CommandVersion {
                installation: self.admission.owner.clone(),
                version: self.admission.version.clone(),
                sha256: self.admission.digest.clone(),
            },
        })
    }
}
/// Recognition of installed bytes, independent of a running candidate's source validity.
pub struct CommandObservation {
    pub version: Option<String>,
    pub pending: bool,
    /// New bytes are already published; finish via owned recovery, not another ordinary preview.
    pub published_pending: bool,
}
/// This object has no authorization methods. Construction/preview never creates a directory.
pub struct ManagedInstaller {
    pub(crate) home: PathBuf,
    pub(crate) home_pin: Pin,
    fault: Option<Arc<dyn InstallFault>>,
}
impl ManagedInstaller {
    pub fn for_home(home: &Path) -> Result<Self> {
        let home_pin = Pin::capture(home)?;
        if !home_pin.complete {
            return Err(InstallError::Unsafe);
        }
        home_pin.open()?;
        Ok(Self {
            home: home.into(),
            home_pin,
            fault: None,
        })
    }
    /// Production resolution is the same bounded effective-user OS adapter as the CLI (D1).
    pub async fn for_effective_user() -> Result<Self> {
        let path = crate::cli::home::resolve()
            .await
            .map_err(|_| InstallError::Unsafe)?;
        tokio::task::spawn_blocking(move || Self::for_home(&path))
            .await
            .map_err(|_| InstallError::Io)?
    }
    pub fn with_fault(mut self, fault: Arc<dyn InstallFault>) -> Self {
        self.fault = Some(fault);
        self
    }
    pub fn command_path(&self) -> PathBuf {
        self.home.join(".local/bin/tmx")
    }
    pub(crate) fn state_path(&self) -> PathBuf {
        self.home
            .join(".local/share/teamarness/external-gateway/managed")
    }
    pub(crate) fn boundary(&self, b: Boundary) -> Result<()> {
        if let Some(f) = &self.fault {
            io(f.before(b))?;
        }
        Ok(())
    }
    pub(crate) fn lock(&self) -> Result<(File, File)> {
        self.home_pin.check()?;
        // Same traversal/private-parent rules as ClientStore, without publishing credentials.
        let team = self.home.join(".local/share/teamarness");
        let state = self.home.join(".local/share/teamarness/external-gateway");
        open_dir(&team, true, true)?;
        open_dir(&state, true, true)?;
        let dir = open_dir(&self.state_path(), true, true)?;
        let lock: File = io(sys::openat(
            &dir,
            "install.lock",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::RUSR | Mode::WUSR,
        ))?
        .into();
        file_safe(&lock, 0o600)?;
        sys::flock(&lock, sys::FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            if e == rustix::io::Errno::WOULDBLOCK {
                InstallError::Busy
            } else {
                InstallError::Io
            }
        })?;
        Ok((dir, lock))
    }
    /// Read only: no installer lock, directory creation, temporary cleanup or recovery.
    pub fn observe_command(&self) -> Result<CommandObservation> {
        self.home_pin.check()?;
        let output = self.command_path();
        let receipt = self.state_path().join(COMMAND_RECEIPT);
        let state = Pin::capture(&self.state_path())?;
        let pending = pending_journal(&state, "command.journal")?;
        let mut published_pending = false;
        let installed = if let Some(bytes) = &pending {
            let j: Journal = parse(bytes)?;
            if j.kind != Kind::Command || j.output != output || j.receipt != receipt {
                return Err(InstallError::InvalidState);
            }
            for pin in &j.pins {
                pin.check()?;
            }
            let actual = raw_digest(&output, Kind::Command)?;
            if actual == Some(j.new_digest()?) {
                let current = read_at(&state.open()?, COMMAND_RECEIPT, 0o600, MAX_STATE as u64)?;
                if current != j.old_receipt && current.as_ref() != Some(&j.new_receipt) {
                    return Err(InstallError::Collision);
                }
                published_pending = true;
                Some(j.new_receipt)
            } else if actual == j.old_digest {
                ensure_old(&j)?;
                j.old_receipt
            } else {
                return Err(InstallError::Collision);
            }
        } else {
            observed(&output, &receipt, Kind::Command)?.receipt
        };
        let version = installed
            .map(|bytes| parse::<Receipt>(&bytes).map(|r| r.admission.version))
            .transpose()?;
        Ok(CommandObservation {
            version,
            pending: pending.is_some(),
            published_pending,
        })
    }
    pub fn preview(&self, source: &Path, admission: Admitted) -> Result<CommandPlan> {
        self.home_pin.check()?;
        admission.validate()?;
        source_file(source, &admission)?;
        let state = Pin::capture(&self.state_path())?;
        let pending = pending_journal(&state, "command.journal")?;
        let output = self.command_path();
        let parent = Pin::capture(output.parent().unwrap())?;
        let old = observed(
            &output,
            &self.state_path().join(COMMAND_RECEIPT),
            Kind::Command,
        )?;
        if let Some(bytes) = &pending {
            let j: Journal = parse(bytes)?;
            j.validate()?;
            if j.kind != Kind::Command
                || j.output != output
                || j.receipt != self.state_path().join(COMMAND_RECEIPT)
            {
                return Err(InstallError::InvalidState);
            }
            ensure_old(&j)?;
            for pin in &j.pins {
                pin.check()?;
            }
        }
        let disposition = if pending.is_some() {
            Disposition::ExplicitRepair
        } else {
            match &old.receipt {
                None => Disposition::Install,
                Some(b) => {
                    let r: Receipt = parse(b)?;
                    if r.admission == admission {
                        Disposition::Unchanged
                    } else if r.admission.owner == admission.owner
                        && semver::Version::parse(&admission.version)
                            .unwrap()
                            .cmp_precedence(&semver::Version::parse(&r.admission.version).unwrap())
                            .is_gt()
                    {
                        Disposition::Refresh
                    } else {
                        Disposition::ExplicitRepair
                    }
                }
            }
        };
        Ok(CommandPlan {
            source: source.into(),
            output,
            admission,
            parent,
            state,
            old,
            disposition,
            pending,
        })
    }
    pub fn apply(&self, plan: &CommandPlan, approval: Approval) -> Result<()> {
        if plan.output != self.command_path() {
            return Err(InstallError::StalePreview);
        }
        if approval == Approval::Automatic
            && matches!(
                plan.disposition,
                Disposition::Install | Disposition::ExplicitRepair
            )
        {
            return Err(InstallError::ConsentRequired);
        }
        plan.parent.check()?;
        plan.state.check()?;
        let mut source = source_file(&plan.source, &plan.admission)?;
        let (state, _lock) = self.lock()?;
        plan.parent.existing_check()?;
        plan.state.existing_check()?;
        if read_at(&state, "command.journal", 0o600, MAX_STATE as u64)? != plan.pending {
            return Err(InstallError::StalePreview);
        }
        let receipt = self.state_path().join(COMMAND_RECEIPT);
        if observed(&plan.output, &receipt, Kind::Command)? != plan.old {
            return Err(InstallError::StalePreview);
        }
        if plan.disposition == Disposition::Unchanged {
            return Ok(());
        }
        let out_dir = open_dir(plan.output.parent().unwrap(), true, false)?;
        let parent = Pin::capture(plan.output.parent().unwrap())?;
        let next = encode(&Receipt::new(
            plan.admission.clone(),
            plan.admission.digest.clone(),
            Kind::Command,
        ))?;
        let journal = Journal::new(
            Kind::Command,
            plan.output.clone(),
            receipt,
            vec![parent.clone(), Pin::capture(&self.state_path())?],
            plan.old.clone(),
            next,
        );
        let temp = ".tmx.tmp";
        // Abandoned temps are discarded, never promoted; executable-mode leftovers need recorded bytes.
        discard_command_temp(&out_dir, plan.pending.as_deref())?;
        self.write_journal(&state, "command.journal", &journal)?;
        let mut copy = create(&out_dir, temp)?;
        self.boundary(Boundary::CopyWrite)?;
        io(source.seek(SeekFrom::Start(0)))?;
        io(std::io::copy(&mut source.take(MAX_BINARY + 1), &mut copy))?;
        self.boundary(Boundary::CopiedVerify)?;
        if hash_file(&mut copy)? != plan.admission.digest {
            return Err(InstallError::ReloadRequired);
        }
        io(sys::fchmod(&copy, Mode::from_raw_mode(0o755)))?;
        self.boundary(Boundary::FileSync)?;
        io(copy.sync_all())?;
        parent.check()?;
        ensure_old(&journal)?;
        self.boundary(Boundary::OutputRename)?;
        parent.check()?;
        ensure_old(&journal)?;
        if plan.old.digest.is_none() {
            io(sys::renameat_with(
                &out_dir,
                temp,
                &out_dir,
                "tmx",
                sys::RenameFlags::NOREPLACE,
            ))?;
        } else {
            io(sys::renameat(&out_dir, temp, &out_dir, "tmx"))?;
        }
        self.boundary(Boundary::OutputDirectorySync)?;
        io(out_dir.sync_all())?;
        self.finish(&state, "command.journal", &journal)
    }
    pub fn recover_command(&self) -> Result<Recovery> {
        self.recover("command.journal", Kind::Command)
    }
    pub(crate) fn write_journal(&self, state: &File, name: &str, j: &Journal) -> Result<()> {
        self.atomic(
            state,
            name,
            &encode(j)?,
            [
                Boundary::JournalWrite,
                Boundary::JournalSync,
                Boundary::JournalRename,
                Boundary::JournalDirectorySync,
            ],
        )
    }
    fn atomic(&self, dir: &File, name: &str, bytes: &[u8], b: [Boundary; 4]) -> Result<()> {
        self.atomic_checked(dir, name, bytes, b, || Ok(()))
    }
    fn atomic_checked(
        &self,
        dir: &File,
        name: &str,
        bytes: &[u8],
        b: [Boundary; 4],
        check: impl Fn() -> Result<()>,
    ) -> Result<()> {
        let temp = format!("{name}.tmp");
        discard_temp(dir, &temp, 0o600)?;
        let mut f = create(dir, &temp)?;
        self.boundary(b[0])?;
        io(f.write_all(bytes))?;
        self.boundary(b[1])?;
        io(f.sync_all())?;
        self.boundary(b[2])?;
        check()?;
        io(sys::renameat(dir, &temp, dir, name))?;
        self.boundary(b[3])?;
        io(dir.sync_all())
    }
    pub(crate) fn finish(&self, state: &File, name: &str, j: &Journal) -> Result<()> {
        j.validate()?;
        for p in &j.pins {
            p.check()?;
        }
        if raw_digest(&j.output, j.kind)? != Some(j.new_digest()?) {
            return Err(InstallError::Collision);
        }
        let rp = Pin::capture(j.receipt.parent().unwrap())?.open()?;
        let current = read_at(
            &rp,
            j.receipt
                .file_name()
                .unwrap()
                .to_str()
                .ok_or(InstallError::Unsafe)?,
            0o600,
            MAX_STATE as u64,
        )?;
        if current != j.old_receipt && current != Some(j.new_receipt.clone()) {
            return Err(InstallError::Collision);
        }
        self.atomic_checked(
            &rp,
            j.receipt.file_name().unwrap().to_str().unwrap(),
            &j.new_receipt,
            [
                Boundary::ReceiptWrite,
                Boundary::ReceiptSync,
                Boundary::ReceiptRename,
                Boundary::ReceiptDirectorySync,
            ],
            || {
                for pin in &j.pins {
                    pin.check()?;
                }
                if raw_digest(&j.output, j.kind)? != Some(j.new_digest()?) {
                    return Err(InstallError::Collision);
                }
                let current = read_at(
                    &rp,
                    j.receipt.file_name().unwrap().to_str().unwrap(),
                    0o600,
                    MAX_STATE as u64,
                )?;
                if current != j.old_receipt && current != Some(j.new_receipt.clone()) {
                    return Err(InstallError::Collision);
                }
                Ok(())
            },
        )?;
        self.boundary(Boundary::JournalRemove)?;
        for pin in &j.pins {
            pin.check()?;
        }
        if raw_digest(&j.output, j.kind)? != Some(j.new_digest()?) {
            return Err(InstallError::Collision);
        }
        io(sys::unlinkat(state, name, sys::AtFlags::empty()))?;
        self.boundary(Boundary::CompletionSync)?;
        io(state.sync_all())
    }
    fn recover(&self, name: &str, kind: Kind) -> Result<Recovery> {
        let pin = Pin::capture(&self.state_path())?;
        if pending_journal(&pin, name)?.is_none() {
            return Ok(Recovery::NothingPending);
        }
        let (state, _lock) = self.lock()?;
        let Some(bytes) = read_at(&state, name, 0o600, MAX_STATE as u64)? else {
            return Ok(Recovery::NothingPending);
        };
        let j: Journal = parse(&bytes)?;
        j.validate()?;
        if j.kind != kind
            || (kind == Kind::Command
                && (j.output != self.command_path()
                    || j.receipt != self.state_path().join(COMMAND_RECEIPT)))
        {
            return Err(InstallError::InvalidState);
        }
        let actual = raw_digest(&j.output, kind)?;
        if kind == Kind::Skill && j.old_digest.is_none() && actual.is_none() {
            j.pins[0].check()?;
            return Ok(Recovery::RepairRequired);
        }
        for p in &j.pins {
            p.check()?;
        }
        if actual == Some(j.new_digest()?) {
            self.finish(&state, name, &j)?;
            Ok(Recovery::Completed)
        } else if actual == j.old_digest {
            ensure_old(&j)?;
            Ok(Recovery::RepairRequired)
        } else {
            Err(InstallError::Collision)
        }
    }
    pub(crate) fn recover_skill(&self) -> Result<Recovery> {
        self.recover("skill.journal", Kind::Skill)
    }
}
pub(crate) fn create(dir: &File, name: &str) -> Result<File> {
    let f: File = io(sys::openat(
        dir,
        name,
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    ))?
    .into();
    file_safe(&f, 0o600)?;
    Ok(f)
}
pub(crate) fn discard_temp(dir: &File, name: &str, mode: u32) -> Result<()> {
    if read_at(dir, name, mode, MAX_BINARY)?.is_some() {
        io(sys::unlinkat(dir, name, sys::AtFlags::empty()))?;
    }
    Ok(())
}
fn raw_digest(path: &Path, kind: Kind) -> Result<Option<String>> {
    let p = Pin::capture(path.parent().ok_or(InstallError::Unsafe)?)?;
    if !p.complete {
        return Ok(None);
    }
    Ok(read_at(
        &p.open()?,
        path.file_name()
            .and_then(|s| s.to_str())
            .ok_or(InstallError::Unsafe)?,
        if kind == Kind::Command { 0o755 } else { 0o600 },
        if kind == Kind::Command {
            MAX_BINARY
        } else {
            65536
        },
    )?
    .as_deref()
    .map(digest))
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Journal {
    pub(crate) format_version: u32,
    pub(crate) kind: Kind,
    pub(crate) output: PathBuf,
    pub(crate) receipt: PathBuf,
    pub(crate) pins: Vec<Pin>,
    pub(crate) old_digest: Option<String>,
    pub(crate) old_receipt: Option<Vec<u8>>,
    pub(crate) new_receipt: Vec<u8>,
    pub(crate) stage_name: Option<String>,
}
impl Journal {
    pub(crate) fn new(
        kind: Kind,
        output: PathBuf,
        receipt: PathBuf,
        pins: Vec<Pin>,
        old: Observed,
        new_receipt: Vec<u8>,
    ) -> Self {
        Self {
            format_version: 1,
            kind,
            output,
            receipt,
            pins,
            old_digest: old.digest,
            old_receipt: old.receipt,
            new_receipt,
            stage_name: None,
        }
    }
    pub(crate) fn validate(&self) -> Result<()> {
        if self.format_version != 1
            || self.pins.len() != 2
            || self.old_digest.is_some() != self.old_receipt.is_some()
        {
            return Err(InstallError::InvalidState);
        }
        if self.kind == Kind::Skill && self.old_digest.is_none() {
            let name = self
                .stage_name
                .as_deref()
                .ok_or(InstallError::InvalidState)?;
            if !name.starts_with(".teamarness-")
                || !name.ends_with(".tmp")
                || name.len() != 48
                || !name.as_bytes()[12..44]
                    .iter()
                    .copied()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(InstallError::InvalidState);
            }
        } else if self.stage_name.is_some() {
            return Err(InstallError::InvalidState);
        }
        let new: Receipt = parse(&self.new_receipt)?;
        new.validate(self.kind)?;
        let output_parent = self.output.parent().ok_or(InstallError::InvalidState)?;
        let receipt_parent = self.receipt.parent().ok_or(InstallError::InvalidState)?;
        if self.kind == Kind::Command
            && (self.pins[0].path != output_parent || self.pins[1].path != receipt_parent)
        {
            return Err(InstallError::InvalidState);
        }
        if self.kind == Kind::Skill
            && (self.pins[0].path != output_parent.parent().ok_or(InstallError::InvalidState)?
                || self.pins[1].path != output_parent)
        {
            return Err(InstallError::InvalidState);
        }
        if let Some(bytes) = &self.old_receipt {
            let old: Receipt = parse(bytes)?;
            old.validate(self.kind)?;
            if Some(old.installed_digest) != self.old_digest {
                return Err(InstallError::InvalidState);
            }
        }
        if self.kind == Kind::Skill
            && (self.output.file_name().and_then(|s| s.to_str()) != Some("SKILL.md")
                || self
                    .output
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    != Some("teamarness")
                || self.receipt != self.output.parent().unwrap().join(SKILL_RECEIPT))
        {
            return Err(InstallError::InvalidState);
        }
        Ok(())
    }
    fn new_digest(&self) -> Result<String> {
        Ok(parse::<Receipt>(&self.new_receipt)?.installed_digest)
    }
}
pub(crate) fn ensure_old(j: &Journal) -> Result<()> {
    if observed(&j.output, &j.receipt, j.kind)?
        != (Observed {
            digest: j.old_digest.clone(),
            receipt: j.old_receipt.clone(),
        })
    {
        return Err(InstallError::StalePreview);
    }
    Ok(())
}

pub(crate) fn pending_journal(pin: &Pin, name: &str) -> Result<Option<Vec<u8>>> {
    if !pin.complete {
        return Ok(None);
    }
    let bytes = read_at(&pin.open()?, name, 0o600, MAX_STATE as u64)?;
    if let Some(b) = &bytes {
        let j: Journal = parse(b)?;
        j.validate()?;
    }
    Ok(bytes)
}
fn discard_command_temp(dir: &File, pending: Option<&[u8]>) -> Result<()> {
    match read_at(dir, ".tmx.tmp", 0o600, MAX_BINARY) {
        Ok(Some(_)) => {
            if pending.is_none() {
                return Err(InstallError::Collision);
            }
            io(sys::unlinkat(dir, ".tmx.tmp", sys::AtFlags::empty()))
        }
        Ok(None) => Ok(()),
        Err(InstallError::Unsafe) => {
            let j: Journal = parse(pending.ok_or(InstallError::Collision)?)?;
            let bytes =
                read_at(dir, ".tmx.tmp", 0o755, MAX_BINARY)?.ok_or(InstallError::Collision)?;
            if digest(&bytes) != j.new_digest()? {
                return Err(InstallError::Collision);
            }
            io(sys::unlinkat(dir, ".tmx.tmp", sys::AtFlags::empty()))
        }
        Err(e) => Err(e),
    }
}
