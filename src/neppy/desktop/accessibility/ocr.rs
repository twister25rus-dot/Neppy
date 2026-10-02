//! On-device OCR and frame signatures (Apple Vision, in the Swift helper).
//!
//! These take a path to an image the caller owns (a [`super::capture::TempPng`])
//! and return **text or a downscaled grid only**. They are `pub(super)` on
//! purpose: the public surface is the capture functions, which delete the image
//! right after reading it.
// Parts are only reached from macOS-only code paths; the rest is tested everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::change_detector::FrameSignature;
use super::types::SensorError;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrLevel {
    Fast,
    Accurate,
}

impl OcrLevel {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            OcrLevel::Fast => "fast",
            OcrLevel::Accurate => "accurate",
        }
    }
}

/// Recognition options. Defaults: accurate, English, long edge capped at 1600 px.
/// (`automaticallyDetectsLanguage` was measured ~19 s on a Retina frame vs ~0.2 s with
/// an explicit language list, so languages are always explicit.)
#[derive(Debug, Clone)]
pub struct OcrOpts {
    pub max_chars: usize,
    pub level: OcrLevel,
    pub languages: Vec<String>,
    pub max_dim: u32,
}

impl Default for OcrOpts {
    fn default() -> Self {
        Self {
            max_chars: 4_000,
            level: OcrLevel::Accurate,
            languages: vec!["en-US".to_string()],
            max_dim: 1_600,
        }
    }
}

/// Recognised text. The only thing that ever leaves a capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OcrText {
    pub text: String,
    pub ms: u64,
    pub lines: usize,
    pub truncated: bool,
}

pub(super) fn parse_ocr(v: &serde_json::Value, max_chars: usize) -> Result<OcrText, SensorError> {
    let text = v
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| SensorError::Failed("ocr: missing text".into()))?;
    let mut truncated = v
        .get("truncated")
        .and_then(|b| b.as_bool())
        .unwrap_or(false);
    if text.chars().count() > max_chars {
        truncated = true;
    }
    Ok(OcrText {
        text: text.chars().take(max_chars).collect(),
        ms: v.get("ms").and_then(|m| m.as_u64()).unwrap_or(0),
        lines: v.get("lines").and_then(|l| l.as_u64()).unwrap_or(0) as usize,
        truncated,
    })
}

pub(super) fn parse_signature(v: &serde_json::Value) -> Result<FrameSignature, SensorError> {
    let cols = v.get("cols").and_then(|c| c.as_u64()).unwrap_or(0) as u16;
    let rows = v.get("rows").and_then(|c| c.as_u64()).unwrap_or(0) as u16;
    let hex = v
        .get("signature")
        .and_then(|s| s.as_str())
        .ok_or_else(|| SensorError::Failed("frame_signature: missing signature".into()))?;
    FrameSignature::from_hex(cols, rows, hex).map_err(SensorError::Failed)
}

#[cfg(target_os = "macos")]
pub(super) fn ocr_png(path: &Path, opts: &OcrOpts) -> Result<OcrText, SensorError> {
    let resp = super::sensor_util::call_helper(
        &serde_json::json!({
            "type": "ocr",
            "path": path.to_string_lossy(),
            "max_chars": opts.max_chars,
            "level": opts.level.as_str(),
            "languages": opts.languages,
            "max_dim": opts.max_dim,
        }),
        super::sensor_util::OCR_TIMEOUT,
    )?;
    let out = parse_ocr(&resp, opts.max_chars)?;
    log::debug!(
        "[accessibility][sensors] ocr ms={} lines={} chars={} truncated={}",
        out.ms,
        out.lines,
        out.text.chars().count(),
        out.truncated
    );
    Ok(out)
}

#[cfg(target_os = "macos")]
pub(super) fn frame_signature(path: &Path) -> Result<FrameSignature, SensorError> {
    use super::change_detector::{SIGNATURE_COLS, SIGNATURE_ROWS};
    let resp = super::sensor_util::call_helper(
        &serde_json::json!({
            "type": "frame_signature",
            "path": path.to_string_lossy(),
            "cols": SIGNATURE_COLS,
            "rows": SIGNATURE_ROWS,
        }),
        super::sensor_util::SENSOR_TIMEOUT.max(std::time::Duration::from_secs(3)),
    )?;
    parse_signature(&resp)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn ocr_png(_path: &Path, _opts: &OcrOpts) -> Result<OcrText, SensorError> {
    Err(SensorError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
pub(super) fn frame_signature(_path: &Path) -> Result<FrameSignature, SensorError> {
    Err(SensorError::Unsupported)
}

#[cfg(test)]
#[path = "ocr_tests.rs"]
mod tests;
