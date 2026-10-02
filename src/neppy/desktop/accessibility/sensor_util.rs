//! Shared plumbing for the Pet Mode sensors that go through the Swift helper.
// Parts are only reached from macOS-only code paths; the rest is tested everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::types::SensorError;
#[cfg(target_os = "macos")]
use std::time::Duration;

/// Deadline for cheap helper calls (clipboard, window lookup, signature).
#[cfg(target_os = "macos")]
pub(super) const SENSOR_TIMEOUT: Duration = Duration::from_millis(1_500);
/// Deadline for an OCR pass.
#[cfg(target_os = "macos")]
pub(super) const OCR_TIMEOUT: Duration = Duration::from_secs(15);

/// Classify a helper error string. Strings never carry captured content.
pub(super) fn map_helper_error(message: String) -> SensorError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("timed out") || lower.contains("helper busy") {
        SensorError::Timeout
    } else if lower.contains("failed to compile")
        || lower.contains("failed to invoke swiftc")
        || lower.contains("failed to spawn")
        || lower.contains("helper unavailable")
        || lower.contains("failed to create cache dir")
        || lower.contains("failed to write helper source")
    {
        SensorError::HelperUnavailable(message)
    } else {
        SensorError::Failed(message)
    }
}

/// One helper round trip with a deadline, error-mapped.
#[cfg(target_os = "macos")]
pub(super) fn call_helper(
    request: &serde_json::Value,
    timeout: Duration,
) -> Result<serde_json::Value, SensorError> {
    let resp = super::helper::helper_send_receive_with_timeout(request, timeout)
        .map_err(map_helper_error)?;
    // The helper reports per-command failures as a short code in `error`.
    if let Some(err) = resp.get("error").and_then(|v| v.as_str()) {
        return Err(SensorError::Failed(err.to_string()));
    }
    Ok(resp)
}
