//! Building a step's prompt and reading the model's reply.
//!
//! A prompt is assembled from the task record and the retrieved snippets, and
//! from nothing else: there is no transcript. The goal, the cumulative summary
//! (which the model rewrites each step), the decisions, the changed files, the
//! last test tail and the next step are the whole memory of earlier steps. That
//! is how "summarize older history instead of sending it again" is met by
//! construction, and why the prompt does not grow with the length of a task.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::neppy::config::schema::LocalAssistantConfig;

use super::types::*;

pub(crate) const SYSTEM_PROMPT: &str = "You are a careful coding assistant working on a project \
that lives on disk. You see only short excerpts. Work in small steps. Reply with one JSON object \
and nothing else.";

const REPLY_SCHEMA: &str = r#"{
  "summary": "the UPDATED summary of the whole task so far, at most 1500 characters",
  "decisions": ["decisions made in this step, each one short"],
  "edits": [{"path": "relative/path", "search": "exact text that occurs once", "replace": "new text"}],
  "run_tests": false,
  "next_step": "what the next step should do",
  "done": false,
  "search_queries": ["identifiers or words to look up for the next step"]
}"#;

/// Characters of the goal kept in a prompt.
const GOAL_MAX: usize = 2000;
/// Changed files listed.
const CHANGED_SHOWN: usize = 30;
/// Below this many remaining characters a truncated final snippet is skipped.
const MIN_SNIPPET_CHARS: usize = 200;
/// Decisions a single reply may carry.
const DECISIONS_PER_STEP: usize = 10;
const QUERY_MAX_CHARS: usize = 120;
/// Longest reply considered, bytes.
const REPLY_MAX: usize = 64 * 1024;

/// Characters-per-token, starting from a conservative guess for code and
/// calibrated from the server's reported `prompt_tokens` when it gives one.
#[derive(Debug, Clone)]
pub(crate) struct TokenEstimator {
    chars_per_token: f64,
}

impl Default for TokenEstimator {
    fn default() -> Self {
        Self {
            chars_per_token: 3.0,
        }
    }
}

impl TokenEstimator {
    pub(crate) fn estimate(&self, chars: usize) -> usize {
        (chars as f64 / self.chars_per_token).ceil() as usize
    }

    pub(crate) fn chars_for(&self, tokens: usize) -> usize {
        (tokens as f64 * self.chars_per_token) as usize
    }

    /// Fold in one observation. Clamped so one odd reply cannot make the
    /// estimate absurd in either direction.
    pub(crate) fn calibrate(&mut self, prompt_chars: usize, prompt_tokens: u64) {
        if prompt_tokens == 0 || prompt_chars == 0 {
            return;
        }
        let observed = (prompt_chars as f64 / prompt_tokens as f64).clamp(1.5, 6.0);
        self.chars_per_token = 0.5 * self.chars_per_token + 0.5 * observed;
    }

    pub(crate) fn chars_per_token(&self) -> f64 {
        self.chars_per_token
    }
}

pub(crate) struct PromptInput<'a> {
    pub(crate) task: &'a TaskRecord,
    pub(crate) step_no: u32,
    pub(crate) max_steps: u32,
    pub(crate) edits_allowed: bool,
    pub(crate) tests_available: bool,
    /// Set on the one retry after an unreadable reply.
    pub(crate) correction: Option<&'a str>,
}

pub(crate) struct BuiltPrompt {
    pub(crate) user: String,
    pub(crate) est_tokens: usize,
    pub(crate) snippets_used: usize,
}

impl BuiltPrompt {
    pub(crate) fn chars(&self) -> usize {
        SYSTEM_PROMPT.len() + self.user.len()
    }
}

fn header(input: &PromptInput<'_>) -> String {
    let t = input.task;
    let mut out = String::new();
    out.push_str("GOAL:\n");
    out.push_str(&clip(t.goal.trim(), GOAL_MAX));
    out.push_str(&format!(
        "\n\nThis is step {} of at most {}.\n",
        input.step_no, input.max_steps
    ));
    out.push_str(if input.edits_allowed {
        "Edits are allowed. Each edit's `search` must match exactly one place in the file; an \
         empty `search` creates a new file.\n"
    } else {
        "Edits are NOT allowed in this task. Leave `edits` empty and report findings in \
         `summary`.\n"
    });
    out.push_str(if input.tests_available {
        "A test command is configured. Set `run_tests` to true to run it after your edits.\n"
    } else {
        "No test command is configured. Leave `run_tests` false.\n"
    });
    out.push_str("\nTASK SO FAR (your own notes from earlier steps):\n");
    out.push_str("summary: ");
    if t.summary.trim().is_empty() {
        out.push_str("(nothing yet)");
    } else {
        out.push_str(t.summary.trim());
    }
    out.push('\n');
    if !t.decisions.is_empty() {
        out.push_str("decisions:\n");
        for d in &t.decisions {
            out.push_str(&format!("- {d}\n"));
        }
    }
    if !t.changed_files.is_empty() {
        let skip = t.changed_files.len().saturating_sub(CHANGED_SHOWN);
        out.push_str(&format!(
            "files changed: {}\n",
            t.changed_files[skip..].join(", ")
        ));
    }
    if let Some(test) = &t.last_test {
        out.push_str(&format!(
            "last test run: exit {}{}\n{}\n",
            test.exit_code,
            if test.timed_out { " (timed out)" } else { "" },
            clip_tail(test.tail.trim(), TEST_TAIL_STORE)
        ));
    }
    if !t.next_step.trim().is_empty() {
        out.push_str(&format!("next step you planned: {}\n", t.next_step.trim()));
    }
    if let Some(note) = input.correction {
        out.push_str(&format!(
            "\nYour previous reply could not be used: {note}\n"
        ));
    }
    out
}

fn footer() -> String {
    format!(
        "\nReply with ONE JSON object in exactly this shape and nothing else:\n{REPLY_SCHEMA}\n"
    )
}

fn snippet_block(s: &Snippet, body: &str) -> String {
    let mut block = format!("--- {}:{}-{} ({})\n{}", s.path, s.start, s.end, s.why, body);
    if !block.ends_with('\n') {
        block.push('\n');
    }
    block
}

/// Assemble the prompt. Snippets are added in rank order until the snippet
/// budget, the snippet count, or the prompt's token budget runs out.
pub(crate) fn build_prompt(
    input: &PromptInput<'_>,
    snippets: &[Snippet],
    cfg: &LocalAssistantConfig,
    estimator: &TokenEstimator,
) -> Result<BuiltPrompt> {
    let head = header(input);
    let foot = footer();
    let fixed = SYSTEM_PROMPT.len() + head.len() + foot.len() + 120;
    let budget_chars = estimator.chars_for(cfg.prompt_budget_tokens as usize);
    if fixed >= budget_chars {
        return Err(AssistantError::Invalid(format!(
            "the task notes alone ({fixed} chars) exceed the prompt budget ({budget_chars} chars)"
        )));
    }
    let mut remaining = (budget_chars - fixed).min(cfg.snippet_budget_chars);
    let mut blocks = String::new();
    let mut used = 0usize;
    for s in snippets.iter().take(cfg.max_snippets) {
        let block = snippet_block(s, &s.text);
        if block.len() <= remaining {
            remaining -= block.len();
            blocks.push_str(&block);
            used += 1;
            continue;
        }
        // Fit a cut of it when there is room worth filling, then stop.
        if remaining >= MIN_SNIPPET_CHARS {
            let overhead = snippet_block(s, "").len();
            if remaining > overhead + MIN_SNIPPET_CHARS {
                let mut cut = clip(&s.text, remaining - overhead);
                if let Some(nl) = cut.rfind('\n') {
                    cut.truncate(nl + 1);
                }
                if !cut.is_empty() {
                    blocks.push_str(&snippet_block(s, &cut));
                    used += 1;
                }
            }
        }
        break;
    }
    let mut user = String::with_capacity(head.len() + blocks.len() + foot.len() + 120);
    user.push_str(&head);
    if used > 0 {
        user.push_str(
            "\nRELEVANT PROJECT MATERIAL (excerpts only; the project is on disk and is not shown in full):\n",
        );
        user.push_str(&blocks);
    } else {
        user.push_str(
            "\n(No project excerpts matched. Ask for something to look up in `search_queries`.)\n",
        );
    }
    user.push_str(&foot);
    let est_tokens = estimator.estimate(SYSTEM_PROMPT.len() + user.len());
    if est_tokens > cfg.prompt_budget_tokens as usize {
        return Err(AssistantError::Invalid(format!(
            "prompt estimate {est_tokens} exceeds the budget {}",
            cfg.prompt_budget_tokens
        )));
    }
    Ok(BuiltPrompt {
        user,
        est_tokens,
        snippets_used: used,
    })
}

/// Whether a prompt of `est_prompt` tokens plus the step's completion fits the
/// context window.
pub(crate) fn fits_context(est_prompt: usize, cfg: &LocalAssistantConfig) -> bool {
    est_prompt + cfg.step_max_tokens as usize <= cfg.context_limit_tokens as usize
}

static THINK: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)<think>.*?</think>").expect("think regex"));

/// The first balanced `{...}` starting at `start`, honouring strings.
fn balanced_object(text: &str, start: usize) -> Option<&str> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut escaped = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if in_str {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Read a step plan out of a model reply. Lenient about what surrounds the
/// JSON (prose, code fences, thinking blocks), strict about its content.
pub(crate) fn parse_plan(reply: &str) -> std::result::Result<StepPlan, String> {
    let reply = clip(reply, REPLY_MAX);
    let cleaned = THINK.replace_all(&reply, "");
    let mut last_err = "no JSON object found in the reply".to_string();
    for (tries, (start, _)) in cleaned.match_indices('{').enumerate() {
        if tries >= 20 {
            break;
        }
        let Some(candidate) = balanced_object(&cleaned, start) else {
            continue;
        };
        match serde_json::from_str::<StepPlan>(candidate) {
            Ok(plan) => return normalize(plan),
            Err(err) => last_err = format!("invalid JSON: {err}"),
        }
    }
    Err(last_err)
}

fn normalize(mut plan: StepPlan) -> std::result::Result<StepPlan, String> {
    if plan.edits.len() > EDITS_PER_STEP {
        return Err(format!(
            "too many edits ({}); at most {EDITS_PER_STEP} per step",
            plan.edits.len()
        ));
    }
    for edit in &plan.edits {
        if edit.path.trim().is_empty() {
            return Err("an edit has no path".into());
        }
        if edit.search.len() > EDIT_FIELD_MAX || edit.replace.len() > EDIT_FIELD_MAX {
            return Err(format!(
                "an edit on `{}` is too large; keep search and replace under {EDIT_FIELD_MAX} bytes",
                clip(&edit.path, 80)
            ));
        }
    }
    for edit in &mut plan.edits {
        edit.path = edit.path.trim().to_string();
    }
    plan.summary = clip(plan.summary.trim(), SUMMARY_MAX * 2);
    plan.next_step = clip(plan.next_step.trim(), NEXT_STEP_MAX);
    plan.decisions = plan
        .decisions
        .iter()
        .map(|d| clip(d.trim(), DECISION_MAX_CHARS))
        .filter(|d| !d.is_empty())
        .take(DECISIONS_PER_STEP)
        .collect();
    plan.search_queries = plan
        .search_queries
        .iter()
        .map(|q| clip(q.trim(), QUERY_MAX_CHARS))
        .filter(|q| !q.is_empty())
        .take(QUERIES_PER_STEP)
        .collect();
    if plan.summary.is_empty() && plan.next_step.is_empty() && plan.edits.is_empty() && !plan.done {
        return Err("the plan is empty: no summary, next step or edits".into());
    }
    Ok(plan)
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
