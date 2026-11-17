//! Versioned private state. All opens are relative to pinned, non-symlink directory descriptors.
use crate::tokens::TokenRecord;
use rustix::fs::{self as sys, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Write},
    path::{Component, Path},
    sync::Arc,
};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub const MAX_TOKENS: usize = 32;
pub const MAX_CONTRACTS: usize = 256;
pub const MAX_TOOLS: usize = 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolSelection {
    pub fingerprint: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ContractSelection {
    pub major: u16,
    pub provider: String,
    pub enabled: bool,
    pub tools: BTreeMap<String, ToolSelection>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<ManagedAssociation>,
    pub config_version: u32,
    pub desired_enabled: bool,
    pub port: Option<u16>,
    pub tokens: Vec<TokenRecord>,
    pub contracts: BTreeMap<String, ContractSelection>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: 1,
            managed: None,
            desired_enabled: false,
            port: None,
            tokens: vec![],
            contracts: BTreeMap::new(),
        }
    }
}
/// Verifier-only continuity marker. Revoked/disconnected entries are tombstones, not repair authority.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManagedAssociation {
    pub token_id: String,
    pub generation: u64,
    pub operation_id: String,
    pub origin_nonce: String,
    pub disposition: ManagedDisposition,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ManagedDisposition {
    Current,
    Revoked,
    Disconnected,
}
impl ManagedAssociation {
    fn valid(&self, tokens: &[TokenRecord]) -> bool {
        crate::tokens::valid_id(&self.token_id)
            && self.generation > 0
            && self.operation_id.len() == 64
            && self
                .operation_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && self.origin_nonce.len() == 64
            && self
                .origin_nonce
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && match self.disposition {
                ManagedDisposition::Current => tokens
                    .iter()
                    .any(|t| t.id == self.token_id && t.generation == self.generation),
                _ => !tokens.iter().any(|t| t.id == self.token_id),
            }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    Unsafe,
    Invalid,
    Capacity,
    Io,
}
impl Config {
    pub fn validate(&self) -> Result<(), StorageError> {
        use teamarness_package_contract::identity::is_valid_package_identity as identity;
        if !matches!(self.config_version, 1 | 2)
            || (self.config_version == 1 && self.managed.is_some())
            || (self.config_version == 2 && self.managed.is_none())
            || self
                .managed
                .as_ref()
                .is_some_and(|m| !m.valid(&self.tokens))
            || self.port == Some(0)
            || self.tokens.iter().any(|t| !t.valid())
        {
            return Err(StorageError::Invalid);
        }
        let mut ids = std::collections::BTreeSet::new();
        if self.tokens.iter().any(|t| !ids.insert(&t.id)) {
            return Err(StorageError::Invalid);
        }
        if self.tokens.len() > MAX_TOKENS
            || self.contracts.len() > MAX_CONTRACTS
            || self
                .contracts
                .values()
                .map(|c| c.tools.len())
                .sum::<usize>()
                > MAX_TOOLS
        {
            return Err(StorageError::Capacity);
        }
        for (id, c) in &self.contracts {
            if !identity(id)
                || !identity(&c.provider)
                || c.major == 0
                || c.tools.iter().any(|(name, tool)| {
                    name.len() > 128
                        || name.is_empty()
                        || tool.fingerprint.len() != 64
                        || !tool
                            .fingerprint
                            .bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                })
            {
                return Err(StorageError::Invalid);
            }
        }
        if serde_json::to_vec(self)
            .map_err(|_| StorageError::Invalid)?
            .len()
            > MAX_CONFIG_BYTES
        {
            return Err(StorageError::Capacity);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteStage {
    Write,
    FileSync,
    Rename,
    DirectorySync,
}
// Fault injection occurs at actual durability boundaries, never in a fake filesystem.
pub trait StorageFault: Send + Sync {
    fn before(&self, stage: WriteStage) -> Result<(), std::io::Error>;
    fn before_origin(&self, _stage: WriteStage) -> Result<(), std::io::Error> {
        Ok(())
    }
}
struct NoFault;
impl StorageFault for NoFault {
    fn before(&self, _: WriteStage) -> Result<(), std::io::Error> {
        Ok(())
    }
}
pub struct FileStore {
    directory: File,
    fault: Arc<dyn StorageFault>,
}
const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
fn io<T>(r: Result<T, impl std::fmt::Debug>) -> Result<T, StorageError> {
    r.map_err(|_| StorageError::Io)
}
fn owned(file: &File, directory: bool) -> Result<(), StorageError> {
    let s = io(sys::fstat(file))?;
    let kind = FileType::from_raw_mode(s.st_mode);
    if s.st_uid != rustix::process::geteuid().as_raw()
        || (directory && (kind != FileType::Directory || s.st_mode & 0o777 != 0o700))
        || (!directory
            && (kind != FileType::RegularFile || s.st_mode & 0o777 != 0o600 || s.st_nlink != 1))
    {
        return Err(StorageError::Unsafe);
    }
    Ok(())
}
impl FileStore {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        if !path.is_absolute() {
            return Err(StorageError::Unsafe);
        }
        let mut directory: File = io(sys::open("/", DIRECTORY, Mode::empty()))?.into();
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(name) => {
                    directory = io(sys::openat(&directory, name, DIRECTORY, Mode::empty()))?.into();
                }
                _ => return Err(StorageError::Unsafe),
            }
        }
        let parent_stat = io(sys::fstat(&directory))?;
        if parent_stat.st_uid != rustix::process::geteuid().as_raw() {
            return Err(StorageError::Unsafe);
        }
        match sys::mkdirat(&directory, "config", Mode::RWXU) {
            Ok(()) => io(directory.sync_all())?,
            Err(rustix::io::Errno::EXIST) => {}
            Err(_) => return Err(StorageError::Io),
        }
        let directory: File =
            io(sys::openat(&directory, "config", DIRECTORY, Mode::empty()))?.into();
        owned(&directory, true)?;
        Ok(Self {
            directory,
            fault: Arc::new(NoFault),
        })
    }
    pub fn read_origin(&self) -> Result<Option<String>, StorageError> {
        let file: File = match sys::openat(
            &self.directory,
            "origin",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(fd) => fd.into(),
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(_) => return Err(StorageError::Unsafe),
        };
        owned(&file, false)?;
        let mut bytes = vec![];
        io(file.take(65).read_to_end(&mut bytes))?;
        let s = String::from_utf8(bytes).map_err(|_| StorageError::Invalid)?;
        if s.len() != 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(StorageError::Invalid);
        }
        Ok(Some(s))
    }
    /// Stable profile-local origin. Created only by accepted setup, never preview/open.
    pub fn setup_origin(
        &self,
        entropy: &dyn crate::tokens::Entropy,
    ) -> Result<String, StorageError> {
        if let Some(s) = self.read_origin()? {
            // Also finishes a prior publication whose directory sync was uncertain.
            io(self.directory.sync_all())?;
            return Ok(s);
        }
        let mut bytes = [0; 32];
        entropy.fill(&mut bytes).map_err(|_| StorageError::Io)?;
        let s = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        // Only this invocation's exclusive stage is ours to remove. A pre-publication
        // death may leave an orphan; retries never scan, adopt or reclaim those bytes.
        for _ in 0..8 {
            let mut suffix = [0; 16];
            entropy.fill(&mut suffix).map_err(|_| StorageError::Io)?;
            let name = format!(
                "origin-{}.tmp",
                suffix
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            );
            let mut file: File = match sys::openat(
                &self.directory,
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => fd.into(),
                Err(rustix::io::Errno::EXIST) => continue,
                Err(_) => return Err(StorageError::Io),
            };
            let result = (|| {
                owned(&file, false)?;
                self.origin_stage(WriteStage::Write)?;
                io(file.write_all(s.as_bytes()))?;
                self.origin_stage(WriteStage::FileSync)?;
                io(file.sync_all())?;
                self.origin_stage(WriteStage::Rename)?;
                match sys::renameat_with(
                    &self.directory,
                    name.as_str(),
                    &self.directory,
                    "origin",
                    sys::RenameFlags::NOREPLACE,
                ) {
                    Ok(()) => {}
                    Err(rustix::io::Errno::EXIST) => {
                        // A competing accepted setup won. Validate it, never replace it.
                        let winner = self.read_origin()?.ok_or(StorageError::Io)?;
                        io(self.directory.sync_all())?;
                        return Ok(winner);
                    }
                    Err(_) => return Err(StorageError::Io),
                }
                self.origin_stage(WriteStage::DirectorySync)?;
                io(self.directory.sync_all())?;
                Ok(s.clone())
            })();
            // Never unlink the final origin, including after uncertain directory sync.
            let _ = sys::unlinkat(&self.directory, name.as_str(), sys::AtFlags::empty());
            return result;
        }
        Err(StorageError::Io)
    }
    pub fn with_fault(mut self, fault: Arc<dyn StorageFault>) -> Self {
        self.fault = fault;
        self
    }
    pub fn read(&self) -> Result<Config, StorageError> {
        let file = sys::openat(
            &self.directory,
            "state.json",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        );
        let file: File = match file {
            Ok(fd) => fd.into(),
            Err(rustix::io::Errno::NOENT) => return Ok(Config::default()),
            Err(_) => return Err(StorageError::Unsafe),
        };
        owned(&file, false)?;
        if io(file.metadata())?.len() > MAX_CONFIG_BYTES as u64 {
            return Err(StorageError::Capacity);
        }
        let mut bytes = vec![];
        io(file
            .take(MAX_CONFIG_BYTES as u64 + 1)
            .read_to_end(&mut bytes))?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(StorageError::Capacity);
        }
        let value = teamarness_package_contract::json::strict_json(&bytes)
            .map_err(|_| StorageError::Invalid)?;
        if value
            .get("configVersion")
            .and_then(serde_json::Value::as_u64)
            == Some(1)
            && value.get("managed").is_some()
        {
            return Err(StorageError::Invalid);
        }
        let config: Config = serde_json::from_value(value).map_err(|_| StorageError::Invalid)?;
        config.validate()?;
        Ok(config)
    }
    /// Call under the policy's serialized owner, with one live Backend generation per
    /// Data Directory. Opening/reading never reclaims a possibly live writer's temporary file.
    /// Errors never acknowledge authorization.
    pub fn write(&self, config: &Config) -> Result<(), StorageError> {
        config.validate()?;
        let committed = self.read()?; // Refuse unsafe/unrecognized state or an unsupported downgrade.
        if committed.config_version > config.config_version {
            return Err(StorageError::Invalid);
        }
        self.discard_abandoned_temp()?;
        let bytes = io(serde_json::to_vec(config))?;
        let fd = io(sys::openat(
            &self.directory,
            "state.tmp",
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ))?;
        let mut file: File = fd.into();
        let result = (|| {
            owned(&file, false)?;
            self.stage(WriteStage::Write)?;
            io(file.write_all(&bytes))?;
            self.stage(WriteStage::FileSync)?;
            io(file.sync_all())?;
            self.stage(WriteStage::Rename)?;
            io(sys::renameat(
                &self.directory,
                "state.tmp",
                &self.directory,
                "state.json",
            ))?;
            self.stage(WriteStage::DirectorySync)?;
            io(self.directory.sync_all())
        })();
        // This name was exclusively created here. Unlink never follows a symlink.
        if result.is_err() {
            let _ = sys::unlinkat(&self.directory, "state.tmp", sys::AtFlags::empty());
        }
        result
    }
    fn discard_abandoned_temp(&self) -> Result<(), StorageError> {
        // The serialized write owner outlives cancelled observers, and the Host retires
        // the prior process before replacement. Only here can state.tmp be abandoned.
        // Its contents are never read or promoted, even when committed state is missing.
        let file: File = match sys::openat(
            &self.directory,
            "state.tmp",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(fd) => fd.into(),
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            Err(_) => return Err(StorageError::Unsafe),
        };
        owned(&file, false)?;
        io(sys::unlinkat(
            &self.directory,
            "state.tmp",
            sys::AtFlags::empty(),
        ))
    }
    fn origin_stage(&self, stage: WriteStage) -> Result<(), StorageError> {
        self.fault
            .before_origin(stage)
            .map_err(|_| StorageError::Io)
    }
    fn stage(&self, stage: WriteStage) -> Result<(), StorageError> {
        self.fault.before(stage).map_err(|_| StorageError::Io)
    }
}
