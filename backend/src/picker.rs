//! One owned chooser child, then (only after selection) a mount-scoped selection lease.
//! Fixture executable injection is an OS seam; production uses only admitted current bytes.
use crate::{error, managed_install::Admitted, setup::SetupCoordinator};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
};
use teamarness_package_sdk::{BackendError, CancellationToken, EventSubscription, event_channel};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    task::JoinHandle,
};

pub const REQUEST: &[u8] = b"{\"version\":1,\"operation\":\"choose-skill-folder\"}\n";
pub const FRAME_BYTES: usize = 8192;
pub const PATH_BYTES: usize = 4096;
const STOP: std::time::Duration = std::time::Duration::from_secs(5);
type Result<T> = std::result::Result<T, BackendError>;
#[derive(Debug, PartialEq, Eq)]
pub enum HelperResult {
    Selected(PathBuf),
    Cancelled,
    Error,
}
#[derive(Deserialize)]
#[serde(tag = "outcome", rename_all = "lowercase", deny_unknown_fields)]
enum Response {
    Selected { version: u8, path: String },
    Cancelled { version: u8 },
    Error { version: u8, code: String },
}
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= PATH_BYTES
        && path.starts_with('/')
        && !path.chars().any(char::is_control)
}
pub fn parse_response(bytes: &[u8]) -> Result<HelperResult> {
    if bytes.len() > FRAME_BYTES
        || !bytes.ends_with(b"\n")
        || bytes[..bytes.len() - 1].contains(&b'\n')
    {
        return Err(error("picker_protocol_invalid"));
    }
    match serde_json::from_slice(bytes).map_err(|_| error("picker_protocol_invalid"))? {
        Response::Selected { version: 1, path } if valid_path(&path) => {
            Ok(HelperResult::Selected(path.into()))
        }
        Response::Cancelled { version: 1 } => Ok(HelperResult::Cancelled),
        Response::Error { version: 1, code }
            if [
                "picker_failed",
                "picker_request_invalid",
                "picker_path_invalid",
            ]
            .contains(&code.as_str()) =>
        {
            Ok(HelperResult::Error)
        }
        _ => Err(error("picker_protocol_invalid")),
    }
}
struct Job {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}
struct Memory {
    active: bool,
    closed: bool,
    jobs: Vec<Job>,
}
pub struct Picker {
    source: PathBuf,
    admitted: std::result::Result<Admitted, crate::managed_install::InstallError>,
    memory: Mutex<Memory>,
}
impl Picker {
    pub fn new(
        source: PathBuf,
        admitted: std::result::Result<Admitted, crate::managed_install::InstallError>,
    ) -> Arc<Self> {
        Arc::new(Self {
            source,
            admitted,
            memory: Mutex::new(Memory {
                active: false,
                closed: false,
                jobs: vec![],
            }),
        })
    }
    /// Returns without hashing, spawning or waiting for the Person. Jobs are joined on lifecycle stop.
    pub fn choose(
        self: &Arc<Self>,
        setup: Arc<SetupCoordinator>,
        observer: CancellationToken,
    ) -> Result<EventSubscription> {
        let mut m = self.memory.lock().expect("picker memory");
        m.jobs.retain(|j| !j.task.is_finished());
        if m.closed {
            return Err(error("unavailable"));
        }
        if m.active || m.jobs.len() >= 4 {
            return Err(error("picker_busy"));
        }
        m.active = true;
        let (sender, stream) = event_channel(1);
        let cancel = CancellationToken::new();
        let owned = cancel.clone();
        let this = self.clone();
        let task = tokio::spawn(async move {
            let cancellation = async {
                tokio::select! {biased; _=owned.cancelled()=>{},_=observer.cancelled()=>{},_=sender.closed()=>{}}
            };
            let result = this.child(cancellation).await;
            this.memory.lock().expect("picker memory").active = false;
            if owned.is_cancelled() || observer.is_cancelled() {
                return;
            }
            let work = async {
                match result? {
                    HelperResult::Selected(path) => {
                        let canonical = tokio::task::spawn_blocking(move || path.canonicalize())
                            .await
                            .map_err(|_| error("picker_path_invalid"))?
                            .map_err(|_| error("picker_path_invalid"))?;
                        let text = canonical
                            .to_str()
                            .filter(|s| valid_path(s))
                            .ok_or_else(|| error("picker_path_invalid"))?
                            .to_owned();
                        let id = setup.register_selection(canonical).await?;
                        // This guard spans delivery and the lease; dropping the future invalidates it.
                        let _selection = Selection {
                            setup: setup.clone(),
                            id: id.clone(),
                        };
                        sender
                            .send(json!({"outcome":"selected","selectionId":id,"path":text}))
                            .await
                            .map_err(|_| error("picker_cancelled"))?;
                        std::future::pending::<()>().await;
                    }
                    HelperResult::Cancelled => {
                        let _ = sender.send(json!({"outcome":"cancelled"})).await;
                    }
                    HelperResult::Error => return Err(error("picker_failed")),
                }
                Ok::<_, BackendError>(())
            };
            tokio::select! {biased;
                _=owned.cancelled()=>{}, _=observer.cancelled()=>{}, _=sender.closed()=>{},
                result=work=>{if let Err(e)=result {let _=sender.fail(e).await;}}
            }
        });
        m.jobs.push(Job { cancel, task });
        Ok(stream)
    }
    async fn child(
        &self,
        cancellation: impl std::future::Future<Output = ()>,
    ) -> Result<HelperResult> {
        // Spawn and hash are blocking work. Keep the verified file open through spawn and
        // recheck its identity immediately before using the fixed admitted path.
        let source = self.source.clone();
        let admitted = self
            .admitted
            .clone()
            .map_err(|e| BackendError::new(e.code(), false, json!({})))?;
        let mut child = tokio::task::spawn_blocking(move || {
            let verified = admitted
                .verify_source(&source)
                .map_err(|e| BackendError::new(e.code(), false, json!({})))?;
            use std::os::unix::fs::MetadataExt;
            let a = verified.metadata().map_err(|_| error("reload_required"))?;
            let b = std::fs::symlink_metadata(&source).map_err(|_| error("reload_required"))?;
            if a.dev() != b.dev() || a.ino() != b.ino() {
                return Err(error("reload_required"));
            }
            Command::new(&source)
                .arg("--tmx-picker")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|_| error("picker_spawn_failed"))
        })
        .await
        .map_err(|_| error("picker_spawn_failed"))??;
        let mut lifeline = child.stdin.take().expect("picker stdin");
        let mut stdout = child.stdout.take().expect("picker stdout");
        let stderr = child.stderr.take().expect("picker stderr");
        let frame = async {
            lifeline
                .write_all(REQUEST)
                .await
                .map_err(|_| error("picker_protocol_invalid"))?;
            let mut bytes = Vec::new();
            loop {
                let b = stdout
                    .read_u8()
                    .await
                    .map_err(|_| error("picker_protocol_invalid"))?;
                bytes.push(b);
                if bytes.len() > FRAME_BYTES {
                    return Err(error("picker_protocol_invalid"));
                }
                if b == b'\n' {
                    break;
                }
            }
            let result = parse_response(&bytes)?;
            // Once a terminal appears there is no browsing wait left. Require exit, EOF,
            // and no extra output in a bounded interval before admitting a selection.
            tokio::time::timeout(STOP, async {
                let mut extra = [0];
                if stdout
                    .read(&mut extra)
                    .await
                    .map_err(|_| error("picker_protocol_invalid"))?
                    != 0
                {
                    return Err(error("picker_protocol_invalid"));
                }
                let status = child.wait().await.map_err(|_| error("picker_failed"))?;
                if !status.success() {
                    return Err(error("picker_failed"));
                }
                Ok(result)
            })
            .await
            .map_err(|_| error("picker_protocol_invalid"))?
        };
        let drain = async { bounded_drain(stderr).await };
        let result = tokio::select! {biased;
            _=cancellation=>Err(error("picker_cancelled")),
            result=async {tokio::try_join!(frame,drain).map(|(result,())|result)}=>result,
        };
        drop(lifeline);
        if result.is_err() {
            let _ = child.start_kill();
            if tokio::time::timeout(STOP, child.wait()).await.is_err() {
                return Err(error("picker_cleanup_failed"));
            }
        }
        result
    }
    pub fn close(&self) {
        let mut m = self.memory.lock().expect("picker memory");
        m.closed = true;
        for job in &m.jobs {
            job.cancel.cancel();
        }
    }
    pub async fn stop(&self) {
        self.close();
        let jobs = std::mem::take(&mut self.memory.lock().expect("picker memory").jobs);
        for job in jobs {
            let _ = job.task.await;
        }
    }
    pub fn resume(&self) {
        self.memory.lock().expect("picker memory").closed = false;
    }
}
struct Selection {
    setup: Arc<SetupCoordinator>,
    id: String,
}
impl Drop for Selection {
    fn drop(&mut self) {
        self.setup.invalidate_selection(&self.id);
    }
}
async fn bounded_drain(mut reader: impl AsyncRead + Unpin) -> Result<()> {
    let mut total = 0;
    let mut bytes = [0; 1024];
    loop {
        let n = reader
            .read(&mut bytes)
            .await
            .map_err(|_| error("picker_failed"))?;
        if n == 0 {
            return Ok(());
        }
        total += n;
        if total > FRAME_BYTES {
            return Err(error("picker_failed"));
        }
    }
}
