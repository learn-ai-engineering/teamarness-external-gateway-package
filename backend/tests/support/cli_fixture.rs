//! Injected-home subprocess evidence only. Never assembled into the Package Directory.
fn main() -> std::process::ExitCode {
    let Some(home) = std::env::var_os("TEAMARNESS_FIXTURE_HOME").map(std::path::PathBuf::from)
    else {
        eprintln!("fixture home required");
        return std::process::ExitCode::from(2);
    };
    if !home.is_absolute() {
        return std::process::ExitCode::from(2);
    }
    teamarness_generated_package_backend::cli::run(std::env::args().skip(1).collect(), Some(home))
        .into()
}
