#![forbid(unsafe_code)]
fn main() -> std::process::ExitCode {
    // Keep the portable resource in every native mode, including before setup is wired.
    std::hint::black_box(teamarness_generated_package_backend::skill_install::TEMPLATE);
    teamarness_generated_package_backend::cli::entry(std::env::args_os().collect())
}
