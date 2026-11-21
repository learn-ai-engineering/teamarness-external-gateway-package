//! Effective OS-user home lookup. HOME and cwd are never credential authorities.
//! macOS asks `/usr/bin/dscacheutil`; Linux asks `/usr/bin/getent passwd <euid>`.
//! Both parsers compile everywhere so each is testable on every platform; only
//! `resolve()` selects one by target.
use super::output::Failure;
use std::{path::PathBuf, process::Stdio};
use tokio::io::AsyncReadExt;
fn unavailable() -> Failure {
    Failure::usage("cli.home_unavailable")
}
fn valid_home(home: &str) -> bool {
    home.starts_with('/') && !home.chars().any(char::is_control)
}
/// macOS `dscacheutil -q user -a uid <euid>` output: exactly one `dir: ` line.
pub fn parse(bytes: &[u8]) -> Result<PathBuf, Failure> {
    let text = std::str::from_utf8(bytes).map_err(|_| unavailable())?;
    let fields: Vec<_> = text
        .lines()
        .filter_map(|l| l.strip_prefix("dir: "))
        .collect();
    if fields.len() != 1 || !valid_home(fields[0]) {
        return Err(unavailable());
    }
    Ok(PathBuf::from(fields[0]))
}
/// Linux `getent passwd <euid>` output: exactly one passwd line of exactly seven
/// colon-separated fields whose UID field is the effective UID's canonical decimal
/// form and whose home field is absolute and control-free.
pub fn parse_getent(bytes: &[u8], euid: u32) -> Result<PathBuf, Failure> {
    let text = std::str::from_utf8(bytes).map_err(|_| unavailable())?;
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.is_empty() || line.contains('\n') {
        return Err(unavailable());
    }
    let fields: Vec<_> = line.split(':').collect();
    if fields.len() != 7 || fields[2] != euid.to_string() || !valid_home(fields[5]) {
        return Err(unavailable());
    }
    Ok(PathBuf::from(fields[5]))
}
#[cfg(target_os = "macos")]
pub async fn resolve() -> Result<PathBuf, Failure> {
    lookup(tokio::process::Command::new("/usr/bin/dscacheutil").args([
        "-q",
        "user",
        "-a",
        "uid",
        &rustix::process::geteuid().as_raw().to_string(),
    ]))
    .await
}
#[cfg(not(target_os = "macos"))]
pub async fn resolve() -> Result<PathBuf, Failure> {
    let euid = rustix::process::geteuid().as_raw();
    lookup_with(
        tokio::process::Command::new("/usr/bin/getent")
            .env_clear()
            .args(["passwd", &euid.to_string()]),
        |bytes| parse_getent(bytes, euid),
    )
    .await
}
/// Bounded lookup with the macOS `dscacheutil` parser.
pub async fn lookup(command: &mut tokio::process::Command) -> Result<PathBuf, Failure> {
    lookup_with(command, parse).await
}
/// Bounded, timed, fixed-command lookup: 16 KiB stdout, 4 KiB stderr, two seconds,
/// success exit required, then exactly the supplied parser. Any failure is
/// `cli.home_unavailable` with no fallback.
pub async fn lookup_with(
    command: &mut tokio::process::Command,
    parser: impl FnOnce(&[u8]) -> Result<PathBuf, Failure>,
) -> Result<PathBuf, Failure> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| unavailable())?;
    let mut out = child.stdout.take().ok_or_else(unavailable)?;
    let mut err = child.stderr.take().ok_or_else(unavailable)?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        // stdout/stderr have different concrete pipe types.
        let stdout = async {
            let mut b = vec![];
            (&mut out).take(16385).read_to_end(&mut b).await?;
            Ok::<_, std::io::Error>(b)
        };
        let stderr = async {
            let mut b = vec![];
            (&mut err).take(4097).read_to_end(&mut b).await?;
            Ok::<_, std::io::Error>(b)
        };
        let (o, e) = tokio::try_join!(stdout, stderr).map_err(|_| unavailable())?;
        if o.len() > 16384 || e.len() > 4096 {
            return Err(unavailable());
        }
        if !child.wait().await.map_err(|_| unavailable())?.success() {
            return Err(unavailable());
        }
        parser(&o)
    })
    .await;
    match result {
        Ok(Ok(path)) => Ok(path),
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(unavailable())
        }
    }
}
