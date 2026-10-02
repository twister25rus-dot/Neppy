//! Cross-thread execution history for the "Agent runs" view.
//!
//! `run_ledger_list` already returns raw ledger rows across threads, but a row
//! is not something a person can read: its `status` is a lifecycle state, its
//! `agentId` is a registry key, and the thread it belongs to is an opaque id.
//! This module is the projection the UI actually wants — one simplified **phase**
//! per run (what the worker is doing in plain terms), one aggregate phase per
//! thread, and the thread's title and operating mode joined in — without
//! changing or duplicating the ledger itself.
//!
//! RPC: `openhuman.subagent_runs_history` (see `subagent_control.rs`).
//!
//! # Phases
//!
//! A *live* run (`pending` / `running`) is classified by the role of its agent:
//!
//! | phase           | agents                                                              |
//! | --------------- | ------------------------------------------------------------------- |
//! | `researching`   | researcher, context_scout, agent_memory, help, vision_agent, flow_discovery, flow_memory_agent |
//! | `planning`      | planner, task_manager_agent, scheduler_agent                        |
//! | `reviewing`     | critic, anything with `review` in its id                            |
//! | `testing`       | anything with `test`, `verify` or `qa` in its id                    |
//! | `implementing`  | every other worker (code_executor, tools_agent, integrations, ...)  |
//!
//! A *settled* run reports its outcome instead of its role, because "what it was
//! doing" stops being interesting the moment it ended: `completed`, `failed`
//! (failed / interrupted), `cancelled`, or `awaiting_user` for a worker parked on
//! a question. The role is still available as `agentId`.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;
use tinyagents::session::run_ledger::{
    list_agent_runs, AgentRun, AgentRunListRequest, AgentRunStatus,
};

use crate::neppy::config::Config;
use crate::neppy::memory::conversations;
use crate::neppy::threads::mode::ThreadMode;

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;
/// How many ledger rows are read before filtering/truncating. The ledger lists
/// newest first; a mode filter can drop rows, so read a wider window than the
/// page the caller asked for.
const LEDGER_WINDOW: u32 = 500;
const SUMMARY_MAX_CHARS: usize = 300;

/// Simplified phase of one run, or the aggregate of a thread's runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    Researching,
    Planning,
    Implementing,
    Testing,
    Reviewing,
    AwaitingUser,
    Completed,
    Failed,
    Cancelled,
}

impl RunPhase {
    fn is_live(self) -> bool {
        matches!(
            self,
            Self::Researching
                | Self::Planning
                | Self::Implementing
                | Self::Testing
                | Self::Reviewing
        )
    }
}

/// Phase for a run with this `agent_id` in `status`.
pub fn phase_for(agent_id: Option<&str>, status: AgentRunStatus) -> RunPhase {
    match status {
        AgentRunStatus::Completed => return RunPhase::Completed,
        AgentRunStatus::Failed | AgentRunStatus::Interrupted => return RunPhase::Failed,
        AgentRunStatus::Cancelled => return RunPhase::Cancelled,
        AgentRunStatus::AwaitingUser | AgentRunStatus::Paused => return RunPhase::AwaitingUser,
        AgentRunStatus::Pending | AgentRunStatus::Running => {}
    }
    let id = agent_id.unwrap_or("").to_ascii_lowercase();
    match id.as_str() {
        "researcher" | "context_scout" | "agent_memory" | "help" | "vision_agent"
        | "flow_discovery" | "flow_memory_agent" => RunPhase::Researching,
        "planner" | "task_manager_agent" | "scheduler_agent" => RunPhase::Planning,
        "critic" => RunPhase::Reviewing,
        _ if id.contains("review") => RunPhase::Reviewing,
        _ if id.contains("test") || id.contains("verify") || id.contains("qa") => RunPhase::Testing,
        _ => RunPhase::Implementing,
    }
}

/// Aggregate phase of a thread from its runs' phases: the most recently updated
/// live run decides; with none live, a failure outranks completion so a thread
/// with a broken worker never reads as "completed".
fn aggregate_phase(runs: &[&RunRow]) -> RunPhase {
    // `runs` is newest-updated first.
    if let Some(live) = runs.iter().find(|r| r.phase.is_live()) {
        return live.phase;
    }
    if runs.iter().any(|r| r.phase == RunPhase::AwaitingUser) {
        return RunPhase::AwaitingUser;
    }
    if runs.iter().any(|r| r.phase == RunPhase::Failed) {
        return RunPhase::Failed;
    }
    if runs.iter().all(|r| r.phase == RunPhase::Cancelled) {
        return RunPhase::Cancelled;
    }
    RunPhase::Completed
}

/// One row of the "Agent runs" list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRow {
    pub run_id: String,
    pub thread_id: Option<String>,
    pub thread_title: Option<String>,
    /// Operating mode of the owning thread (`chat` / `orchestration`).
    pub thread_mode: Option<String>,
    pub agent_id: Option<String>,
    /// Ledger kind: `subagent`, `worker_thread`, `background_agent`,
    /// `team_member` or `workflow_child`.
    pub kind: String,
    /// Raw lifecycle status from the ledger.
    pub status: String,
    pub phase: RunPhase,
    /// Bounded, plain-text outcome (never the prompt).
    pub summary: Option<String>,
    pub error: Option<String>,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub elapsed_ms: i64,
    pub model: Option<String>,
    pub tool_count: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// Per-thread rollup, so the view can group runs under their conversation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRollup {
    pub thread_id: String,
    pub thread_title: Option<String>,
    pub thread_mode: Option<String>,
    pub run_count: usize,
    pub active_count: usize,
    pub failed_count: usize,
    pub phase: RunPhase,
    pub last_updated_at: String,
}

/// Result of `subagent_runs_history`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunsHistory {
    pub runs: Vec<RunRow>,
    pub threads: Vec<ThreadRollup>,
    /// `runs.len()`.
    pub count: usize,
}

/// Filters accepted by the RPC. All optional.
#[derive(Debug, Clone, Default)]
pub struct RunsHistoryRequest {
    pub limit: Option<usize>,
    /// Restrict to runs of one thread.
    pub thread_id: Option<String>,
    /// Restrict to runs whose thread is in this mode.
    pub mode: Option<ThreadMode>,
    /// Raw ledger status filter (`running`, `failed`, ...).
    pub status: Option<String>,
    /// Drop runs that belong to no thread (cron / CLI). Default `true`: the
    /// view is about conversations.
    pub only_threaded: Option<bool>,
}

fn truncate_summary(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= SUMMARY_MAX_CHARS {
        flat
    } else {
        let cut: String = flat.chars().take(SUMMARY_MAX_CHARS).collect();
        format!("{cut}…")
    }
}

fn row_for(run: &AgentRun, threads: &HashMap<String, (String, ThreadMode)>) -> RunRow {
    let now = chrono::Utc::now();
    let end = run.completed_at.unwrap_or(now);
    let thread = run
        .parent_thread_id
        .as_deref()
        .and_then(|id| threads.get(id));
    RunRow {
        run_id: run.id.clone(),
        thread_id: run.parent_thread_id.clone(),
        thread_title: thread.map(|(title, _)| title.clone()),
        thread_mode: thread.map(|(_, mode)| mode.as_str().to_string()),
        agent_id: run.agent_id.clone(),
        kind: run.kind.as_str().to_string(),
        status: run.status.as_str().to_string(),
        phase: phase_for(run.agent_id.as_deref(), run.status),
        summary: run.summary.as_deref().map(truncate_summary),
        error: run.error.as_deref().map(truncate_summary),
        started_at: run.started_at.to_rfc3339(),
        updated_at: run.updated_at.to_rfc3339(),
        completed_at: run.completed_at.map(|t| t.to_rfc3339()),
        elapsed_ms: (end - run.started_at).num_milliseconds().max(0),
        model: run.telemetry.as_ref().and_then(|t| t.model.clone()),
        tool_count: run.telemetry.as_ref().map(|t| t.tool_count),
        cost_usd: run.telemetry.as_ref().map(|t| t.cost_usd),
    }
}

/// Pure projection: ledger rows + thread directory -> history. Split from the
/// I/O so the shape and the phase rules are testable without a database.
pub fn project(
    runs: &[AgentRun],
    threads: &HashMap<String, (String, ThreadMode)>,
    request: &RunsHistoryRequest,
) -> RunsHistory {
    let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let only_threaded = request.only_threaded.unwrap_or(true);

    let mut rows: Vec<RunRow> = runs
        .iter()
        .map(|run| row_for(run, threads))
        .filter(|row| !only_threaded || row.thread_id.is_some())
        .filter(|row| {
            request
                .thread_id
                .as_deref()
                .is_none_or(|id| row.thread_id.as_deref() == Some(id))
        })
        .filter(|row| {
            request.mode.is_none_or(|mode| {
                // A run whose thread is gone from the store reads as chat, the
                // same default `threads` applies.
                row.thread_mode.as_deref().unwrap_or("chat") == mode.as_str()
            })
        })
        .collect();
    rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    rows.truncate(limit);

    let mut by_thread: Vec<(String, Vec<&RunRow>)> = Vec::new();
    for row in &rows {
        let Some(thread_id) = row.thread_id.as_ref() else {
            continue;
        };
        match by_thread.iter_mut().find(|(id, _)| id == thread_id) {
            Some((_, group)) => group.push(row),
            None => by_thread.push((thread_id.clone(), vec![row])),
        }
    }
    let rollups = by_thread
        .into_iter()
        .map(|(thread_id, group)| ThreadRollup {
            thread_title: group[0].thread_title.clone(),
            thread_mode: group[0].thread_mode.clone(),
            run_count: group.len(),
            active_count: group.iter().filter(|r| r.phase.is_live()).count(),
            failed_count: group.iter().filter(|r| r.phase == RunPhase::Failed).count(),
            phase: aggregate_phase(&group),
            last_updated_at: group[0].updated_at.clone(),
            thread_id,
        })
        .collect();

    let count = rows.len();
    RunsHistory {
        runs: rows,
        threads: rollups,
        count,
    }
}

/// Reads the ledger and the thread directory and projects them.
pub async fn runs_history(
    config: &Config,
    request: RunsHistoryRequest,
) -> Result<RunsHistory, String> {
    log::debug!(
        "[runs_history] entry thread={:?} mode={:?} status={:?} limit={:?}",
        request.thread_id,
        request.mode.map(ThreadMode::as_str),
        request.status,
        request.limit
    );
    let ledger_request = AgentRunListRequest {
        status: request.status.clone(),
        parent_thread_id: request.thread_id.clone(),
        limit: Some(LEDGER_WINDOW),
        ..Default::default()
    };
    let workspace = config.workspace_dir.clone();
    let runs = tokio::task::spawn_blocking(move || list_agent_runs(&workspace, &ledger_request))
        .await
        .map_err(|e| format!("run ledger read task failed: {e}"))?
        .map_err(|e| e.to_string())?
        .runs;

    let threads: HashMap<String, (String, ThreadMode)> =
        conversations::blocking::list_threads(config.workspace_dir.clone())
            .await
            .unwrap_or_else(|err| {
                log::warn!("[runs_history] thread directory unavailable, titles omitted: {err}");
                Vec::new()
            })
            .into_iter()
            .map(|t| {
                let mode = ThreadMode::from_labels(&t.labels);
                (t.id, (t.title, mode))
            })
            .collect();

    let history = project(&runs, &threads, &request);
    log::debug!(
        "[runs_history] exit runs={} threads={}",
        history.count,
        history.threads.len()
    );
    Ok(history)
}

/// Serialises a [`RunsHistory`] for the RPC layer.
pub fn to_value(history: &RunsHistory) -> Result<Value, String> {
    serde_json::to_value(history).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "runs_history_tests.rs"]
mod tests;
