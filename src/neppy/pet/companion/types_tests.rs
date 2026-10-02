use super::*;
use crate::neppy::pet::companion::sensitive::{scrub, ScrubCtx};

macro_rules! impls_serialize {
    ($t:ty) => {{
        trait Fallback {
            const IS: bool = false;
        }
        impl<T: ?Sized> Fallback for T {}
        struct Probe<T: ?Sized>(std::marker::PhantomData<T>);
        #[allow(dead_code)]
        impl<T: ?Sized + serde::Serialize> Probe<T> {
            const IS: bool = true;
        }
        <Probe<$t>>::IS
    }};
}

#[test]
fn raw_observation_types_have_no_serialize_impl() {
    // PC3: raw observations cannot be persisted or sent by accident.
    assert!(!impls_serialize!(ObservationEvent));
    assert!(!impls_serialize!(
        crate::neppy::pet::companion::sensitive::Scrubbed
    ));
    // The probe itself works: summaries ARE serialisable.
    assert!(impls_serialize!(ObservationSummary));
}

#[test]
fn scrubbed_debug_never_prints_content() {
    let s = scrub("hello visible text", &ScrubCtx::title())
        .into_text()
        .unwrap();
    let dbg = format!("{s:?}");
    assert!(!dbg.contains("hello"), "{dbg}");
    let ev = ObservationEvent {
        at: chrono::Utc::now(),
        kind: ObservationKind::Selection,
        app_name: "Notes".into(),
        bundle_id: None,
        title: Some(s.clone()),
        text: Some(s),
        flags: ObservationFlags::default(),
    };
    assert!(!format!("{ev:?}").contains("hello"));
}

#[test]
fn summary_carries_only_a_title_excerpt() {
    let long = "t".repeat(300);
    let ev = ObservationEvent {
        at: chrono::Utc::now(),
        kind: ObservationKind::TitleChange,
        app_name: "Safari".into(),
        bundle_id: Some("com.apple.Safari".into()),
        title: scrub(&format!("{long} end"), &ScrubCtx::title()).into_text(),
        text: scrub("SECRET-BODY-TEXT here", &ScrubCtx::selection(false)).into_text(),
        flags: ObservationFlags::default(),
    };
    let json = serde_json::to_string(&ev.summary()).unwrap();
    assert!(!json.contains("SECRET-BODY"));
    assert!(ev.summary().title_excerpt.unwrap().chars().count() <= SUMMARY_TITLE_CHARS);
}

#[test]
fn level_serialises_as_a_number() {
    assert_eq!(serde_json::to_string(&CompanionLevel::Assist).unwrap(), "2");
    assert_eq!(
        serde_json::from_str::<CompanionLevel>("3").unwrap(),
        CompanionLevel::Trusted
    );
    assert!(serde_json::from_str::<CompanionLevel>("4").is_err());
    assert!(CompanionLevel::Observe < CompanionLevel::Trusted);
}

#[test]
fn enums_round_trip_their_wire_names() {
    for c in ActionCategory::ALL {
        assert_eq!(ActionCategory::parse(c.as_str()), Some(*c));
        assert_eq!(
            serde_json::to_string(c).unwrap(),
            format!("\"{}\"", c.as_str())
        );
    }
    for k in TriggerKind::ALL {
        assert_eq!(TriggerKind::parse(k.as_str()), Some(*k));
    }
    assert_eq!(
        DropReason::parse("secure_field"),
        Some(DropReason::SecureField)
    );
}

#[test]
fn exactly_the_nine_high_risk_categories_are_high_risk() {
    let high: Vec<&str> = ActionCategory::ALL
        .iter()
        .filter(|c| c.is_high_risk())
        .map(|c| c.as_str())
        .collect();
    assert_eq!(
        high,
        [
            "send_message",
            "delete",
            "purchase",
            "publish",
            "system_settings",
            "install",
            "privileged_command",
            "share_personal_info",
            "irreversible"
        ]
    );
    assert_eq!(ActionCategory::ALL.len(), 16);
    assert!(ActionCategory::HandoffTask.is_medium_risk());
}

#[test]
fn only_three_trigger_kinds_are_proactive() {
    let p: Vec<_> = TriggerKind::ALL
        .iter()
        .filter(|k| k.is_proactive())
        .collect();
    assert_eq!(p.len(), 3);
    assert!(!TriggerKind::Ask.is_proactive() && !TriggerKind::Capture.is_proactive());
}
