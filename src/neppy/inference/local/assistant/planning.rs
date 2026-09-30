//! Planning: the one place a step calls the model.
//!
//! Retrieval, prompt assembly, the call (with its retries, the fallback and
//! the gate's preemption), reply parsing with one correction, and storing the
//! plan. Everything before `save_plan` is repeatable; nothing after it asks the
//! model again.

use std::sync::Arc;

use super::index::{query_terms, ProjectIndex};
use super::model::{ModelFailure, ModelReply};
use super::prompt::{
    build_prompt, fits_context, parse_plan, PromptInput, TokenEstimator, SYSTEM_PROMPT,
};
use super::runner::{RunEnv, StopSignal};
use super::types::*;

/// Attempts at one model call before the step fails.
const MAX_CALL_ATTEMPTS: u32 = 3;

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
        let reply = match call_model(env, stop, &built.user).await {
            Ok(reply) => reply,
            Err(CallFail::Preempt(why)) => return Ok(PlanStep::Preempted(why)),
            Err(CallFail::Cancelled) => return Ok(PlanStep::Cancelled),
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

async fn call_model(
    env: &RunEnv,
    stop: &StopSignal,
    user: &str,
) -> std::result::Result<ModelReply, CallFail> {
    let max_tokens = env.cfg.step_max_tokens;
    let mut last = String::new();
    for attempt in 1..=MAX_CALL_ATTEMPTS {
        let outcome = tokio::select! {
            biased;
            _ = stop.cancel.cancelled() => return Err(CallFail::Cancelled),
            outcome = env.model.complete(SYSTEM_PROMPT, user, max_tokens) => outcome,
        };
        match outcome {
            Ok(reply) => return Ok(reply),
            Err(ModelFailure::Gate(gate)) => return Err(CallFail::Preempt(format!("{gate:?}"))),
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
        let outcome = tokio::select! {
            biased;
            _ = stop.cancel.cancelled() => return Err(CallFail::Cancelled),
            outcome = fallback.complete(SYSTEM_PROMPT, user, max_tokens) => outcome,
        };
        match outcome {
            Ok(reply) => return Ok(reply),
            Err(ModelFailure::Gate(gate)) => return Err(CallFail::Preempt(format!("{gate:?}"))),
            Err(ModelFailure::Other(msg)) => last = msg,
        }
    }
    Err(CallFail::Failed(last))
}
