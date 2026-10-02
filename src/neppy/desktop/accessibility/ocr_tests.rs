use super::*;
use serde_json::json;

#[test]
fn parse_ocr_caps_and_flags_truncation() {
    let o = parse_ocr(&json!({"text": "abcdef", "ms": 12, "lines": 2}), 4).unwrap();
    assert_eq!(o.text, "abcd");
    assert!(o.truncated);
    assert_eq!((o.ms, o.lines), (12, 2));
    assert!(parse_ocr(&json!({}), 4).is_err());
}

#[test]
fn parse_signature_validates_shape() {
    let ok = parse_signature(&json!({"cols": 2, "rows": 1, "signature": "0aff"})).unwrap();
    assert_eq!(ok.cells, vec![0x0a, 0xff]);
    assert!(parse_signature(&json!({"cols": 2, "rows": 2, "signature": "0aff"})).is_err());
    assert!(parse_signature(&json!({"cols": 2, "rows": 1})).is_err());
}

#[test]
fn defaults_use_explicit_language_and_downscale() {
    let d = OcrOpts::default();
    assert_eq!(d.languages, vec!["en-US"]);
    assert!(d.max_dim > 0);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn non_macos_stubs_are_unsupported() {
    let p = Path::new("/nonexistent.png");
    assert_eq!(
        ocr_png(p, &OcrOpts::default()),
        Err(SensorError::Unsupported)
    );
    assert_eq!(frame_signature(p), Err(SensorError::Unsupported));
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "live macOS: needs swiftc + Vision; renders a PNG with qlmanage"]
fn live_ocr_reads_rendered_text() {
    use std::process::Command;
    let dir = std::env::temp_dir().join(format!("neppy-ocr-live-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fixture = dir.join("fixture.txt");
    std::fs::write(&fixture, "NEPPY OCR CHECK 4271\n").unwrap();
    let st = Command::new("qlmanage")
        .args(["-t", "-s", "1000", "-o"])
        .arg(&dir)
        .arg(&fixture)
        .output()
        .unwrap();
    assert!(st.status.success());
    let png = dir.join("fixture.txt.png");
    assert!(png.exists(), "qlmanage produced no png");
    let out = ocr_png(&png, &OcrOpts::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.text.contains("4271"), "ocr text was {:?}", out.text);
}
