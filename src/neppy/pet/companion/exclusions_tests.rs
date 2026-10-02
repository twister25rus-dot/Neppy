use super::*;
use crate::neppy::pet::companion::settings::{
    apply_patch, CompanionSettings, CompanionSettingsPatch,
};

fn defaults() -> Exclusions {
    Exclusions::from_settings(&CompanionSettings::default())
}

#[test]
fn default_password_managers_are_excluded() {
    let ex = defaults();
    for b in [
        "com.1password.1password",
        "com.agilebits.onepassword7",
        "com.bitwarden.desktop",
        "com.apple.keychainaccess",
        "com.apple.Passwords",
        "com.lastpass.LastPass",
        "org.keepassxc.keepassxc",
        "com.dashlane.dashlanephonefinal",
    ] {
        assert_eq!(
            ex.check_app(Some(b), "x"),
            Some(DropReason::ExcludedApp),
            "{b}"
        );
    }
    // case-insensitive
    assert!(ex
        .check_app(Some("COM.BITWARDEN.DESKTOP"), "Bitwarden")
        .is_some());
    assert!(ex.check_app(Some("com.apple.Notes"), "Notes").is_none());
}

#[test]
fn neppy_itself_is_always_excluded_even_with_no_rules() {
    let ex = Exclusions::compile(&[], &[]).unwrap();
    assert_eq!(
        ex.check_app(Some(NEPPY_BUNDLE_ID), "Neppy"),
        Some(DropReason::ExcludedApp)
    );
}

#[test]
fn incognito_and_private_titles_are_excluded() {
    let ex = defaults();
    for t in [
        "Google - Incognito",
        "Private Browsing - Firefox",
        "InPrivate - Bing",
        "New Private Window",
    ] {
        assert_eq!(ex.check_title(t), Some(DropReason::TitleRule), "{t}");
    }
    assert!(ex.check_title("Quarterly report").is_none());
}

#[test]
fn wildcard_and_name_rules() {
    let ex = Exclusions::compile(&["com.jetbrains.*".into(), "Secret App".into()], &[]).unwrap();
    assert!(ex
        .check_app(Some("com.jetbrains.intellij"), "IDEA")
        .is_some());
    assert!(ex.check_app(None, "secret app").is_some());
    assert!(ex.check_app(Some("com.example"), "Other").is_none());
}

#[test]
fn regex_title_rules_work() {
    let ex = Exclusions::compile(&[], &["re:^bank\\b.*login$".into()]).unwrap();
    assert!(ex.check_title("Bank of X login").is_some());
    assert!(ex.check_title("my bank login page").is_none());
}

#[test]
fn excluded_checks_app_before_title() {
    let ex = defaults();
    assert_eq!(
        ex.excluded(
            Some("com.bitwarden.desktop"),
            "Bitwarden",
            Some("Incognito")
        ),
        Some(DropReason::ExcludedApp)
    );
    assert_eq!(
        ex.excluded(Some("com.apple.Safari"), "Safari", Some("Incognito")),
        Some(DropReason::TitleRule)
    );
    assert_eq!(ex.excluded(Some("com.apple.Safari"), "Safari", None), None);
}

#[test]
fn invalid_rules_are_rejected_at_patch_with_field_names() {
    let cur = CompanionSettings::default();
    let bad_re = CompanionSettingsPatch {
        excluded_title_patterns: Some(vec!["re:(unclosed".into()]),
        ..Default::default()
    };
    let e = apply_patch(&cur, &bad_re).unwrap_err();
    assert!(
        e.contains("excluded_title_patterns") && e.contains("bad regex"),
        "{e}"
    );

    let too_long = CompanionSettingsPatch {
        excluded_apps: Some(vec!["a".repeat(MAX_RULE_CHARS + 1)]),
        ..Default::default()
    };
    assert!(apply_patch(&cur, &too_long)
        .unwrap_err()
        .contains("excluded_apps"));

    let too_many = CompanionSettingsPatch {
        excluded_apps: Some((0..=MAX_RULES).map(|i| format!("app{i}")).collect()),
        ..Default::default()
    };
    assert!(apply_patch(&cur, &too_many)
        .unwrap_err()
        .contains("at most"));

    let empty = CompanionSettingsPatch {
        excluded_apps: Some(vec!["  ".into()]),
        ..Default::default()
    };
    assert!(apply_patch(&cur, &empty).is_err());
}

#[test]
fn pathological_regex_is_bounded_by_the_size_limit() {
    assert!(validate_rules(&[], &["re:(a{1000}){1000}".into()]).is_err());
}
