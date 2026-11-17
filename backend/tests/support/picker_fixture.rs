//! Unassembled fault/lifeline child. Never opens AppKit or an ordinary home.
use std::{
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};
use teamarness_generated_package_backend::picker::REQUEST;
fn main() {
    let exe = std::env::current_exe().unwrap();
    let root = exe.parent().unwrap();
    let mode = exe.file_name().unwrap().to_str().unwrap();
    if std::env::args().nth(1).as_deref() == Some("parent") {
        let mut child = Command::new(&exe)
            .arg("--tmx-picker")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut pipe = child.stdin.take().unwrap();
        pipe.write_all(REQUEST).unwrap();
        println!("{}", child.id());
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            std::hint::black_box(&pipe);
            if let Some(status) = child.try_wait().unwrap() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
    }
    assert_eq!(std::env::args().nth(1).as_deref(), Some("--tmx-picker"));
    teamarness_generated_package_backend::picker_helper::establish_lifeline().unwrap();
    std::fs::write(
        root.join(format!("{mode}.pid.tmp")),
        std::process::id().to_string(),
    )
    .unwrap();
    std::fs::rename(
        root.join(format!("{mode}.pid.tmp")),
        root.join(format!("{mode}.pid")),
    )
    .unwrap();
    match mode {
        "hold" => loop {
            std::thread::sleep(Duration::from_secs(1));
        },
        "late" => {
            while !root.join("release").exists() {
                std::thread::sleep(Duration::from_millis(1));
            }
            selected(root);
        }
        "selected" => selected(root),
        "cancelled" => println!("{{\"version\":1,\"outcome\":\"cancelled\"}}"),
        "error" => println!("{{\"version\":1,\"outcome\":\"error\",\"code\":\"picker_failed\"}}"),
        "malformed" => println!("not json secret-sentinel"),
        "oversize" => print!("{}", "x".repeat(8193)),
        "extra" => println!("{{\"version\":1,\"outcome\":\"cancelled\"}}\n{{}}"),
        "nonzero" => {
            println!("{{\"version\":1,\"outcome\":\"cancelled\"}}");
            std::process::exit(42);
        }
        "stderr" => {
            eprint!("{}", "secret-sentinel".repeat(700));
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
        "safe-stderr" => {
            eprintln!("secret-sentinel");
            selected(root);
        }
        "eof" => {}
        _ => panic!("fixture mode"),
    }
}
fn selected(root: &std::path::Path) {
    println!(
        "{}",
        serde_json::json!({"version":1,"outcome":"selected","path":root.join("chosen")})
    );
}
