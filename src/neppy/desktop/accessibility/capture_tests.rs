use super::*;
use crate::neppy::desktop::accessibility::change_detector::{SIGNATURE_COLS, SIGNATURE_ROWS};
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};

fn flat(v: u8) -> FrameSignature {
    FrameSignature {
        cols: SIGNATURE_COLS,
        rows: SIGNATURE_ROWS,
        cells: vec![v; SIGNATURE_COLS as usize * SIGNATURE_ROWS as usize],
    }
}

fn ocr_text(s: &str) -> OcrText {
    OcrText {
        text: s.into(),
        ms: 1,
        lines: 1,
        truncated: false,
    }
}

// ── TempPng ───────────────────────────────────────────────────────────────

#[test]
fn temp_png_is_created_private_and_deleted_on_drop() {
    let png = TempPng::reserve().unwrap();
    let path = png.path().to_path_buf();
    assert!(path.exists());
    assert!(path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with(TEMP_PREFIX));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    drop(png);
    assert!(!path.exists());
}

#[test]
fn temp_png_is_deleted_when_the_holder_panics() {
    let recorded = RefCell::new(None);
    let r = catch_unwind(AssertUnwindSafe(|| {
        let png = TempPng::reserve().unwrap();
        *recorded.borrow_mut() = Some(png.path().to_path_buf());
        panic!("boom");
    }));
    assert!(r.is_err());
    let path = recorded.into_inner().unwrap();
    assert!(!path.exists(), "temp png must be removed on unwind");
}

#[test]
fn sweep_removes_only_old_prefixed_pngs() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join(format!("{TEMP_PREFIX}old.png"));
    let fresh = dir.path().join(format!("{TEMP_PREFIX}fresh.png"));
    let other = dir.path().join("unrelated.png");
    for p in [&old, &fresh, &other] {
        std::fs::write(p, b"x").unwrap();
    }
    let f = std::fs::File::options().write(true).open(&old).unwrap();
    f.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    drop(f);
    assert_eq!(
        sweep_stale_captures(dir.path(), Duration::from_secs(300)),
        1
    );
    assert!(!old.exists() && fresh.exists() && other.exists());
}

// ── capture_region_with (scripted programs) ───────────────────────────────

#[cfg(unix)]
mod region {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Write an executable script that records its last argument to `record`
    /// and then runs `body` (with `$f` = the last argument).
    fn script(dir: &Path, record: &Path, body: &str) -> PathBuf {
        let p = dir.join(format!("fake-capture-{}.sh", uuid::Uuid::new_v4()));
        let text = format!(
            "#!/bin/sh\nfor a; do f=\"$a\"; done\nprintf '%s' \"$f\" > '{}'\n{body}\n",
            record.display()
        );
        std::fs::write(&p, text).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn no_file_written_is_cancelled_and_nothing_is_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("rec");
        let prog = script(dir.path(), &record, "true");
        let out = capture_region_with(&prog, &[], Duration::from_secs(5)).unwrap();
        assert!(matches!(out, CaptureOutcome::Cancelled));
        let path = PathBuf::from(std::fs::read_to_string(&record).unwrap());
        assert!(!path.exists(), "cancelled capture must not leave a file");
    }

    #[test]
    fn usr_bin_true_is_cancelled() {
        let out =
            capture_region_with(Path::new("/usr/bin/true"), &[], Duration::from_secs(5)).unwrap();
        assert!(matches!(out, CaptureOutcome::Cancelled));
    }

    #[test]
    fn written_file_is_captured_and_gone_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("rec");
        let prog = script(dir.path(), &record, "printf 'PNGDATA' > \"$f\"");
        let out = capture_region_with(&prog, &["-x"], Duration::from_secs(5)).unwrap();
        let CaptureOutcome::Captured(png) = out else {
            panic!("expected capture")
        };
        let path = png.path().to_path_buf();
        assert_eq!(std::fs::read(&path).unwrap(), b"PNGDATA");
        drop(png);
        assert!(!path.exists());
    }

    #[test]
    fn timeout_kills_the_tool_and_deletes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("rec");
        let prog = script(
            dir.path(),
            &record,
            "printf 'partial' > \"$f\"; exec sleep 30",
        );
        let started = Instant::now();
        let err = capture_region_with(&prog, &[], Duration::from_millis(1500))
            .err()
            .unwrap();
        assert_eq!(err, SensorError::Timeout);
        assert!(started.elapsed() < Duration::from_secs(5));
        let path = PathBuf::from(std::fs::read_to_string(&record).unwrap());
        assert!(!path.exists());
    }

    #[test]
    fn missing_program_is_a_failure_not_a_panic() {
        let err = capture_region_with(
            Path::new("/nonexistent/screencapture"),
            &[],
            Duration::from_secs(1),
        )
        .err()
        .unwrap();
        assert!(matches!(err, SensorError::Failed(_)));
    }
}

// ── Autonomous orchestration with a fake backend ──────────────────────────

struct Fake {
    permission: PermissionState,
    guard: Result<(), SensorError>,
    /// Per-call guard results consumed first (then `guard` applies).
    guard_seq: RefCell<Vec<Result<(), SensorError>>>,
    frames: RefCell<Vec<FrameSignature>>,
    ocr_result: Result<OcrText, SensorError>,
    ocr_panics: bool,
    sig_fails: bool,
    ocr_calls: Cell<usize>,
    capture_calls: Cell<usize>,
    paths: RefCell<Vec<PathBuf>>,
    existed_at_ocr: RefCell<Vec<bool>>,
}

impl Fake {
    fn new(frames: Vec<FrameSignature>) -> Self {
        Self {
            permission: PermissionState::Granted,
            guard: Ok(()),
            guard_seq: RefCell::new(vec![]),
            frames: RefCell::new(frames),
            ocr_result: Ok(ocr_text("hello")),
            ocr_panics: false,
            sig_fails: false,
            ocr_calls: Cell::new(0),
            capture_calls: Cell::new(0),
            paths: RefCell::new(vec![]),
            existed_at_ocr: RefCell::new(vec![]),
        }
    }
    fn all_deleted(&self) -> bool {
        self.paths.borrow().iter().all(|p| !p.exists())
    }
}

impl CaptureBackend for Fake {
    fn screen_recording(&self) -> PermissionState {
        self.permission.clone()
    }
    fn guard(&self, _o: &AutonomousCaptureOpts) -> Result<(), SensorError> {
        let mut seq = self.guard_seq.borrow_mut();
        if seq.is_empty() {
            self.guard.clone()
        } else {
            seq.remove(0)
        }
    }
    fn capture(&self, _t: CaptureTarget) -> Result<CaptureOutcome, SensorError> {
        self.capture_calls.set(self.capture_calls.get() + 1);
        let png = TempPng::reserve()?;
        std::fs::write(png.path(), b"fakepng").unwrap();
        self.paths.borrow_mut().push(png.path().to_path_buf());
        Ok(CaptureOutcome::Captured(png))
    }
    fn signature(&self, _p: &Path) -> Result<FrameSignature, SensorError> {
        if self.sig_fails {
            return Err(SensorError::Failed("sig".into()));
        }
        let mut f = self.frames.borrow_mut();
        Ok(if f.len() > 1 {
            f.remove(0)
        } else {
            f[0].clone()
        })
    }
    fn ocr(&self, p: &Path, _o: &OcrOpts) -> Result<OcrText, SensorError> {
        self.ocr_calls.set(self.ocr_calls.get() + 1);
        self.existed_at_ocr.borrow_mut().push(p.exists());
        if self.ocr_panics {
            panic!("ocr exploded");
        }
        self.ocr_result.clone()
    }
}

fn opts() -> AutonomousCaptureOpts {
    AutonomousCaptureOpts::default()
}

/// W6: the frontmost app (or secure input) is re-checked AFTER OCR; a frame
/// that may no longer belong to the target is dropped, the image is gone, and
/// the baseline stays unset so the next sample starts fresh.
#[test]
fn target_change_during_ocr_drops_the_text() {
    for late in [SensorError::TargetChanged, SensorError::SecureFieldFocused] {
        let fake = Fake::new(vec![flat(50)]);
        *fake.guard_seq.borrow_mut() = vec![Ok(()), Err(late.clone())];
        let mut w = ScreenWatcher::default();
        assert_eq!(w.observe_with(&fake, &opts()).unwrap_err(), late);
        assert_eq!(fake.ocr_calls.get(), 1);
        assert!(fake.all_deleted());
        // Not committed: the same frame is OCR'd again next time.
        let again = w.observe_with(&fake, &opts()).unwrap();
        assert!(matches!(again, ScreenObservation::Text { .. }));
        assert_eq!(fake.ocr_calls.get(), 2);
    }
}

#[test]
fn first_frame_is_ocrd_identical_frame_is_not() {
    let fake = Fake::new(vec![flat(50), flat(50)]);
    let mut w = ScreenWatcher::default();
    let first = w.observe_with(&fake, &opts()).unwrap();
    assert!(matches!(first, ScreenObservation::Text { .. }));
    assert_eq!(fake.ocr_calls.get(), 1);
    let second = w.observe_with(&fake, &opts()).unwrap();
    assert!(matches!(second, ScreenObservation::Unchanged { .. }));
    assert_eq!(fake.ocr_calls.get(), 1, "identical frame must not be OCR'd");
    assert_eq!(fake.capture_calls.get(), 2);
    assert!(
        fake.all_deleted(),
        "unchanged path must delete the image too"
    );
}

#[test]
fn changed_frame_is_ocrd_and_image_deleted_immediately_after() {
    let mut half = flat(50);
    for c in half.cells.iter_mut().take(1000) {
        *c = 220;
    }
    let fake = Fake::new(vec![flat(50), half]);
    let mut w = ScreenWatcher::default();
    w.observe_with(&fake, &opts()).unwrap();
    let again = w.observe_with(&fake, &opts()).unwrap();
    let ScreenObservation::Text { text, change } = again else {
        panic!("expected OCR")
    };
    assert_eq!(text.text, "hello");
    assert!(matches!(change, Change::Changed { .. }));
    assert_eq!(fake.ocr_calls.get(), 2);
    assert_eq!(*fake.existed_at_ocr.borrow(), vec![true, true]);
    assert!(fake.all_deleted());
}

#[test]
fn ocr_error_deletes_image_and_keeps_baseline_unset_so_it_retries() {
    let mut fake = Fake::new(vec![flat(10)]);
    fake.ocr_result = Err(SensorError::Timeout);
    let mut w = ScreenWatcher::default();
    assert_eq!(
        w.observe_with(&fake, &opts()).unwrap_err(),
        SensorError::Timeout
    );
    assert!(fake.all_deleted());
    // Same frame again: baseline was not committed, so it is OCR'd again.
    assert_eq!(
        w.observe_with(&fake, &opts()).unwrap_err(),
        SensorError::Timeout
    );
    assert_eq!(fake.ocr_calls.get(), 2);
}

#[test]
fn ocr_panic_still_deletes_image() {
    let mut fake = Fake::new(vec![flat(10)]);
    fake.ocr_panics = true;
    let mut w = ScreenWatcher::default();
    let r = catch_unwind(AssertUnwindSafe(|| w.observe_with(&fake, &opts())));
    assert!(r.is_err());
    assert_eq!(fake.paths.borrow().len(), 1);
    assert!(
        fake.all_deleted(),
        "TempPng must be deleted on panic in OCR"
    );
}

#[test]
fn signature_error_deletes_image() {
    let mut fake = Fake::new(vec![flat(10)]);
    fake.sig_fails = true;
    let mut w = ScreenWatcher::default();
    assert!(w.observe_with(&fake, &opts()).is_err());
    assert!(fake.all_deleted());
    assert_eq!(fake.ocr_calls.get(), 0);
}

#[test]
fn missing_permission_captures_nothing() {
    let mut fake = Fake::new(vec![flat(10)]);
    fake.permission = PermissionState::Denied;
    let mut w = ScreenWatcher::default();
    assert_eq!(
        w.observe_with(&fake, &opts()).unwrap_err(),
        SensorError::PermissionRequired
    );
    assert_eq!(fake.capture_calls.get(), 0);
}

#[test]
fn guard_refusal_captures_nothing() {
    for e in [SensorError::SecureFieldFocused, SensorError::TargetChanged] {
        let mut fake = Fake::new(vec![flat(10)]);
        fake.guard = Err(e.clone());
        let mut w = ScreenWatcher::default();
        assert_eq!(w.observe_with(&fake, &opts()).unwrap_err(), e);
        assert_eq!(fake.capture_calls.get(), 0);
        assert!(fake.paths.borrow().is_empty());
    }
}

#[test]
fn force_ocrs_even_when_unchanged_and_reset_clears_baseline() {
    let fake = Fake::new(vec![flat(7)]);
    let mut w = ScreenWatcher::default();
    w.observe_with(&fake, &opts()).unwrap();
    let mut o = opts();
    o.force = true;
    assert!(matches!(
        w.observe_with(&fake, &o).unwrap(),
        ScreenObservation::Text { .. }
    ));
    assert_eq!(fake.ocr_calls.get(), 2);
    w.reset();
    assert!(matches!(
        w.observe_with(&fake, &opts()).unwrap(),
        ScreenObservation::Text { .. }
    ));
    assert_eq!(fake.ocr_calls.get(), 3);
}

#[cfg(not(target_os = "macos"))]
#[test]
fn non_macos_capture_stubs_are_unsupported() {
    assert_eq!(
        capture_region_interactive(Duration::from_secs(1), &OcrOpts::default()).unwrap_err(),
        SensorError::Unsupported
    );
    assert_eq!(
        ScreenWatcher::default().observe(&opts()).unwrap_err(),
        SensorError::Unsupported
    );
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "live macOS: needs Screen Recording + Accessibility for the host app; captures the active window"]
fn live_autonomous_capture_ocr_then_unchanged() {
    use crate::neppy::desktop::accessibility::permissions::detect_screen_recording_permission;
    if detect_screen_recording_permission() != PermissionState::Granted {
        eprintln!("SKIP: screen recording not granted to the host app");
        return;
    }
    let mut w = ScreenWatcher::default();
    let mut o = opts();
    o.target = CaptureTarget::MainDisplay;
    o.block_on_secure_input = false;
    let t = Instant::now();
    let first = w.observe(&o).expect("first observe");
    eprintln!(
        "first: {:?} in {:?}",
        matches!(first, ScreenObservation::Text { .. }),
        t.elapsed()
    );
    assert!(matches!(first, ScreenObservation::Text { .. }));
    let t = Instant::now();
    let second = w.observe(&o).expect("second observe");
    eprintln!("second: {second:?} in {:?}", t.elapsed());
}
