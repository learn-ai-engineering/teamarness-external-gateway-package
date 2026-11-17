//! Private physical fixture homes only. Never linked into the production entry.
use rustix::fs::{self as sys, FileType, Mode, OFlags};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    path::{Path, PathBuf},
};
use teamarness_generated_package_backend::client_store::ClientStore;

const DIR: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);
type Result<T> = std::result::Result<T, ()>;

fn private_directory(path: &Path) -> Result<File> {
    if !path.is_absolute() || path.canonicalize().ok().as_deref() != Some(path) {
        return Err(());
    }
    // Reuse the real store's no-follow, owned/non-writable ancestor validation, without writes.
    ClientStore::for_home(path).map_err(|_| ())?;
    let directory: File = sys::open(path, DIR, Mode::empty()).map_err(|_| ())?.into();
    validate(&directory)?;
    Ok(directory)
}
fn validate(directory: &File) -> Result<()> {
    let stat = sys::fstat(directory).map_err(|_| ())?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Directory
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o7777 != 0o700
    {
        return Err(());
    }
    Ok(())
}
/// CLI takes the exact already-derived child; no Installation guessing or home fallback.
pub fn cli_home() -> Result<PathBuf> {
    let path = std::env::var_os("TEAMARNESS_FIXTURE_HOME")
        .map(PathBuf::from)
        .ok_or(())?;
    private_directory(&path)?;
    Ok(path)
}
pub struct Root {
    path: PathBuf,
    directory: File,
}
impl Root {
    /// Required before starting SDK stdio. Opening/reading a root never creates outputs.
    pub fn from_environment() -> Result<Self> {
        let path = std::env::var_os("TEAMARNESS_FIXTURE_ROOT")
            .map(PathBuf::from)
            .ok_or(())?;
        let directory = private_directory(&path)?;
        Ok(Self { path, directory })
    }
    /// SHA-256 of the canonical lowercase Installation identity's UTF-8 bytes, hex lowercase.
    /// Workspace and cwd are deliberately absent: one command/store per test Installation.
    pub fn installation_home(&self, identity: &str) -> Result<PathBuf> {
        if identity.len() != 64
            || !identity
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(());
        }
        self.revalidate()?;
        let name: String = Sha256::digest(identity.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        match sys::mkdirat(&self.directory, &name, Mode::RWXU) {
            Ok(()) => self.directory.sync_all().map_err(|_| ())?,
            Err(rustix::io::Errno::EXIST) => {}
            Err(_) => return Err(()),
        }
        let child: File = sys::openat(&self.directory, &name, DIR, Mode::empty())
            .map_err(|_| ())?
            .into();
        validate(&child)?;
        self.revalidate()?;
        let path = self.path.join(name);
        let reopened = private_directory(&path)?;
        same_directory(&child, &reopened)?;
        Ok(path)
    }
    fn revalidate(&self) -> Result<()> {
        validate(&self.directory)?;
        same_directory(&self.directory, &private_directory(&self.path)?)
    }
}
fn same_directory(a: &File, b: &File) -> Result<()> {
    let a = sys::fstat(a).map_err(|_| ())?;
    let b = sys::fstat(b).map_err(|_| ())?;
    if (a.st_dev, a.st_ino) == (b.st_dev, b.st_ino) {
        Ok(())
    } else {
        Err(())
    }
}
