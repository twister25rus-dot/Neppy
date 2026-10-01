//! Planning: the one place a step calls the model.
//!
//! Retrieval, prompt assembly, the call (with its retries, the fallback and
//! the gate's preemption), reply parsing with one correction, and storing the
//! plan. Everything before `save_plan` is repeatable; nothing after it asks the
//! model again.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use super::super::service::mlx_admin::gate::{pinned_scope, GateError};
use super::index::{query_terms, ProjectIndex};
use super::model::{ModelFailure, ModelReply};
use super::prompt::{
    build_prompt, fits_context, parse_plan, PromptInput, TokenEstimator, SYSTEM_PROMPT,
};
use super::runner::{RunEnv, StopSignal};
use super::types::*;

/// Attempts at one model call before the step fails.
const MAX_CALL_ATTEMPTS: u32 = 3;
/// Times one step's call may be cancelled to make room for chat. After that
/// the call is pinned: it queues like chat and is not cancelled again, so the
/// step is guaranteed to finish however busy the chat is.
pub(super) const MAX_YIELDS_PER_STEP: u32 = 3;
/// Yield counts of steps still being planned, by `task|step`. Process-wide
/// because a yielded task is run again by the controller with a fresh env.
static YIELDS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);
/// Entries kept at most; the map only ever holds steps in flight.
const YIELDS_CAP: usize = 256;

fn yield_key(task_id: &str, step_no: u32) -> String {
    format!("{task_id}|{step_no}")
}

fn yields_so_far(key: &str) -> u32 {
    YIELDS
        .lock()
        .as_ref()
        .and_then(|m| m.get(key).copied())
        .unwrap_or(0)
}

fn note_yield(key: &str) -> u32 {
    let mut guard = YIELDS.lock();
    let map = guard.get_or_insert_with(HashMap::new);
    if map.len() >= YIELDS_CAP && !map.contains_key(key) {
        map.clear();
    }
    let count = map.entry(key.to_string()).or_insert(0);
    *count += 1;
    *count
}

fn forget_yields(key: &str) {
    if let Some(map) = YIELDS.lock().as_mut() {
        map.remove(key);
    }
}

#[cfg(test)]
pub(super) fn yields_for_test(task_id: &str, step_no: u32) -> u32 {
    yields_so_far(&yield_key(task_id, step_no))
}

pub(super) enum PlanStep {
    Planned,
    Preempted(String),
    Cancelled,
    BudgetExhausted,
}

enum CallFail {
    Preempt(String),
    Cancelled,
    Failed(String),
}

/// Retrieve, build the prompt, call the model and store the plan. Returns
/// without a plan only when the call was preempted, cancelled or out of budget.
#[allow(clippy::too_many_arguments)]
pub(super) async fn plan_step(
    env: &RunEnv,
    index: &Arc<ProjectIndex>,
    task: &TaskRecord,
    step_no: u32,
    edits_allowed: bool,
    estimator: &mut TokenEstimator,
    stop: &StopSignal,
) -> Result<PlanStep> {
    match index.refresh_async(&env.cfg).await {
        Ok(stats) => log::debug!(
            "[local_assistant] task {} step {step_no} index refreshed: reread={} changed={} removed={}",
            task.id,
            stats.reread,
            stats.changed,
            stats.removed
        ),
        // A stale index still retrieves; it must not stop the task.
        Err(err) => log::warn!("[local_assistant] index refresh failed: {err}"),
    }
    let previous_queries = if step_no > 1 {
        env.store
            .load_step(&task.id, step_no - 1)?
            .and_then(|s| s.plan)
            .map(|p| p.search_queries)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut texts: Vec<&str> = previous_queries.iter().map(String::as_str).collect();
    texts.push(&task.next_step);
    texts.push(&task.goal);
    let terms = query_terms(&texts);
    let snippets = {
        let index = Arc::clone(index);
        let cfg = env.cfg.clone();
        tokio::task::spawn_blocking(move || index.search(&terms, &cfg))
            .await
            .map_err(|e| AssistantError::Io(e.to_string()))?
            .unwrap_or_else(|err| {
                log::warn!("[local_assistant] retrieval failed: {err}");
                Vec::new()
            })
    };

    let mut correction: Option<String> = None;
    let mut spent: u64 = 0;
    for attempt in 1..=2u32 {
        // What the task may still spend. A step asks for no more than that, so
        // the task budget is a ceiling on tokens generated, not a threshold
        // checked after the fact.
        let remaining = u64::from(env.cfg.task_max_completion_tokens)
            .saturating_sub(task.completion_tokens_used + spent);
        if remaining == 0 {
            env.store.add_tokens(&task.id, spent)?;
            return Ok(PlanStep::BudgetExhausted);
        }
        let max_tokens = step_token_cap(env.cfg.step_max_tokens, remaining);
        let input = PromptInput {
            task,
            step_no,
            max_steps: task.max_steps,
            edits_allowed,
            tests_available: task.test_command.is_some(),
            correction: correction.as_deref(),
        };
        let built = build_prompt(&input, &snippets, &env.cfg, estimator)?;
        if !fits_context(built.est_tokens, &env.cfg) {
            return Err(AssistantError::Invalid(format!(
                "prompt (~{} tokens) plus {} completion tokens exceeds the {} token context limit",
                built.est_tokens, env.cfg.step_max_tokens, env.cfg.context_limit_tokens
            )));
        }
        log::debug!(
            "[local_assistant] task {} step {step_no} attempt {attempt}: prompt~{} tokens, {} snippets, {:.2} chars/token",
            task.id,
            built.est_tokens,
            built.snippets_used,
            estimator.chars_per_token()
        );
        let reply = match call_model(env, stop, &built.user, max_tokens, &task.id, step_no).await {
            Ok(reply) => reply,
            Err(CallFail::Preempt(why)) => {
                // Tokens an earlier attempt of this step already spent count.
                env.store.add_tokens(&task.id, spent)?;
                return Ok(PlanStep::Preempted(why));
            }
            Err(CallFail::Cancelled) => {
                env.store.add_tokens(&task.id, spent)?;
                return Ok(PlanStep::Cancelled);
            }
            Err(CallFail::Failed(msg)) => {
                env.store.add_tokens(&task.id, spent)?;
                return Err(AssistantError::Model(msg));
            }
        };
        if let Some(reported) = reply.prompt_tokens {
            estimator.calibrate(built.chars(), reported);
        }
        let prompt_tokens = reply.prompt_tokens.unwrap_or(built.est_tokens as u64);
        let completion = reply
            .completion_tokens
            .unwrap_or_else(|| estimator.estimate(reply.text.len()) as u64);
        spent += completion;
        if reply.prompt_tokens.is_none() {
            // The gated model records reported usage itself; record the
            // estimate only when there was nothing to report.
            if let Some(metrics) = &env.metrics {
                metrics.set_last_usage(prompt_tokens, completion);
            }
        }
        match parse_plan(&reply.text) {
            Ok(plan) => {
                env.store
                    .save_plan(&task.id, step_no, &plan, prompt_tokens, spent)?;
                forget_yields(&yield_key(&task.id, step_no));
                env.event(
                    "task_checkpoint",
                    format!(
                        "task {} step {step_no} planned prompt_tokens={prompt_tokens} completion_tokens={spent}",
                        task.id
                    ),
                );
                return Ok(PlanStep::Planned);
            }
            Err(why) => {
                log::warn!(
                    "[local_assistant] task {} step {step_no} unusable reply (attempt {attempt}): {why}",
                    task.id
                );
                correction = Some(why);
                let used = task.completion_tokens_used + spent;
                if used >= u64::from(env.cfg.task_max_completion_tokens) {
                    env.store.add_tokens(&task.id, spent)?;
                    return Ok(PlanStep::BudgetExhausted);
                }
            }
        }
    }
    env.store.add_tokens(&task.id, spent)?;
    Err(AssistantError::Model(format!(
        "the model did not return a usable plan: {}",
        correction.unwrap_or_default()
    )))
}

/// `max_tokens` for one call: the per-step limit, capped at what is left of the
/// task's completion-token budget.
pub(super) fn step_token_cap(step_max_tokens: u32, remaining: u64) -> u32 {
    u32::try_from(remaining).map_or(step_max_tokens, |left| step_max_tokens.min(left))
}

/// What a gate cancellation costs and means. A yield is counted against the
/// step; a cancellation that interrupted generation is charged to the task at
/// the call's full `max_tokens`, because the server never reported what it
/// had produced and the task budget is a hard ceiling.
fn gate_cancelled(
    env: &RunEnv,
    task_id: &str,
    key: &str,
    gate: GateError,
    generating: bool,
    max_tokens: u32,
) -> CallFail {
    if gate == GateError::Yielded {
        let n = note_yield(key);
        log::info!("[local_assistant] task {task_id} step call yielded to chat ({n} so far)");
    }
    if generating {
        if let Err(err) = env.store.add_tokens(task_id, u64::from(max_tokens)) {
            log::warn!("[local_assistant] could not charge a cancelled call: {err}");
        } else {
            log::debug!(
                "[local_assistant] task {task_id}: charged {max_tokens} tokens for a cancelled call"
            );
        }
    }
    CallFail::Preempt(format!("{gate:?}"))
}

async fn call_model(
    env: &RunEnv,
    stop: &StopSignal,
    user: &str,
    max_tokens: u32,
    task_id: &str,
    step_no: u32,
) -> std::result::Result<ModelReply, CallFail> {
    let key = yield_key(task_id, step_no);
    let mut last = String::new();
    for attempt in 1..=MAX_CALL_ATTEMPTS {
        let pinned = yields_so_far(&key) >= MAX_YIELDS_PER_STEP;
        let outcome = tokio::select! {
            biased;
            _ = stop.cancel.cancelled() => return Err(CallFail::Cancelled),
            outcome = pinned_scope(pinned, env.model.complete(SYSTEM_PROMPT, user, max_tokens)) => outcome,
        };
        match outcome {
            Ok(reply) => return Ok(reply),
            Err(ModelFailure::Gate(gate)) => {
                return Err(gate_cancelled(env, task_id, &key, gate, false, max_tokens))
            }
            Err(ModelFailure::GateAfterStart(gate)) => {
                return Err(gate_cancelled(env, task_id, &key, gate, true, max_tokens))
            }
            Err(ModelFailure::Other(msg)) => {
                log::warn!("[local_assistant] model call attempt {attempt} failed: {msg}");
                last = msg;
                if attempt < MAX_CALL_ATTEMPTS && !env.retry_delay.is_zero() {
                    tokio::select! {
                        _ = stop.cancel.cancelled() => return Err(CallFail::Cancelled),
                        _ = tokio::time::sleep(env.retry_delay * attempt) => {}
                    }
                }
            }
        }
    }
    // The primary model could not be admitted; try the configured fallback
    // once, and say so.
    if let (Some(fallback), true) = (&env.fallback, last.contains("[mlx:worker]")) {
        log::warn!("[local_assistant] primary model unavailable; trying the fallback model");
        env.event(
            "task_fallback_model",
            "primary model not admitted; using fallback (degraded quality)",
        );
        let pinned = yields_so_far(&key) >= MAX_YIELDS_PER_STEP;
        let outcome = tokio::select! {
            biased;
            _ = stop.cancel.cancelled() => return Err(CallFail::Cancelled),
            outcome = pinned_scope(pinned, fallback.complete(SYSTEM_PROMPT, user, max_tokens)) => outcome,
        };
        match outcome {
            Ok(reply) => return Ok(reply),
            Err(ModelFailure::Gate(gate)) => {
                return Err(gate_cancelled(env, task_id, &key, gate, false, max_tokens))
            }
            Err(ModelFailure::GateAfterStart(gate)) => {
                return Err(gate_cancelled(env, task_id, &key, gate, true, max_tokens))
            }
            Err(ModelFailure::Other(msg)) => last = msg,
        }
    }
    Err(CallFail::Failed(last))
}
