use super::*;

fn flat(v: u8) -> FrameSignature {
    FrameSignature {
        cols: SIGNATURE_COLS,
        rows: SIGNATURE_ROWS,
        cells: vec![v; SIGNATURE_COLS as usize * SIGNATURE_ROWS as usize],
    }
}

#[test]
fn first_frame_is_changed() {
    let d = ChangeDetector::default();
    assert_eq!(d.compare(&flat(10)), Change::First);
    assert!(Change::First.should_ocr());
}

#[test]
fn identical_frames_are_unchanged() {
    let mut d = ChangeDetector::default();
    d.commit(flat(100));
    assert_eq!(
        d.compare(&flat(100)),
        Change::Unchanged { changed_cells: 0 }
    );
}

#[test]
fn tiny_difference_is_unchanged_large_is_changed() {
    let mut d = ChangeDetector::default();
    d.commit(flat(100));
    let mut few = flat(100);
    for c in few.cells.iter_mut().take(3) {
        *c = 200; // 3 cells = 0.13%
    }
    assert!(!d.compare(&few).should_ocr());
    let mut many = flat(100);
    for c in many.cells.iter_mut().take(40) {
        *c = 200;
    }
    assert!(matches!(d.compare(&many), Change::Changed { .. }));
}

#[test]
fn slow_drift_accumulates_against_last_committed_frame() {
    let mut d = ChangeDetector::default();
    d.commit(flat(100));
    // +1 per step never trips a step-to-step check, but crosses mean 1.5 vs baseline.
    assert!(!d.compare(&flat(101)).should_ocr());
    assert!(d.compare(&flat(102)).should_ocr());
}

#[test]
fn compare_does_not_move_baseline_and_reset_clears_it() {
    let mut d = ChangeDetector::default();
    d.commit(flat(0));
    let _ = d.compare(&flat(255));
    assert_eq!(d.compare(&flat(0)), Change::Unchanged { changed_cells: 0 });
    d.reset();
    assert!(!d.has_baseline());
    assert_eq!(d.compare(&flat(0)), Change::First);
}

#[test]
fn shape_mismatch_is_changed() {
    let mut d = ChangeDetector::default();
    d.commit(flat(5));
    let other = FrameSignature {
        cols: 2,
        rows: 2,
        cells: vec![5; 4],
    };
    assert!(d.compare(&other).should_ocr());
}

#[test]
fn hex_roundtrip_and_validation() {
    let sig = FrameSignature::from_hex(2, 2, "00ff10Ab").unwrap();
    assert_eq!(sig.cells, vec![0x00, 0xff, 0x10, 0xab]);
    assert!(FrameSignature::from_hex(2, 2, "00ff").is_err());
    assert!(FrameSignature::from_hex(2, 2, "zzzzzzzz").is_err());
    assert!(FrameSignature::from_hex(0, 2, "").is_err());
}
