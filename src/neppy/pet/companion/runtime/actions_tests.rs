//! Suggestion actions: save_note, copy_text, open_chat, hand-off, dismiss /
//! mute, data deletion and retention.

use std::time::Duration;

use super::ops;
use super::test_support::*;
use crate::neppy::pet::companion::store;
use crate::neppy::pet::companion::types::*;

async fn ask_suggestion(h: &Harness, selection: &str) -> String {
    h.enable();
    h.lease();
    *h.sensor.selection.lock().unwrap() = Some(selection.into());
    let out = ops::ask(&h.rt, &h.config, "hotkey").await.unwrap();
    out["suggestion"]["id"]
        .as_str()
        .expect("suggestion")
        .to_string()
}

fn desktop_notes(h: &Harness) -> Vec<(String, String)> {
    crate::neppy::pet::store::with_connection(&h.config, |c| {
        let mut stmt = c.prepare("SELECT source, state FROM pet_notes")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
    .unwrap()
}

#[tokio::test]
async fn save_note_creates_a_queued_desktop_pet_note() {
    let h = harness();
    let id = ask_suggestion(&h, "remember to rotate the build cache").await;
    let r = ops::suggestion_act(&h.rt, &h.config, &id, "save_note", None)
        .await
        .unwrap();
    assert_eq!(r.suggestion.state, SuggestionState::Saved);
    assert_eq!(
        desktop_notes(&h),
        vec![("desktop".to_string(), "queued".to_string())]
    );
    let log = store::list_actions(&h.config, 5).unwrap();
    assert_eq!(log[0].category, ActionCategory::SaveNote);
    assert_eq!(log[0].decision, ActionDecision::Confirmed);
}

#[tokio::test]
async fn copy_text_returns_text_and_never_touches_the_clipboard() {
    let h = harness();
    let id = ask_suggestion(&h, "a sentence to copy").await;
    let reads_before = h.sensor.count("read");
    let r = ops::suggestion_act(&h.rt, &h.config, &id, "copy_text", None)
        .await
        .unwrap();
    assert!(r.copy_text.is_some());
    // The sensor trait has no write API at all; nothing else was read.
    assert_eq!(h.sensor.count("read"), reads_before);
    let r = ops::suggestion_act(&h.rt, &h.config, &id, "open_chat", None)
        .await
        .unwrap();
    assert!(r.chat_prompt.unwrap().contains("a sentence to copy"));
}

#[tokio::test]
async fn unknown_action_and_missing_suggestion_are_field_named() {
    let h = harness();
    let id = ask_suggestion(&h, "something").await;
    let e = ops::suggestion_act(&h.rt, &h.config, &id, "launch", None)
        .await
        .unwrap_err();
    assert!(e.starts_with("invalid 'action'"), "{e}");
    let e = ops::suggestion_act(&h.rt, &h.config, "nope", "dismiss", None)
        .await
        .unwrap_err();
    assert!(e.starts_with("invalid 'id'"), "{e}");
}

#[tokio::test]
async fn handoff_runs_and_the_result_comes_back_scrubbed() {
    let h = harness();
    let id = ask_suggestion(&h, "fix the failing build").await;
    let r = ops::suggestion_act(
        &h.rt,
        &h.config,
        &id,
        "handoff",
        Some("Investigate the build error and propose a fix"),
    )
    .await
    .unwrap();
    let ho = r.suggestion.handoff.expect("handoff");
    assert_eq!(ho.status, HandoffStatus::Running);
    assert_eq!(ho.thread_id, "thread-1");
    assert_eq!(h.handoff.prompts.lock().unwrap().len(), 1);
    assert!(h.rt.lock().handoffs.contains_key(&id), "abort handle kept");
    let tx = h.handoff.senders.lock().unwrap().pop().unwrap();
    tx.send(Ok(
        "Done. The key sk-ant-api03-ZZZZZZZZZZZZZZZZZZZZZZZZZZZZ was rotated.".into(),
    ))
    .unwrap();
    let mut done = None;
    for _ in 0..200 {
        let s = store::get_suggestion(&h.config, &id).unwrap().unwrap();
        if s.handoff
            .as_ref()
            .is_some_and(|x| x.status == HandoffStatus::Done)
        {
            done = Some(s);
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let done = done.expect("hand-off finished");
    let excerpt = done.handoff.unwrap().result_excerpt.unwrap();
    assert!(!excerpt.contains("sk-ant-api03"), "{excerpt}");
    assert!(!h.rt.lock().handoffs.contains_key(&id));
    let log = store::list_actions(&h.config, 5).unwrap();
    assert!(log
        .iter()
        .any(|a| a.category == ActionCategory::HandoffTask));
    // An over-long prompt is refused by name.
    let e = ops::suggestion_act(&h.rt, &h.config, &id, "handoff", Some(&"x".repeat(2001)))
        .await
        .unwrap_err();
    assert!(e.starts_with("invalid 'text'"), "{e}");
}

#[tokio::test]
async fn dismiss_and_mute_feed_the_usefulness_counters() {
    let h = harness();
    h.enable();
    h.lease();
    h.sample();
    h.sensor.copy("error[E0308]: mismatched types");
    h.sample();
    let s = store::list_suggestions(&h.config, 10, None).unwrap();
    let id = s[0].id.clone();
    let r = ops::suggestion_act(&h.rt, &h.config, &id, "mute_kind", None)
        .await
        .unwrap();
    assert_eq!(r.suggestion.state, SuggestionState::Dismissed);
    assert_eq!(h.rt.settings().muted_kinds, vec![TriggerKind::BuildError]);
    let r = ops::suggestion_act(&h.rt, &h.config, &id, "mute_app", None)
        .await
        .unwrap();
    assert_eq!(r.suggestion.state, SuggestionState::Dismissed);
    assert_eq!(
        h.rt.settings().muted_apps,
        vec!["com.apple.terminal".to_string()]
    );
}

#[tokio::test]
async fn delete_all_empties_every_table_and_the_buffer() {
    let h = harness();
    let id = ask_suggestion(&h, "keep this").await;
    ops::suggestion_act(&h.rt, &h.config, &id, "save_note", None)
        .await
        .unwrap();
    assert!(!h.rt.lock().buffer.is_empty());
    let out = ops::data_delete(&h.rt, &h.config, None, true, true).unwrap();
    assert_eq!(out["deleted_suggestions"], 1);
    assert_eq!(out["deleted_notes"], 1);
    assert!(out["deleted_actions"].as_u64().unwrap() >= 1);
    assert_eq!(store::counts(&h.config).unwrap(), (0, 0));
    assert!(h.rt.lock().buffer.is_empty());
    assert!(desktop_notes(&h).is_empty());
    let data = ops::data(&h.rt, &h.config).unwrap();
    assert_eq!(data["counts"]["suggestions"], 0);
    let e = ops::data_delete(&h.rt, &h.config, None, false, false).unwrap_err();
    assert!(e.contains("'suggestion_id'"));
}

#[tokio::test]
async fn delete_one_suggestion() {
    let h = harness();
    let id = ask_suggestion(&h, "one").await;
    let out = ops::data_delete(&h.rt, &h.config, Some(&id), false, false).unwrap();
    assert_eq!(out["deleted_suggestions"], 1);
    assert!(store::get_suggestion(&h.config, &id).unwrap().is_none());
}

#[tokio::test]
async fn retention_prune_removes_old_suggestions() {
    let h = harness();
    let id = ask_suggestion(&h, "old one").await;
    h.clock.advance(Duration::from_secs(8 * 24 * 3600));
    ops::prune(&h.rt);
    assert!(store::get_suggestion(&h.config, &id).unwrap().is_none());
}
