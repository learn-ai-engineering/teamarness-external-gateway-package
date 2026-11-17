//! Apple Silicon macOS OS-user lookup. HOME and cwd are never credential authorities.
use super::output::Failure;
use std::{path::PathBuf, process::Stdio};
use tokio::io::AsyncReadExt;
pub fn parse(bytes: &[u8]) -> Result<PathBuf, Failure> {
    let text = std::str::from_utf8(bytes).map_err(|_| Failure::usage("cli.home_unavailable"))?;
    let fields: Vec<_> = text
        .lines()
        .filter_map(|l| l.strip_prefix("dir: "))
        .collect();
    if fields.len() != 1 || !fields[0].starts_with('/') || fields[0].chars().any(char::is_control) {
        return Err(Failure::usage("cli.home_unavailable"));
    }
    Ok(PathBuf::from(fields[0]))
}
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
pub async fn lookup(command: &mut tokio::process::Command) -> Result<PathBuf, Failure> {
    let fail = || Failure::usage("cli.home_unavailable");
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| fail())?;
    let mut out = child.stdout.take().ok_or_else(fail)?;
    let mut err = child.stderr.take().ok_or_else(fail)?;
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
        let (o, e) = tokio::try_join!(stdout, stderr).map_err(|_| fail())?;
        if o.len() > 16384 || e.len() > 4096 {
            return Err(fail());
        }
        if !child.wait().await.map_err(|_| fail())?.success() {
            return Err(fail());
        }
        parse(&o)
    })
    .await;
    match result {
        Ok(Ok(path)) => Ok(path),
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(fail())
        }
    }
}
