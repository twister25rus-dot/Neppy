use chrono::Utc;

use super::*;
use crate::neppy::pet::companion::sensitive::{scrub, ScrubCtx};
use crate::neppy::pet::companion::types::ObservationFlags;

const TERMINAL: &str = "com.apple.Terminal";
const SAFARI: &str = "com.apple.Safari";

fn event(
    kind: ObservationKind,
    bundle: &str,
    title: Option<&str>,
    text: Option<&str>,
    user: bool,
) -> ObservationEvent {
    ObservationEvent {
        at: Utc::now(),
        kind,
        app_name: "App".into(),
        bundle_id: Some(bundle.into()),
        title: title.and_then(|t| scrub(t, &ScrubCtx::title()).into_text()),
        text: text.and_then(|t| scrub(t, &ScrubCtx::selection(false)).into_text()),
        flags: ObservationFlags {
            redacted: false,
            user_initiated: user,
        },
    }
}

fn kind_of(ev: &ObservationEvent) -> Option<TriggerKind> {
    detect_event(ev).map(|d| d.kind)
}

const RUSTC: &str = "error[E0308]: mismatched types\n --> src/main.rs:4:5";

#[test]
fn build_error_in_terminal_but_not_in_notes() {
    let t = event(
        ObservationKind::Selection,
        TERMINAL,
        None,
        Some(RUSTC),
        false,
    );
    assert_eq!(kind_of(&t), Some(TriggerKind::BuildError));
    let n = event(
        ObservationKind::Selection,
        "com.apple.Notes",
        None,
        Some(RUSTC),
        false,
    );
    assert_eq!(kind_of(&n), None);
}

#[test]
fn every_error_shape_is_recognised() {
    for text in [
        "Traceback (most recent call last):\n  File x",
        "npm ERR! code ELIFECYCLE",
        "FAILED tests/test_a.py::t",
        "** BUILD FAILED **",
        "fatal error: 'x.h' file not found",
        "src/a.ts(1,2): error TS2304: Cannot find name",
        "Exception in thread \"main\" java.lang.Foo",
        "thread 'main' panicked at src/lib.rs:3",
        "cannot find module 'left-pad'",
    ] {
        let ev = event(
            ObservationKind::Clipboard,
            "com.microsoft.VSCode",
            None,
            Some(text),
            false,
        );
        assert_eq!(kind_of(&ev), Some(TriggerKind::BuildError), "{text}");
    }
}

#[test]
fn build_error_from_autonomous_ocr_capture() {
    let ev = event(ObservationKind::Capture, TERMINAL, None, Some(RUSTC), false);
    assert_eq!(kind_of(&ev), Some(TriggerKind::BuildError));
    // Same OCR text in a non-dev app: no trigger.
    let ev = event(
        ObservationKind::Capture,
        "com.apple.Notes",
        None,
        Some(RUSTC),
        false,
    );
    assert_eq!(kind_of(&ev), None);
    // Plain terminal text without an error: no trigger.
    let ev = event(
        ObservationKind::Capture,
        TERMINAL,
        None,
        Some("ls -la\ntotal 8"),
        false,
    );
    assert_eq!(kind_of(&ev), None);
}

#[test]
fn manual_capture_and_ask_always_trigger() {
    let cap = event(
        ObservationKind::Capture,
        "com.apple.Notes",
        None,
        Some("anything"),
        true,
    );
    assert_eq!(kind_of(&cap), Some(TriggerKind::Capture));
    let ask = event(ObservationKind::Ask, "com.apple.Notes", None, None, true);
    assert_eq!(kind_of(&ask), Some(TriggerKind::Ask));
}

#[test]
fn app_switch_and_title_change_alone_never_trigger() {
    for k in [ObservationKind::AppSwitch, ObservationKind::TitleChange] {
        assert_eq!(kind_of(&event(k, TERMINAL, Some("zsh"), None, false)), None);
    }
}

#[test]
fn email_draft_needs_mail_context_and_a_long_selection() {
    let sel = "Hi Sam, thanks for the update on the release schedule.";
    let gmail = event(
        ObservationKind::Selection,
        SAFARI,
        Some("Compose - Gmail"),
        Some(sel),
        false,
    );
    assert_eq!(kind_of(&gmail), Some(TriggerKind::EmailDraft));
    let mail = event(
        ObservationKind::Selection,
        "com.apple.mail",
        None,
        Some(sel),
        false,
    );
    assert_eq!(kind_of(&mail), Some(TriggerKind::EmailDraft));
    let short = event(
        ObservationKind::Selection,
        SAFARI,
        Some("Compose - Gmail"),
        Some("short"),
        false,
    );
    assert_eq!(kind_of(&short), None);
    let news = event(
        ObservationKind::Selection,
        SAFARI,
        Some("Daily news"),
        Some(sel),
        false,
    );
    assert_eq!(kind_of(&news), None);
    let re = event(
        ObservationKind::Selection,
        SAFARI,
        Some("Re: budget"),
        Some(sel),
        false,
    );
    assert_eq!(kind_of(&re), Some(TriggerKind::EmailDraft));
}

#[test]
fn term_needs_a_scholarly_title_and_a_short_selection() {
    let arxiv = event(
        ObservationKind::Selection,
        SAFARI,
        Some("arXiv:2401.00001 - A paper"),
        Some("Kullback\u{2013}Leibler divergence"),
        false,
    );
    assert_eq!(kind_of(&arxiv), Some(TriggerKind::Term));
    let pdf = event(
        ObservationKind::Selection,
        "com.apple.Preview",
        Some("notes.pdf"),
        Some("eigenvalue"),
        false,
    );
    assert_eq!(kind_of(&pdf), Some(TriggerKind::Term));
    let long = event(
        ObservationKind::Selection,
        SAFARI,
        Some("arXiv paper"),
        Some("one two three four five six seven"),
        false,
    );
    assert_eq!(kind_of(&long), None);
    let other = event(
        ObservationKind::Selection,
        SAFARI,
        Some("Cat videos"),
        Some("divergence"),
        false,
    );
    assert_eq!(kind_of(&other), None);
}

#[test]
fn classify_apps() {
    assert_eq!(
        classify_app(Some("com.googlecode.iterm2")),
        AppClass::Terminal
    );
    assert_eq!(classify_app(Some("com.jetbrains.rustrover")), AppClass::Ide);
    assert_eq!(classify_app(Some("com.apple.mail")), AppClass::Mail);
    assert_eq!(classify_app(Some("COM.GOOGLE.CHROME")), AppClass::Browser);
    assert_eq!(classify_app(Some("com.apple.Preview")), AppClass::Reader);
    assert_eq!(classify_app(None), AppClass::Other);
}

#[test]
fn fingerprint_is_stable_across_numbers_and_whitespace_but_not_across_apps_or_kinds() {
    let a = fingerprint(
        TriggerKind::BuildError,
        Some(TERMINAL),
        "error[E0308]: x\n --> a.rs:4:5",
    );
    let b = fingerprint(
        TriggerKind::BuildError,
        Some(TERMINAL),
        "ERROR[E0308]:   x --> a.rs:99:12",
    );
    assert_eq!(a, b);
    assert_ne!(
        a,
        fingerprint(
            TriggerKind::BuildError,
            Some("com.apple.Notes"),
            "error[E0308]: x\n --> a.rs:4:5"
        )
    );
    assert_ne!(
        a,
        fingerprint(
            TriggerKind::Term,
            Some(TERMINAL),
            "error[E0308]: x\n --> a.rs:4:5"
        )
    );
    assert_eq!(a.len(), 64);
}

#[test]
fn build_error_fingerprint_ignores_text_before_the_error() {
    let one = event(
        ObservationKind::Capture,
        TERMINAL,
        None,
        Some(&format!("noise one\n{RUSTC}")),
        false,
    );
    let two = event(
        ObservationKind::Capture,
        TERMINAL,
        None,
        Some(&format!("other noise\n{RUSTC}")),
        false,
    );
    assert_eq!(
        detect_event(&one).unwrap().fingerprint,
        detect_event(&two).unwrap().fingerprint
    );
}
