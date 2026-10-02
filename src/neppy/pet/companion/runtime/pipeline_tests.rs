//! Event → suggestion: triggers, usefulness + rate limit, policy levels,
//! provider choice, privacy of everything that leaves the sensor.

use std::sync::atomic::Ordering;
use std::time::Duration;

use super::bus::subscribe_companion_events;
use super::generate::{choose_provider, GenProvider, ProviderCaps};
use super::metrics::Metrics;
use super::ops;
use super::test_support::*;
use crate::neppy::pet::companion::store;
use crate::neppy::pet::companion::types::*;

const BUILD_ERROR: &str = "error[E0308]: mismatched types\n --> src/main.rs:4:5";

fn proactive(h: &Harness) -> Vec<CompanionSuggestion> {
    store::list_suggestions(&h.config, 100, None)
        .unwrap()
        .into_iter()
        .filter(|s| s.trigger == SuggestionTrigger::Proactive)
        .collect()
}

async fn wait_for(cond: impl Fn() -> bool) -> bool {
    for _ in 0..200 {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    cond()
}

#[tokio::test]
async fn build_error_via_clipboard_makes_exactly_one_suggestion() {
    let h = harness();
    h.enable();
    h.lease();
    h.sample();
    h.sensor.copy(BUILD_ERROR);
    h.sample();
    let s = proactive(&h);
    assert_eq!(s.len(), 1);
    assert_eq!(s[0].kind, TriggerKind::BuildError);
    assert!(s[0].body.is_none(), "level 1: a template, no model call");
    assert_eq!(Metrics::get(&h.rt.metrics.llm_calls), 0);
    // The same error again within 30 min: novelty, nothing new.
    h.clock.advance(Duration::from_secs(11 * 60));
    h.lease();
    h.sensor.copy(&format!("{BUILD_ERROR} "));
    h.sample();
    assert_eq!(proactive(&h).len(), 1);
}

#[tokio::test]
async fn at_most_three_proactive_suggestions_an_hour() {
    let h = harness();
    h.enable();
    h.lease();
    let apps = [
        ("Terminal", "com.apple.Terminal"),
        ("iTerm2", "com.googlecode.iterm2"),
        ("Warp", "dev.warp.Warp-Stable"),
        ("Ghostty", "com.mitchellh.ghostty"),
    ];
    for (i, (name, bundle)) in apps.iter().enumerate() {
        h.sensor.set_app(name, bundle, 500 + i as i32);
        h.sample();
        h.sensor
            .copy(&format!("error: failure number {i} in crate alpha{i}"));
        h.sample();
        h.clock.advance(Duration::from_secs(11 * 60));
        h.lease();
    }
    assert_eq!(
        proactive(&h).len(),
        3,
        "the 4th trigger in the hour is capped"
    );
}

#[tokio::test]
async fn level_zero_shows_nothing_proactive_but_ask_still_works() {
    let h = harness();
    h.enable();
    h.patch(serde_json::json!({ "level": 0 }));
    h.lease();
    h.sample();
    h.sensor.copy(BUILD_ERROR);
    h.sample();
    assert!(proactive(&h).is_empty());
    *h.sensor.selection.lock().unwrap() = Some("what does this mean?".into());
    let out = ops::ask(&h.rt, &h.config, "hotkey").await.unwrap();
    assert_eq!(out["status"], "ok", "{out}");
    assert_eq!(out["suggestion"]["trigger"], "ask");
    // Ask is user-initiated: the model writes a body.
    assert!(wait_for(|| Metrics::get(&h.rt.metrics.llm_calls) == 1).await);
}

#[tokio::test]
async fn level_two_explain_generates_the_body_automatically_and_audits_it() {
    let h = harness();
    h.enable();
    h.patch(serde_json::json!({ "level": 2 }));
    h.lease();
    h.sample();
    h.sensor.copy(BUILD_ERROR);
    h.sample();
    let id = proactive(&h)[0].id.clone();
    let cfg = h.config.clone();
    assert!(
        wait_for(|| store::get_suggestion(&cfg, &id)
            .unwrap()
            .unwrap()
            .body
            .is_some())
        .await,
        "auto body"
    );
    let actions = store::list_actions(&h.config, 10).unwrap();
    assert_eq!(actions[0].decision, ActionDecision::Auto);
    assert_eq!(actions[0].category, ActionCategory::Explain);
    assert_eq!(Metrics::get(&h.rt.metrics.auto_actions), 1);
}

#[tokio::test]
async fn high_risk_categories_are_refused_and_audited() {
    let h = harness();
    h.enable();
    h.patch(serde_json::json!({ "level": 3 }));
    for cat in ActionCategory::HIGH_RISK {
        let err = super::actions::check_policy(&h.rt, &h.config, None, *cat).unwrap_err();
        assert!(err.contains("high-risk"), "{err}");
    }
    let log = store::list_actions(&h.config, 50).unwrap();
    assert_eq!(log.len(), ActionCategory::HIGH_RISK.len());
    assert!(log
        .iter()
        .all(|a| a.decision == ActionDecision::RefusedHighRisk));
}

#[tokio::test]
async fn pause_cancels_an_in_flight_generation() {
    let h = harness();
    h.enable();
    h.lease();
    *h.generator.delay.lock().unwrap() = Duration::from_secs(30);
    *h.sensor.selection.lock().unwrap() = Some("explain this".into());
    let out = ops::ask(&h.rt, &h.config, "hotkey").await.unwrap();
    let id = out["suggestion"]["id"].as_str().unwrap().to_string();
    assert!(wait_for(|| h.generator.prompts.lock().unwrap().len() == 1).await);
    h.rt.pause(None, "hotkey");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(store::get_suggestion(&h.config, &id)
        .unwrap()
        .unwrap()
        .body
        .is_none());
    // An event stamped after the pause is dropped.
    let late = ops::ask(&h.rt, &h.config, "hotkey").await.unwrap();
    assert_eq!(late["status"], "paused");
}

#[test]
fn provider_choice_defaults_to_the_chat_model_and_falls_back_locally() {
    let remote = ProviderCaps {
        chat_is_local: false,
        local_available: true,
    };
    assert_eq!(
        choose_provider(true, remote),
        GenProvider::Chat,
        "D3: cloud default"
    );
    assert_eq!(choose_provider(false, remote), GenProvider::Local);
    let none = ProviderCaps {
        chat_is_local: false,
        local_available: false,
    };
    assert_eq!(choose_provider(false, none), GenProvider::None);
    let local_chat = ProviderCaps {
        chat_is_local: true,
        local_available: false,
    };
    assert_eq!(choose_provider(false, local_chat), GenProvider::Chat);
}

#[tokio::test]
async fn cloud_off_without_a_local_model_means_no_body_and_open_chat() {
    let h = harness();
    h.enable();
    h.patch(serde_json::json!({ "allow_cloud_model": false }));
    h.lease();
    *h.sensor.selection.lock().unwrap() = Some("explain this please".into());
    let out = ops::ask(&h.rt, &h.config, "hotkey").await.unwrap();
    let id = out["suggestion"]["id"].as_str().unwrap().to_string();
    let act = ops::suggestion_act(&h.rt, &h.config, &id, "explain", None)
        .await
        .unwrap();
    assert!(act.suggestion.body.is_none());
    assert!(act.chat_prompt.is_some(), "the user can send it themselves");
    assert!(
        h.generator.prompts.lock().unwrap().is_empty(),
        "no model was called"
    );
    // With a local model it is used instead of the cloud.
    h.generator.caps.lock().unwrap().local_available = true;
    let act = ops::suggestion_act(&h.rt, &h.config, &id, "explain", None)
        .await
        .unwrap();
    assert!(act.suggestion.body.is_some());
    assert_eq!(
        *h.generator.providers.lock().unwrap(),
        vec![GenProvider::Local]
    );
    assert_eq!(Metrics::get(&h.rt.metrics.local_model_calls), 1);
}

/// PC10: fixture secrets fed through every source never reach the store,
/// socket events, status, or the prompt.
#[tokio::test]
async fn fixture_secrets_never_leave_the_sensor() {
    const CARD: &str = "4111 1111 1111 1111";
    const KEY: &str = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const OTP: &str = "482913";
    const PASS: &str = "hunter2!";
    const GH: &str = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let needles = [CARD, "4111111111111111", KEY, OTP, PASS, GH];
    let mut events = subscribe_companion_events();
    let h = harness();
    h.enable();
    h.patch(serde_json::json!({ "level": 2 }));
    h.lease();
    let secret_text = format!(
        "error[E0308]: mismatched types\nkey {KEY}\ncard {CARD}\nYour verification code is {OTP}\npassword: {PASS}\ntoken {GH}"
    );
    *h.sensor.title.lock().unwrap() = Some(format!("zsh {KEY}"));
    *h.sensor.selection.lock().unwrap() = Some(secret_text.clone());
    h.sample();
    h.sensor.copy(&secret_text);
    h.sample();
    h.clock.advance(Duration::from_secs(31));
    h.lease();
    *h.sensor.screen_text.lock().unwrap() = Some(secret_text.clone());
    h.sample();
    *h.sensor.region.lock().unwrap() = Some(secret_text.clone());
    let _ = ops::ask(&h.rt, &h.config, "hotkey").await.unwrap();
    let _ = ops::capture(&h.rt, &h.config, "hotkey").await.unwrap();
    assert!(wait_for(|| !h.generator.prompts.lock().unwrap().is_empty()).await);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!store::list_suggestions(&h.config, 100, None)
        .unwrap()
        .is_empty());

    let mut haystacks: Vec<String> = Vec::new();
    // Every row of every companion table, as text.
    store::with_connection(&h.config, |c| {
        for table in [
            "companion_settings",
            "companion_suggestions",
            "companion_actions",
        ] {
            let mut stmt = c.prepare(&format!("SELECT * FROM {table}"))?;
            let n = stmt.column_count();
            let mut rows = stmt.query([])?;
            while let Some(r) = rows.next()? {
                for i in 0..n {
                    let v: rusqlite::types::Value = r.get(i)?;
                    haystacks.push(format!("{v:?}"));
                }
            }
        }
        Ok(())
    })
    .unwrap();
    while let Ok(ev) = events.try_recv() {
        haystacks.push(serde_json::to_string(&ev).unwrap());
    }
    haystacks.push(ops::status_value(&h.rt).to_string());
    haystacks.extend(h.generator.prompts.lock().unwrap().iter().cloned());
    for hay in &haystacks {
        for n in needles {
            assert!(!hay.contains(n), "secret {n:?} leaked into {hay:.200}");
        }
    }
}

/// The companion never writes main memory: after a full run the workspace
/// holds only the Pet's own databases.
#[tokio::test]
async fn no_main_memory_write_after_generation() {
    let h = harness();
    h.enable();
    h.lease();
    *h.sensor.selection.lock().unwrap() = Some("explain this please".into());
    let out = ops::ask(&h.rt, &h.config, "hotkey").await.unwrap();
    assert_eq!(out["status"], "ok");
    assert!(wait_for(|| Metrics::get(&h.rt.metrics.llm_calls) == 1).await);
    let mut files = Vec::new();
    let mut stack = vec![h.config.workspace_dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            if e.path().is_dir() {
                stack.push(e.path());
            } else {
                files.push(
                    e.path()
                        .strip_prefix(&h.config.workspace_dir)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                );
            }
        }
    }
    assert!(
        files.iter().all(|f| f.starts_with("pet/")),
        "unexpected workspace writes: {files:?}"
    );
}

#[tokio::test]
async fn metrics_are_populated() {
    let h = harness();
    h.enable();
    h.lease();
    for _ in 0..3 {
        let t0 = std::time::Instant::now();
        h.sample();
        h.rt.metrics.record_sample(t0.elapsed().as_millis() as u32);
    }
    h.sensor.secure_field.store(true, Ordering::SeqCst);
    h.sample();
    let m = h.rt.metrics.snapshot();
    assert_eq!(m.samples_total, 3);
    assert!(m.events_accepted >= 1);
    assert_eq!(m.drops_by_reason.get("secure_field"), Some(&1));
    assert!(m.capture_ms_total < 10_000);
}
