use super::*;
use crate::neppy::pet::companion::types::{ActionCategory as C, CompanionLevel as L};

#[test]
fn defaults_follow_the_user_decisions() {
    let s = CompanionSettings::default();
    // The companion itself is OFF until the user consents...
    assert!(!s.enabled);
    // ...but once enabled every source is ON.
    assert!(s.sources.app_window && s.sources.selection && s.sources.clipboard);
    assert!(s.sources.screen_capture);
    assert_eq!(s.screen_min_interval_secs, 30);
    assert!(s.allow_cloud_model);
    assert_eq!(s.level, L::Suggest);
    assert_eq!(s.retention_days, 7);
    assert_eq!(s.chattiness, Chattiness::Normal);
    assert_eq!(
        (s.min_interval_min, s.max_per_hour, s.app_cooldown_min),
        (10, 3, 30)
    );
    assert_eq!(s.hotkeys.pause, "CmdOrCtrl+Alt+Shift+P");
    assert_eq!(s.hotkeys.ask, "CmdOrCtrl+Alt+Shift+Space");
    assert_eq!(s.hotkeys.capture, "CmdOrCtrl+Alt+Shift+S");
    assert!(s
        .excluded_apps
        .iter()
        .any(|a| a == "com.1password.1password"));
    assert_eq!(s.excluded_title_patterns.len(), 4);
}

#[test]
fn default_category_levels() {
    let s = CompanionSettings::default();
    for (c, l) in [
        (C::Explain, L::Assist),
        (C::DraftText, L::Assist),
        (C::SaveNote, L::Assist),
        (C::FormatText, L::Suggest),
        (C::PrepareCommand, L::Suggest),
        (C::OpenChat, L::Suggest),
        (C::HandoffTask, L::Suggest),
    ] {
        assert_eq!(s.category_level(c), l, "{c:?}");
    }
    assert_eq!(s.category_levels.len(), C::ALL.len());
    for c in C::HIGH_RISK {
        assert!(s.category_level(*c) <= L::Suggest);
    }
}

#[test]
fn wire_json_shape_and_partial_json_defaults() {
    let v = serde_json::to_value(CompanionSettings::default()).unwrap();
    assert_eq!(v["level"], 1);
    assert_eq!(v["sources"]["screen_capture"], true);
    assert_eq!(v["category_levels"]["explain"], 2);
    assert_eq!(v["enabled"], false);
    let partial: CompanionSettings = serde_json::from_str(r#"{"enabled": true}"#).unwrap();
    assert!(partial.enabled && partial.sources.screen_capture && partial.allow_cloud_model);
}

#[test]
fn patch_rejects_unknown_fields() {
    let r: Result<CompanionSettingsPatch, _> = serde_json::from_str(r#"{"bogus": 1}"#);
    assert!(r.is_err());
    let r: Result<CompanionSettingsPatch, _> = serde_json::from_str(r#"{"sources":{"x":true}}"#);
    assert!(r.is_err());
}

#[test]
fn patch_applies_and_merges() {
    let cur = CompanionSettings::default();
    let p: CompanionSettingsPatch = serde_json::from_value(serde_json::json!({
        "enabled": true, "level": 2, "sources": {"clipboard": false},
        "allow_cloud_model": false, "retention_days": 14, "chattiness": "quiet",
        "screen_min_interval_secs": 60,
        "category_levels": {"explain": 3, "handoff_task": 2},
        "muted_kinds": ["term"], "muted_apps": ["com.example.App"],
        "hotkeys": {"pause": "Ctrl+Alt+X"}
    }))
    .unwrap();
    let s = apply_patch(&cur, &p).unwrap();
    assert!(s.enabled && !s.sources.clipboard && s.sources.selection);
    assert_eq!(s.level, L::Assist);
    assert!(!s.allow_cloud_model);
    assert_eq!(s.retention_days, 14);
    assert_eq!(s.chattiness, Chattiness::Quiet);
    assert_eq!(s.screen_min_interval_secs, 60);
    assert_eq!(s.category_level(C::Explain), L::Trusted);
    assert_eq!(s.category_level(C::DraftText), L::Assist);
    assert_eq!(s.muted_kinds, vec![TriggerKind::Term]);
    assert_eq!(s.hotkeys.pause, "Ctrl+Alt+X");
    assert_eq!(s.hotkeys.ask, DEFAULT_HOTKEY_ASK);
}

fn err(v: serde_json::Value) -> String {
    let p: CompanionSettingsPatch = serde_json::from_value(v).unwrap();
    apply_patch(&CompanionSettings::default(), &p).unwrap_err()
}

#[test]
fn patch_rejects_high_risk_above_one() {
    for c in C::HIGH_RISK {
        let e = err(serde_json::json!({"category_levels": {c.as_str(): 2}}));
        assert!(e.contains("is high-risk and always asks"), "{e}");
        assert!(e.contains(c.as_str()));
        // 0 and 1 are fine.
        let p: CompanionSettingsPatch =
            serde_json::from_value(serde_json::json!({"category_levels": {c.as_str(): 0}}))
                .unwrap();
        assert!(apply_patch(&CompanionSettings::default(), &p).is_ok());
    }
}

#[test]
fn patch_errors_name_the_field() {
    assert!(err(serde_json::json!({"level": 4})).contains("'level'"));
    assert!(err(serde_json::json!({"level": -1})).contains("'level'"));
    assert!(err(serde_json::json!({"retention_days": 0})).contains("'retention_days'"));
    assert!(err(serde_json::json!({"retention_days": 91})).contains("'retention_days'"));
    assert!(err(serde_json::json!({"min_interval_min": 0})).contains("'min_interval_min'"));
    assert!(err(serde_json::json!({"max_per_hour": 21})).contains("'max_per_hour'"));
    assert!(err(serde_json::json!({"app_cooldown_min": 481})).contains("'app_cooldown_min'"));
    assert!(err(serde_json::json!({"screen_min_interval_secs": 4}))
        .contains("'screen_min_interval_secs'"));
    assert!(err(serde_json::json!({"chattiness": "loud"})).contains("'chattiness'"));
    assert!(err(serde_json::json!({"category_levels": {"nope": 1}})).contains("unknown category"));
    assert!(err(serde_json::json!({"category_levels": {"explain": 9}})).contains("category_levels"));
    assert!(err(serde_json::json!({"muted_kinds": ["ask"]})).contains("'muted_kinds'"));
    assert!(err(serde_json::json!({"muted_kinds": ["nope"]})).contains("'muted_kinds'"));
}

#[test]
fn hotkey_validation() {
    assert!(validate_hotkey("pause", "CmdOrCtrl+Alt+Shift+P").is_ok());
    assert!(validate_hotkey("pause", "P").is_err());
    assert!(validate_hotkey("pause", "Ctrl+").is_err());
    assert!(validate_hotkey("pause", "Foo+P").is_err());
    assert!(validate_hotkey("pause", "Ctrl+Shift").is_err());
    assert!(validate_hotkey("pause", "Ctrl+Sh!ft+P").is_err());
    assert!(err(serde_json::json!({"hotkeys": {"pause": "bad"}})).contains("hotkeys.pause"));
    let dup = err(serde_json::json!({"hotkeys": {"pause": "CmdOrCtrl+Alt+Shift+Space"}}));
    assert!(dup.contains("must differ"), "{dup}");
}

#[test]
fn normalized_clamps_stored_high_risk_levels_and_fills_missing() {
    let mut s = CompanionSettings::default();
    s.category_levels.insert(C::Purchase, L::Trusted);
    s.category_levels.remove(&C::Explain);
    let s = s.normalized();
    assert_eq!(s.category_levels[&C::Purchase], L::Suggest);
    assert_eq!(s.category_levels[&C::Explain], L::Assist);
}
