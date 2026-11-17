//! CLI interaction major 1. No Host, listener, or AppKit initialization.
pub mod args;
pub mod home;
pub mod output;
pub mod transport;
use crate::{
    client_store::{ClientStore, ResolveError},
    limits::*,
};
use args::{Args, Command};
use output::Failure;
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
pub const CLI_MAJOR: u16 = 1;
pub struct Outcome {
    pub bytes: Vec<u8>,
    pub exit: u8,
    pub hint: Option<&'static str>,
}
impl From<Failure> for Outcome {
    fn from(e: Failure) -> Self {
        Self {
            bytes: e.bytes(),
            exit: e.exit,
            hint: Some(e.hint()),
        }
    }
}
fn success(bytes: Vec<u8>) -> Outcome {
    Outcome {
        bytes,
        exit: 0,
        hint: None,
    }
}
pub async fn execute(
    args: Args,
    home: &Path,
    progress: Arc<AtomicBool>,
) -> Result<Outcome, Failure> {
    let store =
        ClientStore::for_home(home).map_err(|_| Failure::usage("cli.client_store_invalid"))?;
    if args.command == Command::Workspaces {
        return Ok(success(serde_json::to_vec(&json!({"workspaces":store.summaries().map_err(|_|Failure::usage("cli.client_store_invalid"))?})).unwrap()));
    }
    let connection = store
        .resolve(
            args.workspace
                .as_deref()
                .ok_or_else(|| Failure::usage("cli.workspace_required"))?,
        )
        .map_err(|e| match e {
            ResolveError::Ambiguous(ids) => {
                let mut e = Failure::usage("cli.workspace_ambiguous");
                e.details = json!({"candidateIds":ids});
                e
            }
            ResolveError::Missing => Failure::usage("cli.workspace_not_configured"),
            _ => Failure::usage("cli.client_store_invalid"),
        })?;
    let preflight = transport::request(
        connection.port,
        &connection.bearer,
        "/v1",
        vec![],
        SUMMARY_BYTES + ENVELOPE_ALLOWANCE_BYTES,
        None,
    )
    .await?;
    if output::upstream_error(&preflight.bytes) {
        return Ok(Outcome {
            bytes: preflight.bytes,
            exit: 1,
            hint: None,
        });
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&preflight.bytes).map_err(|_| Failure::transport(false))?;
    if manifest["version"] != 1 {
        return Err(Failure::usage("cli.http_contract_unsupported"));
    }
    if manifest["workspace"]["id"] != connection.workspace_id {
        return Err(Failure::usage("cli.workspace_mismatch"));
    }
    let name = manifest["workspace"]["name"]
        .as_str()
        .filter(|n| !n.is_empty())
        .ok_or_else(|| Failure::transport(false))?;
    if preflight.status != 200 || !output::envelope(&preflight.bytes, "manifest") {
        return Err(Failure::transport(false));
    }
    if name != connection.workspace_name {
        store
            .refresh_name(&connection, name)
            .map_err(|_| Failure::usage("cli.client_store_changed"))?;
    }
    if args.command == Command::Status {
        return Ok(success(serde_json::to_vec(&json!({"workspace":{"id":connection.workspace_id,"name":name},"endpoint":format!("http://127.0.0.1:{}",connection.port),"reachable":true})).unwrap()));
    }
    let call = args.command == Command::Call;
    let response = transport::request(
        connection.port,
        &connection.bearer,
        &args.target,
        args.body,
        args.response_limit,
        call.then_some(progress.clone()),
    )
    .await?;
    let upstream_error = output::upstream_error(&response.bytes);
    let valid_success = match args.command {
        Command::Call => output::result_envelope(&response.bytes),
        Command::Contracts => output::envelope(&response.bytes, "contracts"),
        Command::Schema => output::envelope(&response.bytes, "schema"),
        Command::Guide => output::envelope(&response.bytes, "guide"),
        _ => false,
    };
    if !upstream_error && (!(200..300).contains(&response.status) || !valid_success) {
        return Err(Failure::transport(call && progress.load(Ordering::SeqCst)));
    }
    Ok(Outcome {
        bytes: response.bytes,
        exit: if upstream_error { 1 } else { 0 },
        hint: None,
    })
}
/// Fixture callers inject a physical home. Production always uses effective-user OS resolution.
pub fn run(arguments: Vec<String>, injected_home: Option<PathBuf>) -> u8 {
    let parsed = match args::parse(&arguments, &mut std::io::stdin().lock()) {
        Ok(a) => a,
        Err(e) => return emit(e.into()),
    };
    match parsed.command {
        Command::Help => {
            return emit(success(args::HELP.as_bytes().to_vec()));
        }
        Command::Version => {
            return emit(success(
                format!("tmx {} (CLI {CLI_MAJOR})", crate::PACKAGE_VERSION).into_bytes(),
            ));
        }
        _ => {}
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(_) => return emit(Failure::transport(false).into()),
    };
    let outcome=runtime.block_on(async {
        let progress=Arc::new(AtomicBool::new(false));
        let work=async {
            let home=match injected_home {Some(h)=>h,None=>home::resolve().await?};
            execute(parsed,&home,progress.clone()).await
        };
        tokio::select! {biased;
            _=tokio::signal::ctrl_c()=>{let mut e=Failure::transport(progress.load(Ordering::SeqCst));e.exit=130;if !e.uncertain{e.code="cli.cancelled";}Err(e)},
            result=work=>result,
        }
    });
    emit(outcome.unwrap_or_else(Into::into))
}
fn emit(mut outcome: Outcome) -> u8 {
    outcome.bytes.push(b'\n');
    if std::io::stdout().lock().write_all(&outcome.bytes).is_err() {
        return 1;
    }
    if let Some(hint) = outcome.hint {
        let _ = writeln!(std::io::stderr().lock(), "{hint}");
    }
    outcome.exit
}
/// Entry selection must happen before constructing any async runtime or native application.
pub fn entry(arguments: Vec<std::ffi::OsString>) -> std::process::ExitCode {
    let is_tmx = arguments
        .first()
        .and_then(|p| Path::new(p).file_name())
        .is_some_and(|p| p == "tmx");
    let mut args = arguments.into_iter().skip(1).collect::<Vec<_>>();
    let cli = is_tmx || args.first().is_some_and(|a| a == "--tmx-cli");
    if cli {
        if args.first().is_some_and(|a| a == "--tmx-cli") {
            args.remove(0);
        }
        let args = match args
            .into_iter()
            .map(|a| a.into_string())
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(a) => a,
            Err(_) => return emit(Failure::usage("cli.invalid_arguments").into()).into(),
        };
        return run(args, None).into();
    }
    if !args.is_empty() {
        return emit(
            Failure::usage(if args.len() == 1 && args[0] == "--tmx-picker" {
                "cli.picker_unavailable"
            } else {
                "cli.invalid_arguments"
            })
            .into(),
        )
        .into();
    }
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r.block_on(crate::router().run()),
        Err(_) => std::process::ExitCode::FAILURE,
    }
}
