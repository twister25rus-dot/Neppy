use super::*;

#[test]
fn default_is_chat_and_no_label_reads_as_chat() {
    assert_eq!(ThreadMode::default(), ThreadMode::Chat);
    assert_eq!(ThreadMode::from_labels(&[]), ThreadMode::Chat);
    assert_eq!(
        ThreadMode::from_labels(&["general".to_string()]),
        ThreadMode::Chat
    );
}

#[test]
fn orchestration_label_round_trips() {
    let labels = labels_with_mode(vec!["general".into()], ThreadMode::Orchestration);
    assert_eq!(labels, vec!["general", ORCHESTRATION_LABEL]);
    assert_eq!(ThreadMode::from_labels(&labels), ThreadMode::Orchestration);

    // Switching back removes it, leaving the user's labels untouched.
    let back = labels_with_mode(labels, ThreadMode::Chat);
    assert_eq!(back, vec!["general"]);
    assert_eq!(ThreadMode::from_labels(&back), ThreadMode::Chat);
}

#[test]
fn labels_with_mode_never_duplicates_the_label() {
    let once = labels_with_mode(vec![], ThreadMode::Orchestration);
    let twice = labels_with_mode(once, ThreadMode::Orchestration);
    assert_eq!(twice, vec![ORCHESTRATION_LABEL]);
}

#[test]
fn strip_removes_every_reserved_label_only() {
    let stripped = strip_mode_labels(vec![
        "general".into(),
        "mode:orchestration".into(),
        "mode:future".into(),
        "tasks".into(),
    ]);
    assert_eq!(stripped, vec!["general", "tasks"]);
}

#[test]
fn parse_is_strict_but_case_insensitive() {
    assert_eq!(ThreadMode::parse(" Chat "), Some(ThreadMode::Chat));
    assert_eq!(
        ThreadMode::parse("ORCHESTRATION"),
        Some(ThreadMode::Orchestration)
    );
    assert_eq!(ThreadMode::parse("pet"), None);
    assert_eq!(ThreadMode::parse(""), None);
}

#[test]
fn serde_uses_lowercase_wire_names() {
    assert_eq!(
        serde_json::to_string(&ThreadMode::Orchestration).unwrap(),
        "\"orchestration\""
    );
    assert_eq!(
        serde_json::from_str::<ThreadMode>("\"chat\"").unwrap(),
        ThreadMode::Chat
    );
}

#[tokio::test]
async fn turn_mode_is_scoped_and_absent_outside() {
    assert_eq!(current_turn_mode(), None);
    let seen = with_turn_mode(ThreadMode::Orchestration, async { current_turn_mode() }).await;
    assert_eq!(seen, Some(ThreadMode::Orchestration));
    assert_eq!(current_turn_mode(), None);
}
