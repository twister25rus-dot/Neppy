use super::*;
use std::path::Path;

#[test]
fn bundle_dir_simple_app() {
    assert_eq!(
        bundle_dir_from_exe_path(Path::new("/Applications/Foo.app/Contents/MacOS/Foo")),
        Some(PathBuf::from("/Applications/Foo.app"))
    );
}

#[test]
fn bundle_dir_nested_helper_resolves_to_innermost() {
    assert_eq!(
        bundle_dir_from_exe_path(Path::new(
            "/Applications/Foo.app/Contents/Frameworks/Bar.app/Contents/MacOS/Bar"
        )),
        Some(PathBuf::from(
            "/Applications/Foo.app/Contents/Frameworks/Bar.app"
        ))
    );
}

#[test]
fn bundle_dir_non_bundle_is_none() {
    assert_eq!(bundle_dir_from_exe_path(Path::new("/usr/bin/ssh")), None);
    assert_eq!(
        bundle_dir_from_exe_path(Path::new("/opt/tools/runner")),
        None
    );
    // The executable itself named like a bundle is not a bundle directory.
    assert_eq!(
        bundle_dir_from_exe_path(Path::new("/usr/bin/thing.app")),
        None
    );
    assert_eq!(bundle_dir_from_exe_path(Path::new("/.app/x")), None);
}

#[test]
fn secure_role_detection() {
    assert!(is_secure_role(Some("AXSecureTextField"), None));
    assert!(is_secure_role(
        Some("AXTextField"),
        Some("AXSecureTextField")
    ));
    assert!(!is_secure_role(Some("AXTextField"), Some("AXSearchField")));
    assert!(!is_secure_role(None, None));
}

#[test]
fn snapshot_opts_default_reads_capped_selection() {
    let o = SnapshotOpts::default();
    assert!(o.want_selection);
    assert!(o.max_selection_chars > 0);
}

/// The native module must never touch whole-field values or flip app AX modes.
#[test]
fn native_source_never_reads_value_or_sets_enhanced_ui() {
    let src = include_str!("frontmost.rs");
    let native = src
        .split("mod native {")
        .nth(1)
        .expect("native module present")
        .split("#[cfg(test)]")
        .next()
        .unwrap();
    for banned in [
        "AXValue\"",
        "kAXValueAttribute",
        "AXEnhancedUserInterface",
        "AXManualAccessibility",
        "AXUIElementSetAttributeValue",
    ] {
        assert!(
            !native.contains(banned),
            "native sensor must not use {banned}"
        );
    }
}

#[cfg(not(target_os = "macos"))]
#[test]
fn non_macos_stubs_are_unsupported() {
    assert_eq!(
        frontmost_snapshot(SnapshotOpts::default()),
        Err(SensorError::Unsupported)
    );
    assert_eq!(secure_entry_active(), Err(SensorError::Unsupported));
    assert_eq!(frontmost_pid(), Err(SensorError::Unsupported));
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "live macOS: needs Accessibility permission for the host app"]
fn live_frontmost_snapshot_has_app_name() {
    let s = frontmost_snapshot(SnapshotOpts::default()).expect("snapshot");
    assert!(!s.app_name.is_empty());
    assert!(s.pid > 0);
    assert!(s.idle_secs >= 0.0);
    eprintln!(
        "app={:?} bundle={:?} role={:?} secure={} title_chars={:?}",
        s.app_name,
        s.bundle_id,
        s.focused_role,
        s.is_secure_field,
        s.window_title.as_ref().map(|t| t.chars().count())
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "live macOS: needs Accessibility permission; 200-sample bench"]
fn live_frontmost_snapshot_bench_p95_under_20ms() {
    let opts = SnapshotOpts::default();
    let _ = frontmost_snapshot(opts).expect("warmup");
    let mut ms: Vec<f64> = (0..200)
        .map(|_| {
            let t = std::time::Instant::now();
            let _ = frontmost_snapshot(opts);
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p95 = ms[(ms.len() * 95 / 100).min(ms.len() - 1)];
    eprintln!(
        "snapshot p50={:.2}ms p95={:.2}ms max={:.2}ms",
        ms[100], p95, ms[199]
    );
    assert!(p95 < 20.0, "p95 {p95}ms");
}
