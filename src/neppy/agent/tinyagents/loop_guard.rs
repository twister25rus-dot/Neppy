//! Host-side loop guard for *successful* no-progress tool loops.
//!
//! The crate breakers cover two shapes: the repeated-failure breaker
//! (`RepeatedToolFailureMiddleware`) halts on the same *error* N times, and the
//! successful-repeat tracker (`RepeatProgressMiddleware`) halts the whole turn on
//! the third identical successful batch. Neither lets the model recover: the turn
//! simply stops after the model has already burned the iterations (minutes each,
//! on a local model), and neither sees a *cycle* (`A, B, A, B, …`) because the
//! streak resets whenever the call changes.
//!
//! This guard sits in front of tool execution and does what a person would:
//!
//! 1. **Nudge.** When the incoming call would start another round of a pattern
//!    that already repeated twice with *identical results* (`A, A` then `A`; or
//!    `A, B, A, B` then `A`; cycle lengths 1 to 3), the call is **not executed**.
//!    The model instead gets a tool result saying what it already did, what came
//!    back, and to change approach or answer with what it has.
//! 2. **Stop gracefully.** If the model repeats the pattern again right after the
//!    nudge (or keeps tripping the guard across the turn), the guard latches a
//!    halt summary and pauses the run, exactly like the other breakers, so the
//!    turn ends with an explanation instead of spending the rest of the budget.
//!
//! Only *successful* results are considered: an error resets the history, because
//! failing calls belong to the failure breakers (which also carry the recoverable
//! / transient headroom). Calls whose contract is to be re-invoked identically
//! ([`is_repeat_call_exempt`], i.e. polling) are ignored. A repeat whose result
//! *differs* (a log tail, a status poll making progress) never trips the guard.
//!
//! The decision logic lives in [`LoopGuardState`] with no async or harness types
//! so it is unit-testable on its own; [`LoopGuardMiddleware`] is the thin adapter.

use std::collections::hash_map::DefaultHasher;
use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use async_trait::async_trait;
use serde_json::Value;

use tinyagents::error::Result as TaResult;
use tinyagents::harness::context::RunContext;
use tinyagents::harness::middleware::{MiddlewareToolOutcome, ToolHandler, ToolMiddleware};
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};
use tinyagents::harness::tool::{ToolCall as TaToolCall, ToolResult as TaToolResult};

use super::middleware::is_repeat_call_exempt;
use super::HaltSummarySlot;

/// Recent executed calls remembered for pattern detection.
const HISTORY_CAP: usize = 12;
/// Longest repeating cycle detected (`1` = the same call again, `2` = `A,B,A,B`, …).
const MAX_CYCLE_PERIOD: usize = 3;
/// Rounds of a pattern that must already have happened (with identical results)
/// before the next round is skipped: the third identical call is the first one
/// refused.
const REPEATS_BEFORE_NUDGE: usize = 2;
/// Consecutive trips (no differing call in between) that end the turn: the first
/// trip is the nudge, the second means the nudge was ignored.
const MAX_CONSECUTIVE_TRIPS: u32 = 2;
/// Trips across one turn that end it even if the model varied a call in between.
const MAX_TOTAL_TRIPS: u32 = 4;
/// Characters of the repeated result quoted back to the model / user.
const RESULT_PREVIEW_CHARS: usize = 200;

/// One executed, successful call: tool, canonical-args hash, result hash.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Record {
    tool: String,
    args: u64,
    digest: u64,
    preview: String,
}

/// What the guard decided for an incoming call.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Execute the call normally.
    Proceed,
    /// Skip the call and hand the model this text as the tool result.
    Nudge(String),
    /// Skip the call and end the turn; `summary` becomes the turn's reply.
    Halt { tool_text: String, summary: String },
}

/// The pattern an incoming call would extend.
struct Detected {
    /// Distinct tool names in the repeating block, in order.
    tools: Vec<String>,
    period: usize,
    /// Rounds already completed (the incoming call starts round `rounds + 1`).
    rounds: usize,
    /// Preview of the (identical) last result.
    preview: String,
}

#[derive(Default)]
pub(crate) struct LoopGuardState {
    history: VecDeque<Record>,
    consecutive_trips: u32,
    total_trips: u32,
}

/// Recursively key-sorted JSON text, so `{"a":1,"b":2}` and `{"b":2,"a":1}` match
/// and a `null` / missing argument object equals `{}`.
fn canonical_args(value: &Value) -> String {
    fn canon(v: &Value) -> Value {
        match v {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for k in keys {
                    out.insert(k.clone(), canon(&map[k]));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(canon).collect()),
            other => other.clone(),
        }
    }
    match value {
        Value::Null => "{}".to_string(),
        other => canon(other).to_string(),
    }
}

fn hash_str(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Single-line, length-capped excerpt of a tool result.
fn preview_of(content: &str) -> String {
    let collapsed = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= RESULT_PREVIEW_CHARS {
        return collapsed;
    }
    let head: String = collapsed.chars().take(RESULT_PREVIEW_CHARS).collect();
    format!("{head}…")
}

impl LoopGuardState {
    /// Look for a repeating block at the tail of the history that the incoming
    /// call `(tool, args)` would extend into another round.
    fn detect(&self, tool: &str, args: u64) -> Option<Detected> {
        let n = self.history.len();
        for period in 1..=MAX_CYCLE_PERIOD {
            if n < period * REPEATS_BEFORE_NUDGE {
                continue;
            }
            let block: Vec<&Record> = self.history.range(n - period..n).collect();
            // The incoming call must be the start of the next round.
            if block[0].tool != tool || block[0].args != args {
                continue;
            }
            let mut rounds = 1usize;
            while n >= period * (rounds + 1) {
                let start = n - period * (rounds + 1);
                let earlier = self.history.range(start..start + period);
                if earlier.zip(block.iter()).all(|(a, b)| a == *b) {
                    rounds += 1;
                } else {
                    break;
                }
            }
            if rounds >= REPEATS_BEFORE_NUDGE {
                let mut tools: Vec<String> = Vec::new();
                for rec in &block {
                    if !tools.contains(&rec.tool) {
                        tools.push(rec.tool.clone());
                    }
                }
                return Some(Detected {
                    tools,
                    period,
                    rounds,
                    preview: block[period - 1].preview.clone(),
                });
            }
        }
        None
    }

    /// Decide what to do with an incoming call, updating the trip counters.
    pub(crate) fn check(&mut self, tool: &str, args: &Value) -> Verdict {
        if is_repeat_call_exempt(tool) {
            return Verdict::Proceed;
        }
        let args_hash = hash_str(&canonical_args(args));
        let Some(found) = self.detect(tool, args_hash) else {
            self.consecutive_trips = 0;
            return Verdict::Proceed;
        };
        self.consecutive_trips += 1;
        self.total_trips += 1;
        let names = found
            .tools
            .iter()
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>()
            .join(", ");
        tracing::warn!(
            tool,
            period = found.period,
            rounds = found.rounds,
            consecutive_trips = self.consecutive_trips,
            total_trips = self.total_trips,
            "[loop_guard] repeated tool pattern with identical results; skipping the call"
        );
        let ended =
            self.consecutive_trips >= MAX_CONSECUTIVE_TRIPS || self.total_trips >= MAX_TOTAL_TRIPS;
        if ended {
            let what = if found.period == 1 {
                format!("kept calling {names} with identical arguments")
            } else {
                format!("kept cycling through the same calls ({names})")
            };
            let summary = format!(
                "Stopping: I {what} and got the same result every time ({} rounds), even after \
                 being told to change approach, so I ended the turn instead of repeating it \
                 again. Last result: \"{}\"\n\nNothing further changed from the repeats. Tell me \
                 which file, command or approach to try next, or give more detail, and I will \
                 continue from here.",
                found.rounds + 1,
                found.preview,
            );
            return Verdict::Halt {
                tool_text: format!(
                    "Loop guard: this call was skipped and the turn is ending. {summary}"
                ),
                summary,
            };
        }
        let text = if found.period == 1 {
            format!(
                "Loop guard: this call was NOT run. You already called {names} with these exact \
                 arguments {} times and got the same result each time: \"{}\". Repeating it \
                 cannot give a different answer. Do not repeat it. Use what that result already \
                 tells you, change approach (a different tool, different arguments, or a smaller \
                 step), or answer with what you have.",
                found.rounds, found.preview,
            )
        } else {
            format!(
                "Loop guard: this call was NOT run. You are cycling through the same calls \
                 ({names}) and every round returned identical results (last result: \"{}\"). \
                 Repeating the cycle cannot give a different answer. Stop cycling: use what you \
                 already have, change approach (different tool or arguments), or answer with \
                 what you have.",
                found.preview,
            )
        };
        Verdict::Nudge(text)
    }

    /// Remember an executed call that succeeded.
    pub(crate) fn record_success(&mut self, tool: &str, args: &Value, content: &str) {
        if is_repeat_call_exempt(tool) {
            return;
        }
        if self.history.len() == HISTORY_CAP {
            self.history.pop_front();
        }
        self.history.push_back(Record {
            tool: tool.to_string(),
            args: hash_str(&canonical_args(args)),
            digest: hash_str(content),
            preview: preview_of(content),
        });
    }

    /// An error breaks every pattern: failing calls are the failure breakers' job.
    pub(crate) fn record_failure(&mut self) {
        self.history.clear();
    }
}

/// `wrap_tool` adapter: asks [`LoopGuardState`] before the call runs and records
/// the outcome after it.
pub(crate) struct LoopGuardMiddleware {
    state: Mutex<LoopGuardState>,
    handle: Option<SteeringHandle>,
    halt_summary: HaltSummarySlot,
}

impl LoopGuardMiddleware {
    pub(crate) fn new(handle: Option<SteeringHandle>, halt_summary: HaltSummarySlot) -> Self {
        Self {
            state: Mutex::new(LoopGuardState::default()),
            handle,
            halt_summary,
        }
    }
}

fn skipped(call: TaToolCall, text: String) -> MiddlewareToolOutcome {
    // `error: Some(..)` so the crate's successful-repeat streak resets on a
    // skipped call (it only counts all-successful batches) and the UI shows the
    // call did not run.
    MiddlewareToolOutcome::Result(TaToolResult {
        call_id: call.id,
        name: call.name,
        content: text.clone(),
        raw: None,
        error: Some(text),
        elapsed_ms: 0,
    })
}

#[async_trait]
impl ToolMiddleware<()> for LoopGuardMiddleware {
    fn name(&self) -> &str {
        "loop_guard"
    }

    async fn wrap_tool(
        &self,
        ctx: &mut RunContext<()>,
        state: &(),
        call: TaToolCall,
        next: ToolHandler<'_, (), ()>,
    ) -> TaResult<MiddlewareToolOutcome> {
        let verdict = match self.state.lock() {
            Ok(mut guard) => guard.check(&call.name, &call.arguments),
            Err(_) => Verdict::Proceed,
        };
        match verdict {
            Verdict::Proceed => {}
            Verdict::Nudge(text) => return Ok(skipped(call, text)),
            Verdict::Halt { tool_text, summary } => {
                if let Ok(mut slot) = self.halt_summary.lock() {
                    *slot = Some(summary);
                }
                if let Some(handle) = &self.handle {
                    handle.send(SteeringCommand::Pause);
                }
                return Ok(skipped(call, tool_text));
            }
        }

        let name = call.name.clone();
        let args = call.arguments.clone();
        let outcome = next.run(ctx, state, call).await?;
        if let MiddlewareToolOutcome::Result(res) = &outcome {
            if let Ok(mut guard) = self.state.lock() {
                if res.error.is_some() {
                    guard.record_failure();
                } else {
                    guard.record_success(&name, &args, &res.content);
                }
            }
        }
        Ok(outcome)
    }
}

#[cfg(test)]
#[path = "loop_guard_tests.rs"]
mod tests;
