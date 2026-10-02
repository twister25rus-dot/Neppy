use super::*;
use chrono::{Duration, TimeZone, Utc};
use tinyagents::session::run_ledger::{AgentRunKind, RunTelemetry};

fn run(
    id: &str,
    thread: Option<&str>,
    agent: &str,
    status: AgentRunStatus,
    updated_secs: i64,
) -> AgentRun {
    let started = Utc.timestamp_opt(1_800_000_000, 0).unwrap();
    AgentRun {
        id: id.to_string(),
        kind: AgentRunKind::Subagent,
        parent_run_id: None,
        parent_thread_id: thread.map(str::to_string),
        agent_id: Some(agent.to_string()),
        status,
        prompt_ref: None,
        worker_thread_id: None,
        task_board_id: None,
        task_card_id: None,
        checkpoint_path: None,
        checkpoint: None,
        summary: Some("found   three\nfiles".to_string()),
        error: None,
        metadata: serde_json::json!({}),
        telemetry: Some(RunTelemetry {
            run_id: id.to_string(),
            tool_count: 4,
            cost_usd: 0.25,
            model: Some("m1".into()),
            ..Default::default()
        }),
        started_at: started,
        updated_at: started + Duration::seconds(updated_secs),
        completed_at: status
            .is_terminal()
            .then(|| started + Duration::seconds(updated_secs)),
    }
}

fn dir() -> HashMap<String, (String, ThreadMode)> {
    HashMap::from([
        (
            "t-orch".to_string(),
            ("Ship the release".to_string(), ThreadMode::Orchestration),
        ),
        (
            "t-chat".to_string(),
            ("Quick question".to_string(), ThreadMode::Chat),
        ),
    ])
}

#[test]
fn live_runs_are_phased_by_agent_role() {
    use AgentRunStatus::Running;
    assert_eq!(
        phase_for(Some("researcher"), Running),
        RunPhase::Researching
    );
    assert_eq!(
        phase_for(Some("agent_memory"), Running),
        RunPhase::Researching
    );
    assert_eq!(phase_for(Some("planner"), Running), RunPhase::Planning);
    assert_eq!(
        phase_for(Some("code_executor"), Running),
        RunPhase::Implementing
    );
    assert_eq!(phase_for(Some("critic"), Running), RunPhase::Reviewing);
    assert_eq!(
        phase_for(Some("code_review_bot"), Running),
        RunPhase::Reviewing
    );
    assert_eq!(phase_for(Some("test_runner"), Running), RunPhase::Testing);
    assert_eq!(phase_for(None, Running), RunPhase::Implementing);
    assert_eq!(
        phase_for(Some("researcher"), AgentRunStatus::Pending),
        RunPhase::Researching
    );
}

#[test]
fn settled_runs_report_their_outcome_not_their_role() {
    assert_eq!(
        phase_for(Some("researcher"), AgentRunStatus::Completed),
        RunPhase::Completed
    );
    assert_eq!(
        phase_for(Some("planner"), AgentRunStatus::Failed),
        RunPhase::Failed
    );
    assert_eq!(
        phase_for(Some("planner"), AgentRunStatus::Interrupted),
        RunPhase::Failed
    );
    assert_eq!(
        phase_for(Some("planner"), AgentRunStatus::Cancelled),
        RunPhase::Cancelled
    );
    assert_eq!(
        phase_for(Some("tools_agent"), AgentRunStatus::AwaitingUser),
        RunPhase::AwaitingUser
    );
}

#[test]
fn projection_joins_thread_title_and_mode_and_shapes_the_row() {
    let runs = vec![run(
        "r1",
        Some("t-orch"),
        "researcher",
        AgentRunStatus::Running,
        10,
    )];
    let history = project(&runs, &dir(), &RunsHistoryRequest::default());
    assert_eq!(history.count, 1);
    let row = &history.runs[0];
    assert_eq!(row.thread_title.as_deref(), Some("Ship the release"));
    assert_eq!(row.thread_mode.as_deref(), Some("orchestration"));
    assert_eq!(row.phase, RunPhase::Researching);
    // Summary is flattened to one line.
    assert_eq!(row.summary.as_deref(), Some("found three files"));
    assert_eq!(row.tool_count, Some(4));
    assert_eq!(row.model.as_deref(), Some("m1"));

    // Wire shape: camelCase keys, snake_case phase.
    let json = to_value(&history).unwrap();
    let first = &json["runs"][0];
    for key in [
        "runId",
        "threadId",
        "threadTitle",
        "threadMode",
        "agentId",
        "kind",
        "status",
        "phase",
        "summary",
        "startedAt",
        "updatedAt",
        "elapsedMs",
        "toolCount",
    ] {
        assert!(first.get(key).is_some(), "missing key {key}: {first}");
    }
    assert_eq!(first["phase"], "researching");
    assert_eq!(json["count"], 1);
    assert!(json["threads"].is_array());
}

#[test]
fn runs_are_newest_first_and_limited() {
    let runs = vec![
        run(
            "old",
            Some("t-orch"),
            "researcher",
            AgentRunStatus::Completed,
            1,
        ),
        run(
            "new",
            Some("t-orch"),
            "planner",
            AgentRunStatus::Running,
            99,
        ),
        run(
            "mid",
            Some("t-orch"),
            "critic",
            AgentRunStatus::Completed,
            50,
        ),
    ];
    let history = project(
        &runs,
        &dir(),
        &RunsHistoryRequest {
            limit: Some(2),
            ..Default::default()
        },
    );
    let ids: Vec<_> = history.runs.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(ids, vec!["new", "mid"]);
}

#[test]
fn mode_and_thread_filters_apply_and_unthreaded_runs_drop_by_default() {
    let runs = vec![
        run(
            "a",
            Some("t-orch"),
            "researcher",
            AgentRunStatus::Running,
            3,
        ),
        run(
            "b",
            Some("t-chat"),
            "researcher",
            AgentRunStatus::Running,
            2,
        ),
        run("c", None, "researcher", AgentRunStatus::Running, 1),
        run(
            "d",
            Some("t-gone"),
            "researcher",
            AgentRunStatus::Running,
            0,
        ),
    ];
    let orch = project(
        &runs,
        &dir(),
        &RunsHistoryRequest {
            mode: Some(ThreadMode::Orchestration),
            ..Default::default()
        },
    );
    assert_eq!(
        orch.runs
            .iter()
            .map(|r| r.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a"]
    );

    // A run whose thread left the store reads as chat.
    let chat = project(
        &runs,
        &dir(),
        &RunsHistoryRequest {
            mode: Some(ThreadMode::Chat),
            ..Default::default()
        },
    );
    assert_eq!(
        chat.runs
            .iter()
            .map(|r| r.run_id.as_str())
            .collect::<Vec<_>>(),
        vec!["b", "d"]
    );

    let all = project(
        &runs,
        &dir(),
        &RunsHistoryRequest {
            only_threaded: Some(false),
            ..Default::default()
        },
    );
    assert_eq!(all.count, 4);

    let one = project(
        &runs,
        &dir(),
        &RunsHistoryRequest {
            thread_id: Some("t-chat".into()),
            ..Default::default()
        },
    );
    assert_eq!(one.count, 1);
}

#[test]
fn thread_rollup_reports_live_phase_then_failure_then_completion() {
    let runs = vec![
        run(
            "a1",
            Some("t-orch"),
            "researcher",
            AgentRunStatus::Completed,
            10,
        ),
        run("a2", Some("t-orch"), "planner", AgentRunStatus::Running, 20),
        run(
            "b1",
            Some("t-chat"),
            "researcher",
            AgentRunStatus::Completed,
            5,
        ),
        run(
            "b2",
            Some("t-chat"),
            "tools_agent",
            AgentRunStatus::Failed,
            6,
        ),
    ];
    let history = project(&runs, &dir(), &RunsHistoryRequest::default());
    let orch = history
        .threads
        .iter()
        .find(|t| t.thread_id == "t-orch")
        .unwrap();
    assert_eq!(orch.phase, RunPhase::Planning);
    assert_eq!(
        (orch.run_count, orch.active_count, orch.failed_count),
        (2, 1, 0)
    );
    let chat = history
        .threads
        .iter()
        .find(|t| t.thread_id == "t-chat")
        .unwrap();
    assert_eq!(chat.phase, RunPhase::Failed);
    assert_eq!(chat.failed_count, 1);

    let done = project(
        &[run(
            "z",
            Some("t-orch"),
            "researcher",
            AgentRunStatus::Completed,
            1,
        )],
        &dir(),
        &RunsHistoryRequest::default(),
    );
    assert_eq!(done.threads[0].phase, RunPhase::Completed);
}

#[test]
fn long_summaries_are_bounded() {
    let mut r = run(
        "r",
        Some("t-orch"),
        "researcher",
        AgentRunStatus::Completed,
        1,
    );
    r.summary = Some("x".repeat(5000));
    let history = project(&[r], &dir(), &RunsHistoryRequest::default());
    let summary = history.runs[0].summary.as_deref().unwrap();
    assert!(summary.chars().count() <= SUMMARY_MAX_CHARS + 1);
    assert!(summary.ends_with('…'));
}
