use super::*;
use serde_json::json;

#[test]
fn peek_parses_change_count() {
    let p = parse_peek(&json!({"change_count": 42, "access_behavior": "ask"})).unwrap();
    assert_eq!(p.change_count, 42);
    assert_eq!(p.access_behavior.as_deref(), Some("ask"));
    assert!(parse_peek(&json!({})).is_err());
}

#[test]
fn plain_text_is_read_and_capped() {
    let r = parse_read(
        &json!({"change_count": 1, "has_text": true, "text": "héllo world", "types": ["public.utf8-plain-text"]}),
        5,
    )
    .unwrap();
    assert_eq!(r.text.as_deref(), Some("héllo"));
    assert!(r.truncated);
    assert!(!r.is_sensitive());
}

#[test]
fn concealed_flag_drops_text_even_if_helper_sent_it() {
    let r = parse_read(
        &json!({"change_count": 2, "concealed": true, "text": "hunter2"}),
        100,
    )
    .unwrap();
    assert!(r.concealed && r.is_sensitive());
    assert_eq!(r.text, None);
}

#[test]
fn sensitive_type_ids_drop_text_even_without_flags() {
    for ty in SENSITIVE_PASTEBOARD_TYPES {
        let r = parse_read(
            &json!({"change_count": 3, "text": "secret", "types": ["public.utf8-plain-text", ty]}),
            100,
        )
        .unwrap();
        assert!(r.is_sensitive(), "{ty}");
        assert_eq!(r.text, None, "{ty}");
    }
}

#[test]
fn transient_and_auto_generated_drop_text() {
    let t = parse_read(
        &json!({"change_count": 4, "transient": true, "text": "x"}),
        9,
    )
    .unwrap();
    assert!(t.transient && t.text.is_none());
    let a = parse_read(
        &json!({"change_count": 5, "auto_generated": true, "text": "x"}),
        9,
    )
    .unwrap();
    assert!(a.auto_generated && a.text.is_none());
}

#[test]
fn missing_change_count_is_an_error() {
    assert!(parse_read(&json!({"text": "x"}), 5).is_err());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn non_macos_stubs_are_unsupported() {
    assert_eq!(clipboard_peek(), Err(SensorError::Unsupported));
    assert_eq!(clipboard_read(10), Err(SensorError::Unsupported));
}

#[cfg(target_os = "macos")]
mod live {
    use super::super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn pbcopy(s: &str) {
        let mut c = Command::new("pbcopy")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
        c.wait().unwrap();
    }

    #[test]
    #[ignore = "live macOS: touches the real clipboard"]
    fn peek_change_count_increments_after_pbcopy() {
        let before = clipboard_peek().unwrap().change_count;
        pbcopy("neppy-live-test-clipboard");
        let after = clipboard_peek().unwrap().change_count;
        assert!(after > before, "{before} -> {after}");
        let read = clipboard_read(100).unwrap();
        assert_eq!(read.text.as_deref(), Some("neppy-live-test-clipboard"));
    }

    #[test]
    #[ignore = "live macOS: touches the real clipboard"]
    fn concealed_pasteboard_reports_flag_and_no_text() {
        let script = r#"ObjC.import('AppKit');
const pb = $.NSPasteboard.generalPasteboard;
pb.clearContents;
pb.setStringForType($('neppy-secret-4271'), $('public.utf8-plain-text'));
pb.setStringForType($(''), $('org.nspasteboard.ConcealedType'));"#;
        let st = Command::new("osascript")
            .args(["-l", "JavaScript", "-e", script])
            .status()
            .unwrap();
        assert!(st.success());
        let read = clipboard_read(100).unwrap();
        assert!(read.concealed, "{read:?}");
        assert_eq!(read.text, None);
        pbcopy("neppy-live-test-clipboard-reset");
    }
}
