//! The sensor seam: the only way the runtime touches the screen. `MacSensor`
//! wraps the `desktop::accessibility` functions (all blocking: the sampler
//! thread calls them directly, async callers go through `spawn_blocking`);
//! tests use `FakeSensor` (`test_support.rs`).
//!
//! Two-phase frontmost read: [`Sensor::frontmost_app`] returns the app identity
//! only, so the runtime can apply app exclusions BEFORE asking for the window
//! title or the selection ([`Sensor::frontmost_context`]). The T1 snapshot reads
//! the title together with the app, so `MacSensor::frontmost_app` drops the
//! title inside this adapter; it never reaches an event, log, store or socket.
//!
//! Nothing here writes the clipboard: the trait has no write API (PC: the core
//! never touches the pasteboard on the user's behalf).

use std::sync::Mutex;
use std::time::Duration;

use crate::neppy::desktop::accessibility::{
    self as ax, AutonomousCaptureOpts, CaptureTarget, ChangeConfig, OcrOpts, PermissionState,
    RegionCapture, ScreenObservation, ScreenWatcher, SensorError, SnapshotOpts,
};

/// Identity of the frontmost app (no title, no field contents).
#[derive(Debug, Clone, PartialEq)]
pub struct AppIdentity {
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub pid: i32,
    pub idle_secs: f64,
    pub is_secure_field: bool,
}

/// Window title and (optional, capped) selection. Raw text: the runtime must
/// pass both through the scrubber before anything else sees them.
#[derive(Debug, Clone, PartialEq)]
pub struct FrontmostContext {
    pub pid: i32,
    pub window_title: Option<String>,
    pub is_secure_field: bool,
    pub selected_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPeek {
    pub change_count: i64,
    /// macOS pasteboard privacy setting (`ask`, `default`, ...), if known.
    pub access_behavior: Option<String>,
}

/// A clipboard read. `text` is absent when the pasteboard is sensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardRead {
    pub change_count: i64,
    pub sensitive: bool,
    pub text: Option<String>,
}

/// Result of one autonomous screen sample.
#[derive(Debug, Clone, PartialEq)]
pub enum ScreenSample {
    Unchanged,
    Text { text: String, ocr_ms: u64 },
}

/// Result of a user-triggered region capture.
#[derive(Debug, Clone, PartialEq)]
pub enum RegionSample {
    Cancelled,
    Text { text: String, ocr_ms: u64 },
}

/// Read-only access to what is on screen. Every method is blocking.
pub trait Sensor: Send + Sync {
    fn platform_supported(&self) -> bool;
    fn accessibility(&self) -> PermissionState;
    fn screen_recording(&self) -> PermissionState;
    /// Shows the system prompt at most once per process (T1 guard).
    fn request_screen_recording(&self) -> PermissionState;
    fn request_accessibility(&self) -> PermissionState;
    /// Warm the Swift helper (first sample may otherwise be slow).
    fn warm_up(&self) {}
    /// Open a System Settings privacy pane (explicit user click only).
    fn open_privacy_pane(&self, _pane: &str) {}
    fn frontmost_app(&self) -> Result<AppIdentity, SensorError>;
    fn frontmost_context(
        &self,
        want_selection: bool,
        max_selection_chars: usize,
    ) -> Result<FrontmostContext, SensorError>;
    fn clipboard_peek(&self) -> Result<ClipboardPeek, SensorError>;
    fn clipboard_read(&self, max_chars: usize) -> Result<ClipboardRead, SensorError>;
    /// Autonomous capture of the frontmost window; OCR only if it changed.
    /// `force` skips change detection (app/window switch).
    fn screen_sample(
        &self,
        expected_pid: Option<i32>,
        force: bool,
        languages: &[String],
    ) -> Result<ScreenSample, SensorError>;
    /// Drop the change-detection baseline (resume, exclusion change).
    fn screen_reset(&self);
    /// User-driven region capture + OCR (the image is deleted at once).
    fn capture_region(
        &self,
        timeout: Duration,
        languages: &[String],
    ) -> Result<RegionSample, SensorError>;
}

fn ocr_opts(languages: &[String]) -> OcrOpts {
    let mut o = OcrOpts::default();
    if !languages.is_empty() {
        o.languages = languages.to_vec();
    }
    o
}

/// Production sensor over `desktop::accessibility`.
pub struct MacSensor {
    watcher: Mutex<ScreenWatcher>,
}

impl Default for MacSensor {
    fn default() -> Self {
        Self {
            watcher: Mutex::new(ScreenWatcher::new(ChangeConfig::default())),
        }
    }
}

impl Sensor for MacSensor {
    fn platform_supported(&self) -> bool {
        cfg!(target_os = "macos")
    }

    fn accessibility(&self) -> PermissionState {
        #[cfg(target_os = "macos")]
        {
            ax::detect_accessibility_permission()
        }
        #[cfg(not(target_os = "macos"))]
        {
            PermissionState::Unsupported
        }
    }

    fn screen_recording(&self) -> PermissionState {
        ax::detect_screen_recording_permission()
    }

    fn request_screen_recording(&self) -> PermissionState {
        ax::request_screen_recording_access()
    }

    fn request_accessibility(&self) -> PermissionState {
        #[cfg(target_os = "macos")]
        {
            ax::request_accessibility_access();
            ax::open_macos_privacy_pane("Privacy_Accessibility");
            ax::detect_accessibility_permission()
        }
        #[cfg(not(target_os = "macos"))]
        {
            PermissionState::Unsupported
        }
    }

    fn warm_up(&self) {
        ax::precompile_helper_background();
    }

    fn open_privacy_pane(&self, _pane: &str) {
        #[cfg(target_os = "macos")]
        ax::open_macos_privacy_pane(_pane);
    }

    fn frontmost_app(&self) -> Result<AppIdentity, SensorError> {
        let s = ax::frontmost_snapshot(SnapshotOpts {
            want_selection: false,
            max_selection_chars: 0,
        })?;
        // `s.window_title` is dropped here, before any exclusion decision can
        // be bypassed: the identity is all that leaves this function.
        Ok(AppIdentity {
            app_name: s.app_name,
            bundle_id: s.bundle_id,
            pid: s.pid,
            idle_secs: s.idle_secs,
            is_secure_field: s.is_secure_field,
        })
    }

    fn frontmost_context(
        &self,
        want_selection: bool,
        max_selection_chars: usize,
    ) -> Result<FrontmostContext, SensorError> {
        let s = ax::frontmost_snapshot(SnapshotOpts {
            want_selection,
            max_selection_chars,
        })?;
        Ok(FrontmostContext {
            pid: s.pid,
            window_title: s.window_title,
            is_secure_field: s.is_secure_field,
            selected_text: s.selected_text,
        })
    }

    fn clipboard_peek(&self) -> Result<ClipboardPeek, SensorError> {
        let p = ax::clipboard_peek()?;
        Ok(ClipboardPeek {
            change_count: p.change_count,
            access_behavior: p.access_behavior,
        })
    }

    fn clipboard_read(&self, max_chars: usize) -> Result<ClipboardRead, SensorError> {
        let r = ax::clipboard_read(max_chars)?;
        let sensitive = r.is_sensitive();
        Ok(ClipboardRead {
            change_count: r.change_count,
            sensitive,
            text: if sensitive { None } else { r.text },
        })
    }

    fn screen_sample(
        &self,
        expected_pid: Option<i32>,
        force: bool,
        languages: &[String],
    ) -> Result<ScreenSample, SensorError> {
        let opts = AutonomousCaptureOpts {
            target: CaptureTarget::ActiveWindow,
            expected_pid,
            block_on_secure_input: true,
            force,
            ocr: ocr_opts(languages),
        };
        let mut w = self
            .watcher
            .lock()
            .map_err(|_| SensorError::Failed("screen watcher poisoned".into()))?;
        match w.observe(&opts)? {
            ScreenObservation::Unchanged { .. } => Ok(ScreenSample::Unchanged),
            ScreenObservation::Text { text, .. } => Ok(ScreenSample::Text {
                ocr_ms: text.ms,
                text: text.text,
            }),
        }
    }

    fn screen_reset(&self) {
        if let Ok(mut w) = self.watcher.lock() {
            w.reset();
        }
    }

    fn capture_region(
        &self,
        timeout: Duration,
        languages: &[String],
    ) -> Result<RegionSample, SensorError> {
        match ax::capture_region_interactive(timeout, &ocr_opts(languages))? {
            RegionCapture::Cancelled => Ok(RegionSample::Cancelled),
            RegionCapture::Text(t) => Ok(RegionSample::Text {
                ocr_ms: t.ms,
                text: t.text,
            }),
        }
    }
}
