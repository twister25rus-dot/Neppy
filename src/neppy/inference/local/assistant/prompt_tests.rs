use super::*;

fn task() -> TaskRecord {
    TaskRecord {
        id: "t1".into(),
        project_root: "/p".into(),
        goal: "Find every spawn site".into(),
        status: TaskStatus::Running,
        allow_edits: true,
        test_command: Some("true".into()),
        max_steps: 8,
        created_at_ms: 0,
        updated_at_ms: 0,
        summary: "Looked at pool.rs.".into(),
        decisions: vec!["Use the pool module".into()],
        changed_files: vec!["a.rs".into()],
        last_test: Some(TestResult {
            exit_code: 1,
            duration_ms: 5,
            tail: "assertion failed".into(),
            timed_out: false,
        }),
        next_step: "Open process.rs".into(),
        completion_tokens_used: 0,
        steps_done: 1,
        error: None,
    }
}

fn snippet(n: usize, lines: usize) -> Snippet {
    Snippet {
        path: format!("src/f{n}.rs"),
        start: 1,
        end: lines as u32,
        text: (0..lines)
            .map(|i| format!("let value_{n}_{i} = {i};\n"))
            .collect(),
        why: "content".into(),
    }
}

fn input(t: &TaskRecord) -> PromptInput<'_> {
    PromptInput {
        task: t,
        step_no: 2,
        max_steps: 8,
        edits_allowed: true,
        tests_available: true,
        correction: None,
    }
}

#[test]
fn the_prompt_never_exceeds_its_budget_however_many_snippets_are_offered() {
    let t = task();
    let cfg = LocalAssistantConfig::default();
    let est = TokenEstimator::default();
    let snippets: Vec<Snippet> = (0..1000).map(|n| snippet(n, 120)).collect();
    let built = build_prompt(&input(&t), &snippets, &cfg, &est).unwrap();
    assert!(built.est_tokens <= cfg.prompt_budget_tokens as usize);
    assert!(built.snippets_used <= cfg.max_snippets);
    let snippet_chars: usize = built.user.matches("--- src/f").count();
    assert!(snippet_chars <= cfg.max_snippets);
    assert!(built.user.len() < cfg.snippet_budget_chars + 8000);
    assert!(fits_context(built.est_tokens, &cfg));
}

#[test]
fn a_tight_budget_trims_snippets_rather_than_overflowing() {
    let t = task();
    let mut cfg = LocalAssistantConfig::default();
    cfg.prompt_budget_tokens = 1500;
    let est = TokenEstimator::default();
    let snippets: Vec<Snippet> = (0..50).map(|n| snippet(n, 100)).collect();
    let built = build_prompt(&input(&t), &snippets, &cfg, &est).unwrap();
    assert!(built.est_tokens <= 1500, "{}", built.est_tokens);
    assert!(built.snippets_used >= 1);
}

#[test]
fn a_budget_smaller_than_the_notes_is_an_error_not_an_overflow() {
    let t = task();
    let mut cfg = LocalAssistantConfig::default();
    cfg.prompt_budget_tokens = 50;
    assert!(build_prompt(&input(&t), &[], &cfg, &TokenEstimator::default()).is_err());
}

#[test]
fn the_prompt_carries_the_notes_and_nothing_from_earlier_replies() {
    let t = task();
    let cfg = LocalAssistantConfig::default();
    let built = build_prompt(
        &input(&t),
        &[snippet(1, 5)],
        &cfg,
        &TokenEstimator::default(),
    )
    .unwrap();
    let p = &built.user;
    for kept in [
        "Find every spawn site",
        "Looked at pool.rs.",
        "Use the pool module",
        "a.rs",
        "assertion failed",
        "Open process.rs",
        "src/f1.rs:1-5",
        "step 2 of at most 8",
    ] {
        assert!(p.contains(kept), "missing `{kept}`");
    }
    // The type of `build_prompt`'s input is the whole argument list: there is
    // no place for a previous raw reply to come from.
    assert!(!p.contains("PRIOR_REPLY_MARKER"));
}

#[test]
fn edit_and_test_availability_are_stated() {
    let mut t = task();
    t.test_command = None;
    let cfg = LocalAssistantConfig::default();
    let mut i = input(&t);
    i.edits_allowed = false;
    i.tests_available = false;
    let p = build_prompt(&i, &[], &cfg, &TokenEstimator::default())
        .unwrap()
        .user;
    assert!(p.contains("Edits are NOT allowed"));
    assert!(p.contains("No test command is configured"));
    assert!(p.contains("No project excerpts matched"));
}

#[test]
fn a_correction_note_appears_only_when_given() {
    let t = task();
    let cfg = LocalAssistantConfig::default();
    let mut i = input(&t);
    i.correction = Some("invalid JSON: eof");
    let p = build_prompt(&i, &[], &cfg, &TokenEstimator::default())
        .unwrap()
        .user;
    assert!(p.contains("could not be used: invalid JSON: eof"));
}

#[test]
fn the_estimator_calibrates_toward_the_observed_ratio_within_bounds() {
    let mut est = TokenEstimator::default();
    assert_eq!(est.estimate(300), 100);
    est.calibrate(4000, 1000);
    assert!(est.chars_per_token() > 3.0 && est.chars_per_token() < 4.0);
    est.calibrate(1000, 1);
    assert!(est.chars_per_token() <= 6.0);
    est.calibrate(0, 10);
    est.calibrate(10, 0);
}

#[test]
fn a_plan_is_read_from_a_bare_fenced_or_chatty_reply() {
    let json = r#"{"summary":"s","decisions":["d"],"edits":[{"path":"a.rs","search":"x","replace":"y"}],
        "run_tests":true,"next_step":"n","done":false,"search_queries":["q"]}"#;
    let bare = parse_plan(json).unwrap();
    assert_eq!(bare.edits[0].path, "a.rs");
    assert!(bare.run_tests);
    let fenced = parse_plan(&format!("Sure!\n```json\n{json}\n```\nDone.")).unwrap();
    assert_eq!(fenced, bare);
    let thinking = parse_plan(&format!("<think>hmm {{not json}}</think>{json}")).unwrap();
    assert_eq!(thinking, bare);
}

#[test]
fn braces_inside_strings_do_not_break_extraction() {
    let reply = r#"{"summary":"uses } and { in text","next_step":"go","edits":[]}"#;
    let plan = parse_plan(reply).unwrap();
    assert_eq!(plan.summary, "uses } and { in text");
}

#[test]
fn unusable_replies_say_why() {
    assert!(parse_plan("no json at all")
        .unwrap_err()
        .contains("no JSON"));
    assert!(parse_plan(r#"{"summary": "cut off"#).is_err());
    assert!(parse_plan("{}").unwrap_err().contains("empty"));
    assert!(parse_plan(r#"{"decisions": 5}"#)
        .unwrap_err()
        .contains("invalid JSON"));
}

#[test]
fn list_fields_accept_a_bare_string_or_null() {
    // Seen from the 9B in a soak: "invalid type: string ..., expected a sequence".
    let plan = parse_plan(
        r#"{"summary":"s","decisions":"track spawn sites","search_queries":null,"next_step":"n"}"#,
    )
    .unwrap();
    assert_eq!(plan.decisions, vec!["track spawn sites".to_string()]);
    assert!(plan.search_queries.is_empty());
    // A bare string alone is still not a plan.
    assert!(parse_plan(r#"{"decisions": "not a list"}"#)
        .unwrap_err()
        .contains("empty"));
}

#[test]
fn the_reply_schema_puts_edits_before_the_free_text_arrays() {
    let t = task();
    let built = build_prompt(
        &input(&t),
        &[],
        &LocalAssistantConfig::default(),
        &TokenEstimator::default(),
    )
    .unwrap();
    let at = |needle: &str| {
        built
            .user
            .rfind(needle)
            .unwrap_or_else(|| panic!("missing {needle}"))
    };
    assert!(at("\"edits\"") < at("\"summary\""));
    assert!(at("\"summary\"") < at("\"decisions\""));
    assert!(built.user.contains("Put edits first"));
}

#[test]
fn plan_fields_are_bounded() {
    let many_edits: String = (0..9)
        .map(|i| format!(r#"{{"path":"f{i}.rs","search":"a","replace":"b"}}"#))
        .collect::<Vec<_>>()
        .join(",");
    let err = parse_plan(&format!(r#"{{"summary":"s","edits":[{many_edits}]}}"#)).unwrap_err();
    assert!(err.contains("too many edits"));

    let huge = "z".repeat(EDIT_FIELD_MAX + 1);
    let err = parse_plan(&format!(
        r#"{{"summary":"s","edits":[{{"path":"a.rs","search":"{huge}","replace":""}}]}}"#
    ))
    .unwrap_err();
    assert!(err.contains("too large"));

    let noisy = format!(
        r#"{{"summary":"{}","next_step":"{}","decisions":[{}],"search_queries":[{}]}}"#,
        "s".repeat(10_000),
        "n".repeat(5000),
        (0..30)
            .map(|i| format!("\"d{i}\""))
            .collect::<Vec<_>>()
            .join(","),
        (0..30)
            .map(|i| format!("\"q{i}\""))
            .collect::<Vec<_>>()
            .join(","),
    );
    let plan = parse_plan(&noisy).unwrap();
    assert!(plan.summary.len() <= SUMMARY_MAX * 2);
    assert!(plan.next_step.len() <= NEXT_STEP_MAX);
    assert_eq!(plan.decisions.len(), 10);
    assert_eq!(plan.search_queries.len(), QUERIES_PER_STEP);
}

#[test]
fn a_repeated_key_keeps_the_last_value_instead_of_failing_the_step() {
    // Seen from the 9B in a soak: "duplicate field `search_queries`".
    let plan = parse_plan(
        r#"{"summary":"first","next_step":"n","search_queries":["a"],"summary":"second","search_queries":["b"]}"#,
    )
    .unwrap();
    assert_eq!(plan.summary, "second");
    assert_eq!(plan.search_queries, vec!["b".to_string()]);
}
