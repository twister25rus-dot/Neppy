use chrono::{Duration, TimeZone, Utc};

use super::*;
use crate::neppy::pet::companion::sensitive::{scrub, ScrubCtx};
use crate::neppy::pet::companion::types::ObservationFlags;

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 2, 12, 0, 0).unwrap()
}

fn ev(at: DateTime<Utc>, text: &str) -> ObservationEvent {
    ObservationEvent {
        at,
        kind: ObservationKind::Selection,
        app_name: "Notes".into(),
        bundle_id: Some("com.apple.Notes".into()),
        title: scrub("A title", &ScrubCtx::title()).into_text(),
        text: scrub(text, &ScrubCtx::selection(false).capped(10_000)).into_text(),
        flags: ObservationFlags::default(),
    }
}

#[test]
fn event_count_is_capped() {
    let mut b = ObservationBuffer::new();
    for i in 0..(MAX_EVENTS + 10) {
        b.push(ev(t0() + Duration::seconds(i as i64), "short text"));
    }
    assert_eq!(b.len(), MAX_EVENTS);
    assert_eq!(b.summaries(1000).len(), MAX_SUMMARIES);
}

#[test]
fn text_bytes_are_capped() {
    let mut b = ObservationBuffer::new();
    let big = "lorem ipsum, dolor sit. ".repeat(100); // ~2.4 KiB
    for i in 0..40 {
        b.push(ev(t0() + Duration::seconds(i), &big));
    }
    assert!(b.text_bytes() <= MAX_TEXT_BYTES, "{}", b.text_bytes());
    assert!(b.len() < 40 && b.len() > 5);
}

#[test]
fn ttl_evicts_old_events_and_summaries() {
    let mut b = ObservationBuffer::new();
    b.push(ev(t0(), "old text"));
    b.record_drop(
        t0(),
        ObservationKind::AppSwitch,
        "Bank",
        DropReason::ExcludedApp,
    );
    b.push(ev(t0() + Duration::minutes(TTL_MINUTES + 1), "new text"));
    assert_eq!(b.len(), 1);
    assert_eq!(b.summaries(10).len(), 1);
    b.evict_expired(t0() + Duration::minutes(TTL_MINUTES * 3));
    assert!(b.is_empty());
    assert_eq!(b.text_bytes(), 0);
}

#[test]
fn clear_forgets_everything() {
    let mut b = ObservationBuffer::new();
    b.push(ev(t0(), "text"));
    b.record_drop(
        t0(),
        ObservationKind::Clipboard,
        "App",
        DropReason::ConcealedClipboard,
    );
    assert!(!b.is_empty());
    b.clear();
    assert!(b.is_empty());
    assert_eq!(b.len(), 0);
    assert_eq!(b.text_bytes(), 0);
    assert!(b.summaries(10).is_empty());
    assert!(b.last().is_none());
}

#[test]
fn summaries_are_newest_first_and_include_drops_without_text() {
    let mut b = ObservationBuffer::new();
    b.push(ev(t0(), "hello world body"));
    b.record_drop(
        t0() + Duration::seconds(1),
        ObservationKind::TitleChange,
        "Safari",
        DropReason::TitleRule,
    );
    let s = b.summaries(5);
    assert_eq!(s[0].dropped, Some(DropReason::TitleRule));
    assert!(s[0].title_excerpt.is_none());
    assert_eq!(s[1].title_excerpt.as_deref(), Some("A title"));
    assert!(!serde_json::to_string(&s).unwrap().contains("hello world"));
}
