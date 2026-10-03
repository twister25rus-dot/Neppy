//! Screen capture for Pet Mode: user-triggered region capture and permitted autonomous
//! capture, both reduced to on-device OCR text.
//!
//! Invariants:
//! - **Images are never persisted or returned.** Every capture lands in a [`TempPng`]
//!   (0600, in the temp dir) that is deleted on drop, so also on error and on panic.
//!   The file is dropped immediately after OCR (or after the change check when the
//!   frame is unchanged). Only [`OcrText`] leaves this module.
//! - Capture needs Screen Recording permission. Missing permission returns
//!   [`SensorError::PermissionRequired`]; the system prompt is requested by the caller
//!   (once, on first need) through `request_screen_recording_access`.
//! - Capture is silent: `screencapture -x`. No shutter sound.
//! - Autonomous capture refuses while a secure field is focused, and when the frontmost
//!   app is not the one the caller sampled.
//!
//! Tool choice (macOS 27): `screencapture` is used rather than CGWindowListCreateImage
//! (obsoleted from macOS 15) or ScreenCaptureKit (async, long-lived stream objects, and
//! a recurring system "continue to allow" nag). `screencapture -x -o -l <windowid>`
//! works headlessly, needs no new crates or entitlements, and honours the same TCC
//! grant. The window id comes from the Swift helper (`CGWindowListCopyWindowInfo`,
//! which needs no permission for ids).
//!
//! All functions here block; call them from `spawn_blocking`.
// Parts are only reached from macOS-only code paths; the rest is tested everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::change_detector::{Change, ChangeConfig, ChangeDetector, FrameSignature};
use super::ocr::{OcrOpts, OcrText};
use super::types::{PermissionState, SensorError};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TEMP_PREFIX: &str = "neppy-pet-capture-";

/// A temp PNG that deletes itself on drop (normal return, `?`, and panic unwind).
#[derive(Debug)]
pub(super) struct TempPng {
    path: PathBuf,
}

impl TempPng {
    /// Reserve a fresh path in the temp dir and create it empty with mode 0600.
    pub(super) fn reserve() -> Result<Self, SensorError> {
        Self::reserve_in(&std::env::temp_dir())
    }

    pub(super) fn reserve_in(dir: &Path) -> Result<Self, SensorError> {
        sweep_stale_once(dir);
        let path = dir.join(format!("{TEMP_PREFIX}{}.png", uuid::Uuid::new_v4()));
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&path)
            .map_err(|e| SensorError::Failed(format!("temp file: {}", e.kind())))?;
        Ok(Self { path })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// True when the capture tool wrote something.
    fn has_content(&self) -> bool {
        std::fs::metadata(&self.path)
            .map(|m| m.len() > 0)
            .unwrap_or(false)
    }
}

impl Drop for TempPng {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!(
                "[accessibility][sensors] failed to delete temp capture: {}",
                e.kind()
            ),
        }
    }
}

/// Remove `neppy-pet-capture-*.png` older than `max_age` left by a killed process.
pub(super) fn sweep_stale_captures(dir: &Path, max_age: Duration) -> usize {
    let mut removed = 0;
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with(TEMP_PREFIX) && name.ends_with(".png")) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|age| age >= max_age)
            .unwrap_or(false);
        if old && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn sweep_stale_once(dir: &Path) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let n = sweep_stale_captures(dir, Duration::from_secs(300));
        if n > 0 {
            log::debug!("[accessibility][sensors] swept {n} stale temp capture(s)");
        }
    });
}

/// Result of running a capture program (private: carries the image).
pub(super) enum CaptureOutcome {
    /// The user pressed Esc, or the tool wrote nothing.
    Cancelled,
    Captured(TempPng),
}

/// Run `program base_args... <tmp>` with a deadline; the temp path is appended last.
/// Exposed with an injectable program so tests do not need `screencapture`.
pub(super) fn capture_region_with(
    program: &Path,
    base_args: &[&str],
    timeout: Duration,
) -> Result<CaptureOutcome, SensorError> {
    let png = TempPng::reserve()?;
    let mut child = Command::new(program)
        .args(base_args)
        .arg(png.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| SensorError::Failed(format!("capture tool: {}", e.kind())))?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if png.has_content() {
                    return Ok(CaptureOutcome::Captured(png));
                }
                log::debug!(
                    "[accessibility][sensors] capture produced no file (status ok={})",
                    status.success()
                );
                return Ok(CaptureOutcome::Cancelled);
            }
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SensorError::Timeout); // png dropped -> deleted
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SensorError::Failed(format!("capture wait: {}", e.kind())));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Interactive (user-triggered) region capture
// ---------------------------------------------------------------------------

/// Outcome of a user-triggered region capture. Never contains pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionCapture {
    Cancelled,
    Text(OcrText),
}

/// The user drags a region (or presses Esc); the region is OCR'd on-device and the image
/// is deleted at once. Requires Screen Recording permission, else
/// [`SensorError::PermissionRequired`]. `timeout` bounds how long the user may take.
#[cfg(target_os = "macos")]
pub fn capture_region_interactive(
    timeout: Duration,
    ocr: &OcrOpts,
) -> Result<RegionCapture, SensorError> {
    if super::permissions::detect_screen_recording_permission() != PermissionState::Granted {
        return Err(SensorError::PermissionRequired);
    }
    match capture_region_with(
        Path::new("/usr/sbin/screencapture"),
        &["-i", "-x", "-t", "png"],
        timeout,
    )? {
        CaptureOutcome::Cancelled => Ok(RegionCapture::Cancelled),
        CaptureOutcome::Captured(png) => {
            let result = super::ocr::ocr_png(png.path(), ocr);
            drop(png);
            result.map(RegionCapture::Text)
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn capture_region_interactive(
    _timeout: Duration,
    _ocr: &OcrOpts,
) -> Result<RegionCapture, SensorError> {
    Err(SensorError::Unsupported)
}

// ---------------------------------------------------------------------------
// Autonomous capture (after the user granted Screen Recording)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTarget {
    /// Topmost normal window of the frontmost app.
    ActiveWindow,
    /// The main display.
    MainDisplay,
}

#[derive(Debug, Clone)]
pub struct AutonomousCaptureOpts {
    pub target: CaptureTarget,
    /// Pid of the app the caller sampled. If the frontmost app differs, the capture is
    /// refused with [`SensorError::TargetChanged`] (e.g. the user just switched to a
    /// password manager).
    pub expected_pid: Option<i32>,
    /// Refuse while secure event input is on (default true). A stuck secure-input
    /// process can block capture until it is quit; the error says so.
    pub block_on_secure_input: bool,
    /// Skip change detection and OCR regardless (app/window switch, first frame).
    pub force: bool,
    pub ocr: OcrOpts,
}

impl Default for AutonomousCaptureOpts {
    fn default() -> Self {
        Self {
            target: CaptureTarget::ActiveWindow,
            expected_pid: None,
            block_on_secure_input: true,
            force: false,
            ocr: OcrOpts::default(),
        }
    }
}

/// What one autonomous sample produced.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenObservation {
    /// Frame matched the last OCR'd frame closely enough; OCR was skipped.
    Unchanged { changed_cells: usize },
    /// Frame changed; this is its on-device OCR text.
    Text { text: OcrText, change: Change },
}

/// Seam over the platform pieces so the orchestration is testable without a screen.
pub(super) trait CaptureBackend {
    fn screen_recording(&self) -> PermissionState;
    fn guard(&self, opts: &AutonomousCaptureOpts) -> Result<(), SensorError>;
    fn capture(&self, target: CaptureTarget) -> Result<CaptureOutcome, SensorError>;
    fn signature(&self, png: &Path) -> Result<FrameSignature, SensorError>;
    fn ocr(&self, png: &Path, opts: &OcrOpts) -> Result<OcrText, SensorError>;
}

/// Holds the change-detector baseline between samples. One per companion runtime.
#[derive(Debug, Default)]
pub struct ScreenWatcher {
    detector: ChangeDetector,
}

impl ScreenWatcher {
    pub fn new(cfg: ChangeConfig) -> Self {
        Self {
            detector: ChangeDetector::new(cfg),
        }
    }

    /// Drop the baseline (resume after pause, exclusion change).
    pub fn reset(&mut self) {
        self.detector.reset();
    }

    /// Capture, compare, and OCR only if the frame changed. Blocking.
    #[cfg(target_os = "macos")]
    pub fn observe(
        &mut self,
        opts: &AutonomousCaptureOpts,
    ) -> Result<ScreenObservation, SensorError> {
        self.observe_with(&MacBackend, opts)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn observe(
        &mut self,
        _opts: &AutonomousCaptureOpts,
    ) -> Result<ScreenObservation, SensorError> {
        Err(SensorError::Unsupported)
    }

    pub(super) fn observe_with(
        &mut self,
        backend: &dyn CaptureBackend,
        opts: &AutonomousCaptureOpts,
    ) -> Result<ScreenObservation, SensorError> {
        if backend.screen_recording() != PermissionState::Granted {
            return Err(SensorError::PermissionRequired);
        }
        backend.guard(opts)?;
        let png = match backend.capture(opts.target)? {
            CaptureOutcome::Captured(p) => p,
            CaptureOutcome::Cancelled => {
                return Err(SensorError::Failed("capture produced no image".into()))
            }
        };
        // `png` is dropped (file deleted) on every path out of this function.
        let sig = backend.signature(png.path());
        let sig = match sig {
            Ok(s) => s,
            Err(e) => {
                drop(png);
                return Err(e);
            }
        };
        let change = if opts.force {
            Change::First
        } else {
            self.detector.compare(&sig)
        };
        if !change.should_ocr() {
            drop(png);
            let changed_cells = match change {
                Change::Unchanged { changed_cells } => changed_cells,
                _ => 0,
            };
            log::trace!("[accessibility][sensors] screen unchanged, OCR skipped");
            return Ok(ScreenObservation::Unchanged { changed_cells });
        }
        let ocr = backend.ocr(png.path(), &opts.ocr);
        drop(png); // delete immediately after reading, before anything else
        let text = ocr?;
        // OCR takes long enough for the user to switch apps or focus a secure
        // field mid-capture. Re-check the same guard after it and drop the text
        // when the frame may no longer belong to the target (the baseline is not
        // committed, so the next sample starts fresh).
        if let Err(e) = backend.guard(opts) {
            drop(text);
            log::debug!("[accessibility][sensors] target changed during OCR, text dropped: {e}");
            return Err(e);
        }
        self.detector.commit(sig);
        Ok(ScreenObservation::Text { text, change })
    }
}

#[cfg(target_os = "macos")]
struct MacBackend;

#[cfg(target_os = "macos")]
impl CaptureBackend for MacBackend {
    fn screen_recording(&self) -> PermissionState {
        super::permissions::detect_screen_recording_permission()
    }

    fn guard(&self, opts: &AutonomousCaptureOpts) -> Result<(), SensorError> {
        if let Some(expected) = opts.expected_pid {
            if super::frontmost::frontmost_pid()? != expected {
                return Err(SensorError::TargetChanged);
            }
        }
        if opts.block_on_secure_input && super::frontmost::secure_entry_active()? {
            return Err(SensorError::SecureFieldFocused);
        }
        Ok(())
    }

    fn capture(&self, target: CaptureTarget) -> Result<CaptureOutcome, SensorError> {
        let program = Path::new("/usr/sbin/screencapture");
        let timeout = Duration::from_secs(10);
        match target {
            CaptureTarget::MainDisplay => {
                capture_region_with(program, &["-x", "-m", "-t", "png"], timeout)
            }
            CaptureTarget::ActiveWindow => {
                let resp = super::sensor_util::call_helper(
                    &serde_json::json!({"type": "frontmost_window"}),
                    super::sensor_util::SENSOR_TIMEOUT,
                )
                .map_err(|e| match e {
                    SensorError::Failed(m)
                        if m == "no_capturable_window" || m == "no_frontmost_app" =>
                    {
                        SensorError::NoFocusedApp
                    }
                    other => other,
                })?;
                let window_id = resp
                    .get("window_id")
                    .and_then(|w| w.as_i64())
                    .ok_or(SensorError::NoFocusedApp)?;
                let arg = window_id.to_string();
                capture_region_with(
                    program,
                    &["-x", "-o", "-t", "png", "-l", arg.as_str()],
                    timeout,
                )
            }
        }
    }

    fn signature(&self, png: &Path) -> Result<FrameSignature, SensorError> {
        super::ocr::frame_signature(png)
    }

    fn ocr(&self, png: &Path, opts: &OcrOpts) -> Result<OcrText, SensorError> {
        super::ocr::ocr_png(png, opts)
    }
}

#[cfg(test)]
#[path = "capture_tests.rs"]
mod tests;
