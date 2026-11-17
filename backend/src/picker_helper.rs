//! Fixed private helper entry. AppKit and synchronous RFD execute on the OS main thread.
//! No Backend runtime, home lookup, Host stdio, credentials or setup writes.
use crate::picker::{FRAME_BYTES, REQUEST, valid_path};
use serde_json::json;
use std::io::{Read, Write};
fn emit(value: serde_json::Value) -> u8 {
    let mut bytes = serde_json::to_vec(&value).expect("fixed picker response");
    bytes.push(b'\n');
    if bytes.len() > FRAME_BYTES || std::io::stdout().lock().write_all(&bytes).is_err() {
        return 2;
    }
    0
}
fn fail(code: &str) -> u8 {
    let _ = emit(json!({"version":1,"outcome":"error","code":code}));
    2
}
/// Also used by non-AppKit fixture children to qualify actual lifeline EOF behavior.
pub fn establish_lifeline() -> std::io::Result<()> {
    let mut request = vec![0; REQUEST.len()];
    std::io::stdin().read_exact(&mut request)?;
    if request != REQUEST {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    // Only this pipe is watched. The OS closes it after parent-only SIGKILL too.
    // No other input is valid; EOF or extra bytes terminate the whole modal helper.
    std::thread::Builder::new()
        .name("picker-lifeline".into())
        .spawn(|| {
            let mut byte = [0];
            let _ = std::io::stdin().read(&mut byte);
            std::process::exit(73);
        })?;
    Ok(())
}
pub fn entry() -> u8 {
    if establish_lifeline().is_err() {
        return fail("picker_request_invalid");
    }
    let Some(main) = objc2::MainThreadMarker::new() else {
        return fail("picker_failed");
    };
    objc2_app_kit::NSApplication::sharedApplication(main).finishLaunching();
    let selected = rfd::FileDialog::new()
        .set_title("Choose a Teamarness skill folder")
        .pick_folder();
    match selected {
        None => emit(json!({"version":1,"outcome":"cancelled"})),
        Some(path) => match path
            .canonicalize()
            .ok()
            .and_then(|p| p.to_str().filter(|s| valid_path(s)).map(str::to_owned))
        {
            Some(path) => emit(json!({"version":1,"outcome":"selected","path":path})),
            None => fail("picker_path_invalid"),
        },
    }
}
