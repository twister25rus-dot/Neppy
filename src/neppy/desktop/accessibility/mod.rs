//! Platform accessibility middleware: focus queries and permission management.
//!
//! Centralises macOS AX/IOKit FFI and the unified Swift helper process.
//! Voice services call into this module instead of owning platform-specific
//! code directly.

mod automation_state;
mod capture;
mod change_detector;
mod clipboard;
mod focus;
mod frontmost;
mod globe;
mod helper;
mod helper_sensors_swift;
mod ocr;
mod permissions;
mod sensor_util;
mod terminal;
mod text_util;
mod types;

pub use automation_state::{
    clear as clear_automation_denial, mark_system_events_denied, system_events_denied,
};
pub use capture::{
    capture_region_interactive, AutonomousCaptureOpts, CaptureTarget, RegionCapture,
    ScreenObservation, ScreenWatcher,
};
pub use change_detector::{
    Change, ChangeConfig, ChangeDetector, FrameSignature, SIGNATURE_COLS, SIGNATURE_ROWS,
};
pub use clipboard::{
    clipboard_peek, clipboard_read, ClipboardPeek, ClipboardRead, SENSITIVE_PASTEBOARD_TYPES,
};
pub use focus::{focused_text_context, focused_text_context_verbose, validate_focused_target};
pub use frontmost::{
    bundle_dir_from_exe_path, frontmost_pid, frontmost_snapshot, secure_entry_active,
    FrontmostSnapshot, SnapshotOpts,
};
pub use globe::{
    globe_listener_poll, globe_listener_start, globe_listener_stop, GlobeHotkeyPollResult,
    GlobeHotkeyStatus,
};
pub use helper::precompile_helper_background;
pub use ocr::{OcrLevel, OcrOpts, OcrText};
#[cfg(target_os = "macos")]
pub use permissions::{
    detect_accessibility_permission, detect_input_monitoring_permission, open_macos_privacy_pane,
    request_accessibility_access,
};
pub use permissions::{
    detect_microphone_permission, detect_permissions, detect_screen_recording_permission,
    microphone_denied_message, permission_to_str, request_microphone_access,
    request_screen_recording_access,
};
pub use terminal::{
    extract_terminal_input_context, is_terminal_app, is_text_role, looks_like_terminal_buffer,
};
pub use text_util::{normalize_ax_value, parse_ax_number, truncate_tail};
pub use types::{
    ElementBounds, FocusedTextContext, PermissionKind, PermissionState, PermissionStatus,
    SensorError,
};
