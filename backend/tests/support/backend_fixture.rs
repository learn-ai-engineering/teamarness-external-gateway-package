//! Injected-home real-Host qualification driver. Never assembled into dist.
//! Host mode requires ROOT; copied basename-tmx CLI mode requires the exact HOME child.
//! Neither has an ordinary-home fallback or a production override.
#![forbid(unsafe_code)]
mod fixture_home;
use teamarness_generated_package_backend::{
    cli::{self, EntryMode},
    setup::BackendEnvironment,
};
fn main() -> std::process::ExitCode {
    match cli::select_entry(std::env::args_os().collect()) {
        EntryMode::Picker => teamarness_generated_package_backend::picker_helper::entry().into(),
        EntryMode::Invalid => 2.into(),
        EntryMode::Cli(args) => match fixture_home::cli_home() {
            Ok(home) => cli::run(args, Some(home)).into(),
            Err(()) => {
                eprintln!("fixture home invalid or missing");
                2.into()
            }
        },
        EntryMode::Backend => {
            let root = match fixture_home::Root::from_environment() {
                Ok(root) => root,
                Err(()) => {
                    eprintln!("fixture root invalid or missing");
                    return 2.into();
                }
            };
            let environment = BackendEnvironment::injected_from_initialization(move |init| {
                root.installation_home(&init.installation().identity)
                    .map_err(|_| {
                        teamarness_generated_package_backend::error("fixture_home_invalid")
                    })
            });
            match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(
                    teamarness_generated_package_backend::router_with_environment(environment)
                        .run(),
                ),
                Err(_) => std::process::ExitCode::FAILURE,
            }
        }
    }
}
