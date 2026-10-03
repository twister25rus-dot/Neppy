//! Sampler gating (lease, pause, disabled), exclusions before reads, secure
//! fields, clipboard and autonomous screen capture, with fakes.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::metrics::Metrics;
use super::state::LEASE_TTL;
use super::test_support::*;
use crate::neppy::desktop::accessibility::PermissionState;
use crate::neppy::pet::companion::types::{DropReason, ObservationKind};

#[test]
fn disabled_companion_never_samples() {
    let h = harness();
    h.lease();
    h.sample();
    assert_eq!(h.sensor.calls().len(), 0);
    assert_eq!(h.rt.state(), ("off", None));
}

#[test]
fn enabled_without_lease_is_suspended_and_reads_nothing() {
    let h = harness();
    h.enable();
    h.sample();
    assert!(h.sensor.calls().is_empty());
    assert_eq!(h.rt.state(), ("suspended", Some("no_indicator")));
    assert_eq!(Metrics::get(&h.rt.metrics.samples_total), 0);
}

#[test]
fn lease_enables_sampling_and_lapses_after_six_seconds() {
    let h = harness();
    h.enable();
    h.lease();
    h.sample();
    assert_eq!(h.sensor.count("app"), 1);
    assert_eq!(h.rt.state().0, "observing");
    // Renewal keeps it alive; without one it lapses within LEASE_TTL (6 s).
    h.clock.advance(LEASE_TTL - Duration::from_millis(1));
    assert!(h.rt.lease_valid());
    h.clock.advance(Duration::from_millis(2));
    assert!(!h.rt.lease_valid());
    h.sample();
    assert_eq!(h.sensor.count("app"), 1, "no sample after the lease lapsed");
    assert_eq!(h.rt.state(), ("suspended", Some("no_indicator")));
    // An invisible indicator revokes the lease at once.
    h.lease();
    h.rt.grant_lease(false);
    h.sample();
    assert_eq!(h.sensor.count("app"), 1);
}

#[test]
fn unsupported_platform_never_samples() {
    let h = harness();
    h.sensor.supported.store(false, Ordering::SeqCst);
    h.enable();
    h.lease();
    h.sample();
    assert!(h.sensor.calls().is_empty());
    assert_eq!(h.rt.state(), ("suspended", Some("unsupported_platform")));
}

#[test]
fn sampler_thread_stops_sampling_when_the_lease_lapses() {
    let h = harness();
    h.patch(serde_json::json!({ "enabled": true }));
    h.lease();
    h.rt.wake_sampler();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while h.sensor.count("app") == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        h.sensor.count("app") > 0,
        "the thread samples under a lease"
    );
    h.clock.advance(LEASE_TTL + Duration::from_millis(1));
    std::thread::sleep(Duration::from_millis(50));
    let after = h.sensor.count("app");
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        h.sensor.count("app"),
        after,
        "no samples once the lease lapsed"
    );
    h.patch(serde_json::json!({ "enabled": false }));
    assert!(!h.rt.sampler_running());
}

#[test]
fn pause_stops_reads_clears_buffer_and_drops_in_flight_results() {
    let h = harness();
    h.enable();
    h.lease();
    h.sample();
    assert!(!h.rt.lock().buffer.is_empty());
    let before = h.sensor.calls().len();
    // A pause that lands DURING a sensor call: the result is discarded.
    let rt = h.rt.clone();
    *h.sensor.on_app.lock().unwrap() = Some(Box::new(move || rt.pause(None, "hotkey")));
    h.sensor.set_app("Notes", "com.apple.Notes", 200);
    h.sample();
    assert_eq!(
        h.sensor.calls().len(),
        before + 1,
        "only the in-flight call ran"
    );
    assert!(h.rt.lock().buffer.is_empty(), "pause clears the buffer");
    *h.sensor.on_app.lock().unwrap() = None;
    for _ in 0..3 {
        h.sample();
    }
    assert_eq!(h.sensor.calls().len(), before + 1, "no reads while paused");
    assert_eq!(h.rt.state().0, "paused");
    // Pause survives a restart (persisted).
    assert!(super::state::load_pause(&h.config).unwrap().0);
    h.rt.resume("ui");
    assert!(!super::state::load_pause(&h.config).unwrap().0);
    h.sample();
    assert!(h.sensor.calls().len() > before + 1);
}

#[test]
fn timed_pause_resumes_by_itself() {
    let h = harness();
    h.enable();
    h.lease();
    h.rt.pause(Some(5), "ui");
    assert_eq!(h.rt.state().0, "paused");
    h.clock.advance(Duration::from_secs(5 * 60 + 1));
    h.lease();
    assert_eq!(h.rt.state().0, "observing");
}

#[test]
fn excluded_app_is_dropped_before_title_or_selection_reads() {
    let h = harness();
    h.enable();
    h.lease();
    h.sensor
        .set_app("1Password", "com.1password.1password", 300);
    *h.sensor.selection.lock().unwrap() = Some("hunter2 secret".into());
    h.sample();
    assert_eq!(
        h.sensor.calls(),
        vec!["app".to_string()],
        "no title/selection read"
    );
    assert_eq!(h.rt.metrics.drops(DropReason::ExcludedApp), 1);
    let recent = h.rt.lock().buffer.summaries(5);
    assert_eq!(recent[0].dropped, Some(DropReason::ExcludedApp));
    // The lock screen is always excluded, whatever the lists say.
    h.patch(serde_json::json!({ "excluded_apps": [] }));
    h.sensor.set_app("loginwindow", "com.apple.loginwindow", 1);
    h.sample();
    assert_eq!(h.sensor.count("context"), 0);
    // Repeated samples of the same excluded app are counted once.
    h.sample();
    assert_eq!(h.rt.metrics.drops(DropReason::ExcludedApp), 2);
}

#[test]
fn title_rule_drops_the_window_and_resets_the_screen_baseline() {
    let h = harness();
    h.enable();
    h.lease();
    *h.sensor.title.lock().unwrap() = Some("Bank - Private Browsing".into());
    *h.sensor.selection.lock().unwrap() = Some("account 1234".into());
    let resets = h.sensor.resets.load(Ordering::SeqCst);
    h.sample();
    assert_eq!(h.rt.metrics.drops(DropReason::TitleRule), 1);
    assert_eq!(
        h.sensor.count("peek"),
        0,
        "nothing else is read in an excluded window"
    );
    assert_eq!(h.sensor.count("screen"), 0);
    assert!(h.sensor.resets.load(Ordering::SeqCst) > resets);
    assert!(h.rt.lock().buffer.summaries(5)[0].title_excerpt.is_none());
}

#[test]
fn secure_field_means_no_selection_read_and_no_capture() {
    let h = harness();
    h.enable();
    h.lease();
    h.sensor.secure_field.store(true, Ordering::SeqCst);
    *h.sensor.selection.lock().unwrap() = Some("my password".into());
    h.sample();
    assert_eq!(h.sensor.count("context+selection"), 0);
    assert_eq!(h.sensor.count("screen"), 0);
    assert_eq!(h.rt.metrics.drops(DropReason::SecureField), 1);
}

#[test]
fn concealed_clipboard_is_dropped_and_baseline_skips_preexisting_content() {
    let h = harness();
    h.enable();
    h.lease();
    h.sensor.copy("copied before the companion started");
    h.sample();
    assert_eq!(
        h.sensor.count("read"),
        0,
        "the first peek only sets a baseline"
    );
    h.sensor.clip_sensitive.store(true, Ordering::SeqCst);
    h.sensor.copy("Hunter2Secret!");
    h.sample();
    assert_eq!(h.sensor.count("read"), 1);
    assert_eq!(h.rt.metrics.drops(DropReason::ConcealedClipboard), 1);
    assert!(h
        .rt
        .lock()
        .buffer
        .events()
        .all(|e| e.kind != ObservationKind::Clipboard));
}

#[test]
fn clipboard_switches_to_on_demand_when_macos_would_ask() {
    let h = harness();
    h.enable();
    h.lease();
    *h.sensor.access_behavior.lock().unwrap() = Some("ask".into());
    h.sample();
    h.sensor.copy("error[E0308]: mismatched types");
    h.sample();
    assert_eq!(h.sensor.count("peek"), 1);
    assert_eq!(h.sensor.count("read"), 0);
}

#[test]
fn clipboard_copied_during_a_pause_is_never_read() {
    let h = harness();
    h.enable();
    h.lease();
    h.sample();
    h.rt.pause(None, "ui");
    h.sensor.copy("4111 1111 1111 1111 paused");
    h.rt.resume("ui");
    h.sample();
    assert_eq!(
        h.sensor.count("read"),
        0,
        "resume re-baselines the clipboard"
    );
}

#[test]
fn fifty_app_switches_produce_summaries_but_no_model_call() {
    let h = harness();
    h.enable();
    h.lease();
    for i in 0..50 {
        let (name, bundle) = if i % 2 == 0 {
            ("Finder", "com.apple.finder")
        } else {
            ("TextEdit", "com.apple.TextEdit")
        };
        h.sensor.set_app(name, bundle, 1000 + i);
        h.sample();
    }
    assert_eq!(Metrics::get(&h.rt.metrics.llm_calls), 0);
    assert_eq!(Metrics::get(&h.rt.metrics.suggestions_created), 0);
    assert!(h.rt.lock().buffer.summaries(50).len() >= 20);
    assert!(h.generator.prompts.lock().unwrap().is_empty());
}

#[test]
fn screen_permission_is_requested_once_then_capture_runs_on_change() {
    let h = harness();
    h.enable();
    h.lease();
    *h.sensor.screen_perm.lock().unwrap() = PermissionState::Denied;
    h.sample();
    h.sample();
    assert_eq!(
        h.sensor.screen_requests.load(Ordering::SeqCst),
        1,
        "asked once"
    );
    assert_eq!(h.sensor.count("screen"), 0);
    assert!(!h.rt.screen_capture_active());

    *h.sensor.screen_perm.lock().unwrap() = PermissionState::Granted;
    h.sensor.set_app("Terminal", "com.apple.Terminal", 101);
    let mut ui = super::bus::subscribe_companion_events();
    h.sample();
    // W7: arming is reported at once (the indicator shows "observing screen")
    // and the first frame waits for the next sample.
    assert!(h.rt.screen_capture_active());
    assert_eq!(h.sensor.count("screen"), 0, "armed, not yet captured");
    let mut saw_armed = false;
    while let Ok(ev) = ui.try_recv() {
        if matches!(
            ev,
            super::bus::CompanionUiEvent::State {
                screen_capture_active: true,
                ..
            }
        ) {
            saw_armed = true;
        }
    }
    assert!(saw_armed, "arming publishes the state");
    h.sample();
    assert_eq!(
        h.sensor.count("screen"),
        1,
        "the first capture runs on the sample after arming"
    );
    assert!(h.rt.screen_capture_active());
    h.sample();
    assert_eq!(
        h.sensor.count("screen"),
        1,
        "rate limited by screen_min_interval_secs"
    );
    h.clock.advance(Duration::from_secs(31));
    h.lease();
    *h.sensor.screen_text.lock().unwrap() = Some("error[E0308]: mismatched types".into());
    h.sample();
    assert_eq!(h.sensor.count("screen"), 2);
    assert_eq!(Metrics::get(&h.rt.metrics.ocr_calls), 1);
    assert!(h
        .rt
        .lock()
        .buffer
        .events()
        .any(|e| e.kind == ObservationKind::Capture));
    // Paused: no capture, indicator off.
    h.rt.pause(None, "ui");
    assert!(!h.rt.screen_capture_active());
}

#[test]
fn sensitive_ocr_text_never_becomes_an_event() {
    let h = harness();
    h.enable();
    h.lease();
    *h.sensor.screen_text.lock().unwrap() =
        Some("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----".into());
    h.sample(); // arms screen capture
    h.sample();
    assert_eq!(Metrics::get(&h.rt.metrics.ocr_calls), 1);
    assert!(h
        .rt
        .lock()
        .buffer
        .events()
        .all(|e| e.kind != ObservationKind::Capture));
    assert_eq!(h.rt.metrics.drops(DropReason::SensitiveContent), 1);
}

/// W6: the frontmost app and the exclusions are re-checked after OCR — text
/// OCR'd while the user switched to another app, or to an excluded window of
/// the same app, never becomes an event.
#[test]
fn ocr_text_is_dropped_when_the_window_changes_during_ocr() {
    let h = harness();
    h.enable();
    h.lease();
    h.sample(); // arms screen capture

    // Another app came to the front while the frame was OCR'd.
    *h.sensor.screen_text.lock().unwrap() = Some("error[E0308]: mismatched types".into());
    *h.sensor.app_after_screen.lock().unwrap() = Some(app("Messages", "com.apple.MobileSMS", 4242));
    h.sample();
    assert_eq!(Metrics::get(&h.rt.metrics.ocr_calls), 1);
    assert!(h
        .rt
        .lock()
        .buffer
        .events()
        .all(|e| e.kind != ObservationKind::Capture));

    // Same app, but the window now matches an exclusion rule.
    h.sensor.set_app("Terminal", "com.apple.Terminal", 100);
    *h.sensor.title.lock().unwrap() = Some("zsh: ~/project".into());
    h.clock.advance(Duration::from_secs(31));
    h.lease();
    h.sample(); // window change: the Terminal frame is due again
    *h.sensor.screen_text.lock().unwrap() = Some("error[E0308]: mismatched types".into());
    *h.sensor.title_after_screen.lock().unwrap() = Some("Bank - Private Browsing".into());
    h.clock.advance(Duration::from_secs(31));
    h.lease();
    h.sample();
    assert!(Metrics::get(&h.rt.metrics.ocr_calls) >= 2);
    assert!(h
        .rt
        .lock()
        .buffer
        .events()
        .all(|e| e.kind != ObservationKind::Capture));
    assert!(h.rt.metrics.drops(DropReason::TitleRule) >= 1);
}

#[test]
fn source_toggles_are_respected() {
    let h = harness();
    h.enable();
    h.patch(serde_json::json!({ "sources": {
        "app_window": true, "selection": false, "clipboard": false, "screen_capture": false
    }}));
    h.lease();
    *h.sensor.selection.lock().unwrap() = Some("some selected text".into());
    h.sensor.copy("x");
    h.sample();
    assert_eq!(h.sensor.count("context+selection"), 0);
    assert_eq!(h.sensor.count("peek"), 0);
    assert_eq!(h.sensor.count("screen"), 0);
    h.patch(serde_json::json!({ "sources": {
        "app_window": false, "selection": false, "clipboard": false, "screen_capture": false
    }}));
    let n = h.sensor.calls().len();
    h.sample();
    assert_eq!(
        h.sensor.calls().len(),
        n,
        "all sources off: nothing is read"
    );
}
