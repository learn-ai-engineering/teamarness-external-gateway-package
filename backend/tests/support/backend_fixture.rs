//! Injected-home real-Host qualification driver. Never assembled into dist.
//! Root is mandatory; no ordinary-home fallback and no production override.
#![forbid(unsafe_code)]
use std::os::unix::fs::PermissionsExt;
fn main() -> std::process::ExitCode {
    let Some(home) = std::env::var_os("TEAMARNESS_FIXTURE_HOME").map(std::path::PathBuf::from)
    else {
        eprintln!("fixture home required");
        return 2.into();
    };
    if !home.is_absolute()
        || home.canonicalize().ok().as_ref() != Some(&home)
        || teamarness_generated_package_backend::client_store::ClientStore::for_home(&home).is_err()
        || home
            .metadata()
            .map(|m| m.permissions().mode() & 0o7777 != 0o700)
            .unwrap_or(true)
        || std::env::args_os().len() != 1
    {
        return 2.into();
    }
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(
            teamarness_generated_package_backend::router_with_environment(
                teamarness_generated_package_backend::setup::BackendEnvironment::injected(home),
            )
            .run(),
        ),
        Err(_) => std::process::ExitCode::FAILURE,
    }
}
