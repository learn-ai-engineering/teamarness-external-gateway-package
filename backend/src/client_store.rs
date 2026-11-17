//! Private machine-local client credentials, deliberately separate from verifier-only Policy.
use crate::config::{StorageError, StorageFault, WriteStage};
use rustix::fs::{self as sys, FileType, Mode, OFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Component, Path},
    sync::Arc,
};

pub const MAX_RECORDS: usize = 128;
pub const MAX_RECORD_BYTES: usize = 16 * 1024;
const DIR: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
const READ: OFlags = OFlags::RDONLY
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC)
    .union(OFlags::NONBLOCK);
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Connection {
    pub format_version: u32,
    pub workspace_id: String,
    pub workspace_name: String,
    pub port: u16,
    pub installation_id: String,
    pub continuity_id: String,
    pub origin_nonce: String,
    pub token_id: String,
    pub generation: u64,
    pub bearer: String,
    pub operation_id: String,
}
#[derive(Serialize)]
pub struct Summary {
    pub id: String,
    pub name: String,
    pub port: u16,
}
#[derive(Debug, PartialEq, Eq)]
pub enum ResolveError {
    Storage(StorageError),
    Missing,
    Ambiguous(Vec<String>),
}
impl From<StorageError> for ResolveError {
    fn from(e: StorageError) -> Self {
        Self::Storage(e)
    }
}
fn io<T>(r: Result<T, impl std::fmt::Debug>) -> Result<T, StorageError> {
    r.map_err(|_| StorageError::Io)
}
fn text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
impl Connection {
    pub fn validate(&self) -> Result<(), StorageError> {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD as B64};
        let parts: Vec<_> = self.bearer.split('.').collect();
        if self.format_version != 1
            || self.port == 0
            || self.generation == 0
            || !text(&self.workspace_id, 256)
            || !text(&self.workspace_name, 1024)
            || !text(&self.installation_id, 256)
            || !text(&self.continuity_id, 256)
            || !text(&self.operation_id, 256)
            || self.origin_nonce.len() != 64
            || !self
                .origin_nonce
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            || parts.len() != 3
            || parts[0] != "eg1"
            || parts[1] != self.token_id
            || [(parts[1], 16), (parts[2], 32)].iter().any(|(s, n)| {
                !B64.decode(s)
                    .is_ok_and(|v| v.len() == *n && B64.encode(v) == *s)
            })
        {
            return Err(StorageError::Invalid);
        }
        if io(serde_json::to_vec(self))?.len() > MAX_RECORD_BYTES {
            return Err(StorageError::Capacity);
        }
        Ok(())
    }
    pub(crate) fn same_origin(&self, other: &Self) -> bool {
        self.workspace_id == other.workspace_id
            && self.installation_id == other.installation_id
            && self.continuity_id == other.continuity_id
            && self.origin_nonce == other.origin_nonce
    }
}
/// No secret-bearing Debug implementation. Constructors accept injected physical homes in tests.
pub struct ClientStore {
    home: File,
    fault: Option<Arc<dyn StorageFault>>,
}
fn stat(file: &File, directory: bool, private: bool) -> Result<(), StorageError> {
    let s = io(sys::fstat(file))?;
    if s.st_uid != rustix::process::geteuid().as_raw()
        || FileType::from_raw_mode(s.st_mode)
            != if directory {
                FileType::Directory
            } else {
                FileType::RegularFile
            }
        || (directory
            && if private {
                s.st_mode & 0o7777 != 0o700
            } else {
                s.st_mode & 0o022 != 0
            })
        || (!directory && (s.st_mode & 0o7777 != 0o600 || s.st_nlink != 1))
    {
        return Err(StorageError::Unsafe);
    }
    Ok(())
}
impl ClientStore {
    pub fn for_home(path: &Path) -> Result<Self, StorageError> {
        if !path.is_absolute() {
            return Err(StorageError::Unsafe);
        }
        let mut home: File = io(sys::open("/", DIR, Mode::empty()))?.into();
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(name) => {
                    let s = io(sys::fstat(&home))?;
                    // System sticky temporary ancestors are safe; user-owned home/intermediates are not writable by others.
                    if (s.st_uid != 0 && s.st_uid != rustix::process::geteuid().as_raw())
                        || (s.st_mode & 0o022 != 0 && !(s.st_uid == 0 && s.st_mode & 0o1000 != 0))
                    {
                        return Err(StorageError::Unsafe);
                    }
                    home = io(sys::openat(&home, name, DIR, Mode::empty()))?.into();
                }
                _ => return Err(StorageError::Unsafe),
            }
        }
        stat(&home, true, false)?;
        Ok(Self { home, fault: None })
    }
    pub fn with_fault(mut self, fault: Arc<dyn StorageFault>) -> Self {
        self.fault = Some(fault);
        self
    }
    pub fn filename(workspace: &str) -> String {
        format!(
            "{}.json",
            Sha256::digest(workspace.as_bytes())
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )
    }
    fn directory(&self, create: bool) -> Result<Option<File>, StorageError> {
        let mut dir = io(self.home.try_clone())?;
        for (name, private) in [
            (".local", false),
            ("share", false),
            ("teamarness", true),
            ("external-gateway", true),
        ] {
            if create {
                match sys::mkdirat(&dir, name, Mode::RWXU) {
                    Ok(()) => io(dir.sync_all())?,
                    Err(rustix::io::Errno::EXIST) => {}
                    Err(_) => return Err(StorageError::Io),
                }
            }
            dir = match sys::openat(&dir, name, DIR, Mode::empty()) {
                Ok(fd) => fd.into(),
                Err(rustix::io::Errno::NOENT) if !create => return Ok(None),
                Err(_) => return Err(StorageError::Unsafe),
            };
            stat(&dir, true, private)?;
        }
        Ok(Some(dir))
    }
    fn read_record(dir: &File, name: &str) -> Result<Connection, StorageError> {
        let f: File = io(sys::openat(dir, name, READ, Mode::empty()))?.into();
        stat(&f, false, true)?;
        let mut bytes = vec![];
        io(f.take(MAX_RECORD_BYTES as u64 + 1).read_to_end(&mut bytes))?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(StorageError::Capacity);
        }
        let value = teamarness_package_contract::json::strict_json(&bytes)
            .map_err(|_| StorageError::Invalid)?;
        let c: Connection = serde_json::from_value(value).map_err(|_| StorageError::Invalid)?;
        c.validate()?;
        if Self::filename(&c.workspace_id) != name {
            return Err(StorageError::Invalid);
        }
        Ok(c)
    }
    fn records(dir: &File) -> Result<Vec<Connection>, StorageError> {
        let mut records = vec![];
        for entry in io(sys::Dir::read_from(dir))? {
            let entry = io(entry)?;
            let name = entry
                .file_name()
                .to_str()
                .map_err(|_| StorageError::Unsafe)?;
            if name.ends_with(".json") {
                if records.len() == MAX_RECORDS {
                    return Err(StorageError::Capacity);
                }
                records.push(Self::read_record(dir, name)?);
            } else if ["managed", "setup"].contains(&name) {
                let managed: File = io(sys::openat(dir, name, DIR, Mode::empty()))?.into();
                stat(&managed, true, true)?;
            } else if ![".", "..", "store.lock", "record.tmp"].contains(&name) {
                return Err(StorageError::Invalid);
            }
        }
        records.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));
        Ok(records)
    }
    pub fn connections(&self) -> Result<Vec<Connection>, StorageError> {
        self.directory(false)?
            .map(|d| Self::records(&d))
            .unwrap_or_else(|| Ok(vec![]))
    }
    pub fn summaries(&self) -> Result<Vec<Summary>, StorageError> {
        Ok(self
            .connections()?
            .into_iter()
            .map(|c| Summary {
                id: c.workspace_id,
                name: c.workspace_name,
                port: c.port,
            })
            .collect())
    }
    pub fn resolve(&self, target: &str) -> Result<Connection, ResolveError> {
        let records = self.connections()?;
        if let Some(c) = records.iter().find(|c| c.workspace_id == target) {
            return Ok(c.clone());
        }
        let mut matches = records.into_iter().filter(|c| c.workspace_name == target);
        let first = matches.next().ok_or(ResolveError::Missing)?;
        let mut ids = vec![first.workspace_id.clone()];
        ids.extend(matches.map(|c| c.workspace_id));
        if ids.len() != 1 {
            return Err(ResolveError::Ambiguous(ids));
        }
        Ok(first)
    }
    fn stage(&self, stage: WriteStage) -> Result<(), StorageError> {
        if let Some(f) = &self.fault {
            io(f.before(stage))?;
        }
        Ok(())
    }
    fn mutate(&self, c: &Connection, observed: Option<&Connection>) -> Result<(), StorageError> {
        c.validate()?;
        let guard = self.lock()?;
        guard.publish(c, observed)
    }
    /// Nonwaiting cross-process owner. Setup acquires this before any Policy mutation.
    pub fn lock(&self) -> Result<ClientMutation<'_>, StorageError> {
        let dir = self.directory(true)?.ok_or(StorageError::Io)?;
        let lock: File = io(sys::openat(
            &dir,
            "store.lock",
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::RUSR | Mode::WUSR,
        ))?
        .into();
        stat(&lock, false, true)?;
        io(sys::flock(
            &lock,
            sys::FlockOperation::NonBlockingLockExclusive,
        ))?;
        Ok(ClientMutation {
            store: self,
            dir,
            _lock: lock,
        })
    }
    /// Setup publication preserves the Workspace's origin tuple; it cannot implicitly transfer profiles.
    pub fn publish(&self, connection: &Connection) -> Result<(), StorageError> {
        self.mutate(connection, None)
    }
    /// Compare the entire resolved record under the write lock; stale name refresh cannot undo rotation.
    pub fn refresh_name(&self, observed: &Connection, name: &str) -> Result<(), StorageError> {
        let mut next = observed.clone();
        next.workspace_name = name.into();
        self.mutate(&next, Some(observed))
    }
}
pub struct ClientMutation<'a> {
    store: &'a ClientStore,
    dir: File,
    _lock: File,
}
impl ClientMutation<'_> {
    pub fn connections(&self) -> Result<Vec<Connection>, StorageError> {
        ClientStore::records(&self.dir)
    }
    pub fn publish(
        &self,
        c: &Connection,
        observed: Option<&Connection>,
    ) -> Result<(), StorageError> {
        c.validate()?;
        let dir = &self.dir;
        let records = ClientStore::records(dir)?;
        let old = records.iter().find(|r| r.workspace_id == c.workspace_id);
        if old.is_some_and(|r| !r.same_origin(c)) {
            return Err(StorageError::Invalid);
        }
        if let Some(observed) = observed
            && old != Some(observed)
        {
            return Err(StorageError::Invalid);
        }
        if old.is_none() && records.len() == MAX_RECORDS {
            return Err(StorageError::Capacity);
        }
        match sys::openat(dir, "record.tmp", READ, Mode::empty()) {
            Ok(fd) => {
                let f: File = fd.into();
                stat(&f, false, true)?;
                io(sys::unlinkat(dir, "record.tmp", sys::AtFlags::empty()))?;
            }
            Err(rustix::io::Errno::NOENT) => {}
            Err(_) => return Err(StorageError::Unsafe),
        }
        let mut f: File = io(sys::openat(
            dir,
            "record.tmp",
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ))?
        .into();
        stat(&f, false, true)?;
        let bytes = io(serde_json::to_vec(c))?;
        let result = (|| {
            self.store.stage(WriteStage::Write)?;
            io(f.write_all(&bytes))?;
            self.store.stage(WriteStage::FileSync)?;
            io(f.sync_all())?;
            self.store.stage(WriteStage::Rename)?;
            io(sys::renameat(
                dir,
                "record.tmp",
                dir,
                ClientStore::filename(&c.workspace_id),
            ))?;
            self.store.stage(WriteStage::DirectorySync)?;
            io(dir.sync_all())
        })();
        if result.is_err() {
            let _ = sys::unlinkat(dir, "record.tmp", sys::AtFlags::empty());
        }
        result
    }
    /// Exact operation identity: a concurrent replacement/name refresh is never blindly deleted.
    pub fn remove_matching(&self, expected: &Connection) -> Result<(), StorageError> {
        let records = self.connections()?;
        let Some(current) = records
            .iter()
            .find(|c| c.workspace_id == expected.workspace_id)
        else {
            return Ok(());
        };
        if !current.same_origin(expected)
            || current.token_id != expected.token_id
            || current.generation != expected.generation
            || current.operation_id != expected.operation_id
        {
            return Err(StorageError::Invalid);
        }
        self.store.stage(WriteStage::Rename)?;
        io(sys::unlinkat(
            &self.dir,
            ClientStore::filename(&expected.workspace_id),
            sys::AtFlags::empty(),
        ))?;
        self.store.stage(WriteStage::DirectorySync)?;
        io(self.dir.sync_all())
    }
}
