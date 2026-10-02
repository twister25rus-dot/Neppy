//! Cheap frame change detection so on-device OCR runs only when the screen changed.
//!
//! The Swift helper reduces a captured frame to a small grayscale grid
//! ([`FrameSignature`], 64x36 by default). The detector compares each new grid
//! with the grid of the **last frame that was OCR'd**, so slow drift (typing a
//! line at a time) accumulates until it crosses the threshold instead of being
//! masked by frame-to-frame similarity. Pure logic, no platform dependency.

/// Grid used for signatures (columns x rows).
pub const SIGNATURE_COLS: u16 = 64;
pub const SIGNATURE_ROWS: u16 = 36;

/// Downscaled grayscale frame. Contains no readable content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameSignature {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<u8>,
}

impl FrameSignature {
    /// Decode the helper's lowercase-hex payload; rejects a length mismatch.
    pub fn from_hex(cols: u16, rows: u16, hex: &str) -> Result<Self, String> {
        let expected = cols as usize * rows as usize;
        if cols == 0 || rows == 0 || hex.len() != expected * 2 {
            return Err(format!(
                "signature length {} does not match {cols}x{rows}",
                hex.len()
            ));
        }
        let bytes = hex.as_bytes();
        let nib = |c: u8| -> Result<u8, String> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => Err("signature is not hex".to_string()),
            }
        };
        let mut cells = Vec::with_capacity(expected);
        for pair in bytes.chunks_exact(2) {
            cells.push((nib(pair[0])? << 4) | nib(pair[1])?);
        }
        Ok(Self { cols, rows, cells })
    }
}

/// Thresholds. Defaults: a cell counts as changed when it moves by >= 10/255; a
/// frame changed when >= 0.4% of cells (about 9 of 2304) changed or the mean
/// absolute delta is >= 1.5. A blinking caret or a clock tick stays below that;
/// a scroll, page load or window switch is far above.
#[derive(Debug, Clone, Copy)]
pub struct ChangeConfig {
    pub cell_delta: u8,
    pub min_changed_fraction: f32,
    pub min_mean_delta: f32,
}

impl Default for ChangeConfig {
    fn default() -> Self {
        Self {
            cell_delta: 10,
            min_changed_fraction: 0.004,
            min_mean_delta: 1.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    /// No previous frame: treat as changed.
    First,
    /// Identical or below threshold: skip OCR.
    Unchanged { changed_cells: usize },
    Changed {
        changed_cells: usize,
        mean_delta: f32,
    },
}

impl Change {
    pub fn should_ocr(&self) -> bool {
        !matches!(self, Change::Unchanged { .. })
    }
}

#[derive(Debug, Default)]
pub struct ChangeDetector {
    last: Option<FrameSignature>,
    cfg: ChangeConfig,
}

impl ChangeDetector {
    pub fn new(cfg: ChangeConfig) -> Self {
        Self { last: None, cfg }
    }

    /// Forget the baseline (app/window switch, resume after pause).
    pub fn reset(&mut self) {
        self.last = None;
    }

    pub fn has_baseline(&self) -> bool {
        self.last.is_some()
    }

    /// Compare against the baseline without changing it.
    pub fn compare(&self, sig: &FrameSignature) -> Change {
        let Some(last) = &self.last else {
            return Change::First;
        };
        if last.cols != sig.cols || last.rows != sig.rows || last.cells.len() != sig.cells.len() {
            return Change::Changed {
                changed_cells: sig.cells.len(),
                mean_delta: 255.0,
            };
        }
        if last.cells == sig.cells {
            return Change::Unchanged { changed_cells: 0 };
        }
        let mut changed = 0usize;
        let mut total: u64 = 0;
        for (a, b) in last.cells.iter().zip(sig.cells.iter()) {
            let d = a.abs_diff(*b);
            total += d as u64;
            if d >= self.cfg.cell_delta {
                changed += 1;
            }
        }
        let n = sig.cells.len().max(1);
        let mean = total as f32 / n as f32;
        let frac = changed as f32 / n as f32;
        if frac >= self.cfg.min_changed_fraction || mean >= self.cfg.min_mean_delta {
            Change::Changed {
                changed_cells: changed,
                mean_delta: mean,
            }
        } else {
            Change::Unchanged {
                changed_cells: changed,
            }
        }
    }

    /// Adopt `sig` as the new baseline. Call only after the frame was OCR'd
    /// successfully, so a failed OCR is retried on the next sample.
    pub fn commit(&mut self, sig: FrameSignature) {
        self.last = Some(sig);
    }
}

#[cfg(test)]
#[path = "change_detector_tests.rs"]
mod tests;
