#![forbid(unsafe_code)]
fn main() -> std::process::ExitCode {
    teamarness_generated_package_backend::cli::entry(std::env::args_os().collect())
}
