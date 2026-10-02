use chrono::{Duration, Utc};
use tempfile::TempDir;

use super::*;
use crate::neppy::pet::companion::settings::CompanionSettingsPatch;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn new_sugg(now: DateTime<Utc>, fp: &str) -> NewSuggestion {
    NewSuggestion {
        trigger: SuggestionTrigger::Proactive,
        kind: TriggerKind::BuildError,
        category: ActionCategory::Explain,
        app_name: "Terminal".into(),
        bundle_id: Some("com.apple.Terminal".into()),
        title_excerpt: "zsh".into(),
        context_excerpt: "error[E0308]: mismatched types".into(),
        headline: "A build error".into(),
        body: None,
        score: 70,
        actions: vec![SuggestionAction::Explain, SuggestionAction::Dismiss],
        fingerprint: fp.into(),
        now,
    }
}

fn new_action(now: DateTime<Utc>) -> NewAction {
    NewAction {
        suggestion_id: None,
        category: ActionCategory::SaveNote,
        decision: ActionDecision::Auto,
        level: CompanionLevel::Assist,
        outcome: ActionOutcome::Ok,
        at: now,
    }
}

#[test]
fn schema_is_versioned_in_its_own_file_and_leaves_pet_db_alone() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    for _ in 0..2 {
        let v: i64 = with_connection(&config, |c| {
            Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }
    let dir = config.workspace_dir.join("pet");
    assert!(dir.join("companion.db").exists());
    assert!(
        !dir.join("pet.db").exists(),
        "the research pet.db must not be touched"
    );
}

#[test]
fn settings_default_to_companion_off_with_all_sources_on() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let s = load_settings(&config).unwrap();
    assert!(!s.enabled);
    assert!(
        s.sources.app_window
            && s.sources.selection
            && s.sources.clipboard
            && s.sources.screen_capture
    );
    assert!(s.allow_cloud_model);
    assert_eq!(s, CompanionSettings::default());
}

#[test]
fn settings_round_trip_and_patch() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let patch: CompanionSettingsPatch = serde_json::from_value(serde_json::json!({
        "enabled": true, "level": 2, "sources": {"screen_capture": false}
    }))
    .unwrap();
    let updated = update_settings(&config, &patch).unwrap();
    assert!(updated.enabled && !updated.sources.screen_capture);
    assert_eq!(load_settings(&config).unwrap(), updated);

    let mut direct = updated.clone();
    direct.retention_days = 3;
    save_settings(&config, &direct).unwrap();
    assert_eq!(load_settings(&config).unwrap().retention_days, 3);
}

#[test]
fn invalid_patch_changes_nothing() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let before = load_settings(&config).unwrap();
    let patch: CompanionSettingsPatch = serde_json::from_value(serde_json::json!({
        "enabled": true, "category_levels": {"purchase": 3}
    }))
    .unwrap();
    match update_settings(&config, &patch) {
        Err(UpdateError::Invalid(m)) => assert!(m.contains("high-risk"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(load_settings(&config).unwrap(), before);
}

#[test]
fn corrupt_settings_row_fails_closed_to_defaults() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    with_connection(&config, |c| {
        c.execute(
            "INSERT INTO companion_settings (id, json, updated_at) VALUES (1, 'not json', 'x')",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    assert!(!load_settings(&config).unwrap().enabled);
}

#[test]
fn suggestion_round_trip_and_ordering() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let now = Utc::now();
    let a = insert_suggestion(&config, &new_sugg(now - Duration::minutes(2), "a")).unwrap();
    let b = insert_suggestion(&config, &new_sugg(now - Duration::minutes(1), "b")).unwrap();
    assert_eq!(a.state, SuggestionState::New);
    assert_eq!(a.kind, TriggerKind::BuildError);
    assert_eq!(a.bundle_id.as_deref(), Some("com.apple.terminal"));
    assert_eq!(
        a.actions,
        vec![SuggestionAction::Explain, SuggestionAction::Dismiss]
    );
    assert_eq!(get_suggestion(&config, &a.id).unwrap().unwrap(), a);
    let list = list_suggestions(&config, 10, None).unwrap();
    assert_eq!(
        list.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
        vec![b.id.clone(), a.id.clone()]
    );
    let before = list_suggestions(&config, 10, Some(b.created_at)).unwrap();
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].id, a.id);
    assert_eq!(list_suggestions(&config, 1, None).unwrap().len(), 1);
    assert!(get_suggestion(&config, "nope").unwrap().is_none());
}

#[test]
fn insert_rescrubs_every_text_field() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let key = format!("sk-ant-{}", "a1B2c3D4".repeat(5));
    let mut n = new_sugg(Utc::now(), "fp");
    n.context_excerpt = format!("the key is {key} and the rest of the line");
    n.title_excerpt = "-----BEGIN PRIVATE KEY-----\nMIIE\n-----END PRIVATE KEY-----".into();
    n.headline = format!("leaked {key} in a headline of words");
    n.body = Some(format!("body mentions {key} too, plus more words here"));
    let s = insert_suggestion(&config, &n).unwrap();
    assert!(!s.context_excerpt.contains("sk-ant"));
    assert_eq!(s.title_excerpt, WITHHELD);
    assert!(!s.headline.contains("sk-ant"));
    assert!(!s.body.as_deref().unwrap().contains("sk-ant"));
    // Nothing secret reached the database file either.
    let raw = std::fs::read(config.workspace_dir.join("pet").join("companion.db")).unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("sk-ant"));
}

#[test]
fn state_body_and_handoff_updates() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let s = insert_suggestion(&config, &new_sugg(Utc::now(), "fp")).unwrap();
    assert!(set_state(&config, &s.id, SuggestionState::Shown).unwrap());
    assert!(set_body(&config, &s.id, "Here is an explanation.").unwrap());
    assert!(set_handoff(
        &config,
        &s.id,
        &Handoff {
            thread_id: "t1".into(),
            status: HandoffStatus::Done,
            result_excerpt: Some("all done".into())
        }
    )
    .unwrap());
    let got = get_suggestion(&config, &s.id).unwrap().unwrap();
    assert_eq!(got.state, SuggestionState::Shown);
    assert_eq!(got.body.as_deref(), Some("Here is an explanation."));
    let h = got.handoff.unwrap();
    assert_eq!(
        (h.thread_id.as_str(), h.status),
        ("t1", HandoffStatus::Done)
    );
    assert_eq!(h.result_excerpt.as_deref(), Some("all done"));
    assert!(!set_state(&config, "missing", SuggestionState::Acted).unwrap());
}

#[test]
fn usefulness_history_reads_recent_fingerprints_and_dismissals() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let now = Utc::now();
    let a = insert_suggestion(&config, &new_sugg(now - Duration::minutes(5), "fresh")).unwrap();
    insert_suggestion(&config, &new_sugg(now - Duration::minutes(45), "stale")).unwrap();
    set_state(&config, &a.id, SuggestionState::Dismissed).unwrap();
    let h = usefulness_history(&config, now).unwrap();
    assert_eq!(h.seen.len(), 1);
    assert_eq!(h.seen[0].0, "fresh");
    assert_eq!(h.dismissals.len(), 1);
    assert_eq!(h.dismissals[0].0, TriggerKind::BuildError);
    assert_eq!(h.dismissals[0].1, "com.apple.terminal");
    let seeded = recent_proactive(&config, now - Duration::hours(1)).unwrap();
    assert_eq!(seeded.len(), 2);
}

#[test]
fn action_log_round_trip_has_no_content() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let a = insert_action(&config, &new_action(Utc::now())).unwrap();
    let mut refused = new_action(Utc::now() + Duration::seconds(1));
    refused.category = ActionCategory::SendMessage;
    refused.decision = ActionDecision::RefusedHighRisk;
    refused.level = CompanionLevel::Suggest;
    insert_action(&config, &refused).unwrap();
    let list = list_actions(&config, 10).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[1], a);
    assert_eq!(list[0].decision, ActionDecision::RefusedHighRisk);
    let json = serde_json::to_value(&list[0]).unwrap();
    assert_eq!(json["decision"], "refused_high_risk");
    assert_eq!(json["level"], 1);
}

#[test]
fn prune_by_retention() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let now = Utc::now();
    insert_suggestion(&config, &new_sugg(now - Duration::days(10), "old")).unwrap();
    let keep = insert_suggestion(&config, &new_sugg(now - Duration::days(2), "new")).unwrap();
    insert_action(&config, &new_action(now - Duration::days(40))).unwrap();
    insert_action(&config, &new_action(now - Duration::days(5))).unwrap();
    assert_eq!(prune(&config, 7, now).unwrap(), (1, 1));
    let left = list_suggestions(&config, 10, None).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, keep.id);
    assert_eq!(counts(&config).unwrap(), (1, 1));
    // A shorter retention removes the rest.
    assert_eq!(prune(&config, 1, now).unwrap().0, 1);
}

#[test]
fn delete_one_keeps_the_audit_row_but_unlinks_it() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let s = insert_suggestion(&config, &new_sugg(Utc::now(), "fp")).unwrap();
    let mut a = new_action(Utc::now());
    a.suggestion_id = Some(s.id.clone());
    insert_action(&config, &a).unwrap();
    assert!(delete_suggestion(&config, &s.id).unwrap());
    assert!(!delete_suggestion(&config, &s.id).unwrap());
    let actions = list_actions(&config, 10).unwrap();
    assert_eq!(actions.len(), 1);
    assert!(actions[0].suggestion_id.is_none());
}

#[test]
fn delete_all_leaves_zero_rows_and_keeps_settings() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let patch: CompanionSettingsPatch =
        serde_json::from_value(serde_json::json!({"enabled": true})).unwrap();
    update_settings(&config, &patch).unwrap();
    for i in 0..3 {
        insert_suggestion(&config, &new_sugg(Utc::now(), &format!("fp{i}"))).unwrap();
        insert_action(&config, &new_action(Utc::now())).unwrap();
    }
    assert_eq!(counts(&config).unwrap(), (3, 3));
    assert_eq!(delete_all(&config).unwrap(), (3, 3));
    assert_eq!(counts(&config).unwrap(), (0, 0));
    assert!(list_suggestions(&config, 10, None).unwrap().is_empty());
    assert!(list_actions(&config, 10).unwrap().is_empty());
    assert!(load_settings(&config).unwrap().enabled);
}
