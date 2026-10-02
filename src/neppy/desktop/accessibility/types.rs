//! Shared platform types for accessibility, focus, and permissions.

use serde::{Deserialize, Serialize};

/// Unified element bounds — used by autocomplete.
#[derive(Debug, Clone, Copy)]
pub struct ElementBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Context returned by an accessibility focus query.
#[derive(Debug, Clone)]
pub struct FocusedTextContext {
    pub app_name: Option<String>,
    pub role: Option<String>,
    pub text: String,
    pub selected_text: Option<String>,
    pub raw_error: Option<String>,
    pub bounds: Option<ElementBounds>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub accessibility: PermissionState,
    pub input_monitoring: PermissionState,
    pub microphone: PermissionState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Accessibility,
    InputMonitoring,
    Microphone,
}

/// Failure modes shared by the Pet Mode sensors (frontmost snapshot, clipboard,
/// OCR, screen capture). Messages never carry captured content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorError {
    /// Accessibility permission is missing, so no AX read can succeed.
    PermissionMissing,
    /// Screen Recording permission is not granted; the caller decides whether to
    /// request it (once, on first need).
    PermissionRequired,
    /// The sensor is not implemented on this platform.
    Unsupported,
    /// The Swift helper could not be compiled or started (for example no Xcode CLT).
    HelperUnavailable(String),
    /// A native call or helper round-trip exceeded its deadline.
    Timeout,
    /// No application has keyboard focus (or no capturable window exists).
    NoFocusedApp,
    /// A secure field (password entry) is focused or secure event input is on.
    SecureFieldFocused,
    /// The frontmost application changed between the snapshot and the capture.
    TargetChanged,
    /// Anything else, with a content-free description.
    Failed(String),
}

impl std::fmt::Display for SensorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionMissing => write!(f, "accessibility permission missing"),
            Self::PermissionRequired => write!(f, "screen recording permission required"),
            Self::Unsupported => write!(f, "sensor unsupported on this platform"),
            Self::HelperUnavailable(m) => write!(f, "sensor helper unavailable: {m}"),
            Self::Timeout => write!(f, "sensor timed out"),
            Self::NoFocusedApp => write!(f, "no focused application"),
            Self::SecureFieldFocused => write!(f, "secure field focused"),
            Self::TargetChanged => write!(f, "capture target changed"),
            Self::Failed(m) => write!(f, "sensor failed: {m}"),
        }
    }
}

impl std::error::Error for SensorError {}
