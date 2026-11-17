//! Private pipe fixtures are protocol evidence, not actual AppKit qualification.
use teamarness_generated_package_backend::picker::{HelperResult, parse_response};
#[test]
fn exact_closed_protocol_and_utf8_path_frame_bounds() {
    assert!(matches!(
        parse_response(b"{\"version\":1,\"outcome\":\"cancelled\"}\n"),
        Ok(HelperResult::Cancelled)
    ));
    assert!(matches!(
        parse_response(
            b"{\"version\":1,\"outcome\":\"selected\",\"path\":\"/private/tmp/a b'c\"}\n"
        ),
        Ok(HelperResult::Selected(_))
    ));
    assert!(matches!(
        parse_response(b"{\"version\":1,\"outcome\":\"error\",\"code\":\"picker_failed\"}\n"),
        Ok(HelperResult::Error)
    ));
    for bytes in [
        b"".as_slice(),
        b"{}",
        b"\xff",
        b"{\"version\":2,\"outcome\":\"cancelled\"}\n",
        b"{\"version\":1,\"outcome\":\"cancelled\",\"path\":\"/tmp\"}\n",
        b"{\"version\":1,\"outcome\":\"cancelled\"}\n{}\n",
        b"{\"version\":1,\"outcome\":\"selected\",\"path\":\"relative\"}\n",
        b"{\"version\":1,\"outcome\":\"error\",\"code\":\"secret sentinel\"}\n",
    ] {
        assert!(parse_response(bytes).is_err(), "{bytes:?}");
    }
    let selected = |path: String| {
        format!(
            "{{\"version\":1,\"outcome\":\"selected\",\"path\":{}}}\n",
            serde_json::to_string(&path).unwrap()
        )
    };
    assert!(parse_response(selected(format!("/{}", "a".repeat(4095))).as_bytes()).is_ok());
    assert!(parse_response(selected(format!("/{}", "é".repeat(2048))).as_bytes()).is_err());
    assert!(parse_response(&vec![b' '; 8193]).is_err());
}
