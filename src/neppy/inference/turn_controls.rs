//! Per-turn model controls: sampling knobs and reasoning effort.
//!
//! A turn can ask for different sampling and a different amount of thinking
//! than the role's configured defaults, without changing anything persisted.
//! That is the whole of this module: a value object carried on a cloned
//! `Config` for the life of one turn, and the mapping from "how hard should it
//! think" onto what a provider actually reads.
//!
//! ## Why `reasoning_effort` is the portable knob
//!
//! It is not a Neppy invention and not local-only. `mlx_vlm.server` declares
//! `reasoning_effort` on its chat request ("OpenAI-compatible reasoning
//! effort") alongside `enable_thinking` and `thinking_budget`, and answers with
//! `reasoning_content`; the OpenAI reasoning models take the same field name.
//! So one string covers both, and the budget rides along for the servers that
//! honour it. Anything that reads none of them ignores the keys — they travel
//! in `provider_options`, which the OpenAI-compatible transport merges into the
//! request body and leaves otherwise untouched.
//!
//! **The server's own `--thinking-budget` is not this.** That one is a command
//! line argument, so changing it restarts the process and reloads the weights —
//! minutes, for a knob a user expects to flip per message. The request field is
//! what makes per-turn effort possible at all.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// How much thinking a turn asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    /// Answer directly. Maps to `enable_thinking: false` for servers that take
    /// it, which is stronger than a budget of zero: a model told to think with
    /// no room for it can still emit an empty reasoning block and stop.
    Off,
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    /// Parse a wire value, accepting the names a UI would send.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "off" | "none" | "quick" => Some(Self::Off),
            "low" => Some(Self::Low),
            "medium" | "reasoning" => Some(Self::Medium),
            "high" | "deep" => Some(Self::High),
            _ => None,
        }
    }

    /// The `reasoning_effort` string, or `None` when thinking is off.
    fn effort_str(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Low => Some("low"),
            Self::Medium => Some("medium"),
            Self::High => Some("high"),
        }
    }

    /// Thinking-token budget for servers that take one.
    ///
    /// Deliberately absent for `Off` (there is nothing to bound) and for `High`
    /// (a ceiling there would contradict the ask — a server's own default is a
    /// better answer than a number invented here).
    fn thinking_budget(self) -> Option<u32> {
        match self {
            Self::Off | Self::High => None,
            Self::Low => Some(1_024),
            Self::Medium => Some(4_096),
        }
    }
}

/// Sampling and reasoning a single turn asked for. Every field is optional:
/// unset means "whatever the role already resolves to".
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TurnModelControls {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub max_tokens: Option<u32>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl TurnModelControls {
    /// Whether this carries nothing, so callers can skip the wrapper entirely
    /// rather than installing one that would copy a request and change nothing.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Provider options for the reasoning ask, or `Value::Null` when the turn
    /// did not express one.
    ///
    /// `enable_thinking` is sent in both directions rather than only when
    /// switching thinking off: a server started with thinking disabled needs
    /// the positive form to turn it on for this request, and one started with
    /// it on needs the negative form to turn it off.
    pub fn provider_options(&self) -> Value {
        let Some(effort) = self.reasoning_effort else {
            return Value::Null;
        };
        let mut options = json!({ "enable_thinking": effort != ReasoningEffort::Off });
        let map = options
            .as_object_mut()
            .expect("json! built an object literal");
        if let Some(effort_str) = effort.effort_str() {
            map.insert("reasoning_effort".to_string(), json!(effort_str));
        }
        if let Some(budget) = effort.thinking_budget() {
            map.insert("thinking_budget".to_string(), json!(budget));
        }
        options
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_controls_carry_nothing() {
        assert!(TurnModelControls::default().is_empty());
        assert_eq!(TurnModelControls::default().provider_options(), Value::Null);
    }

    #[test]
    fn effort_travels_as_the_portable_field_plus_a_budget() {
        let controls = TurnModelControls {
            reasoning_effort: Some(ReasoningEffort::Medium),
            ..TurnModelControls::default()
        };

        assert_eq!(
            controls.provider_options(),
            json!({ "enable_thinking": true, "reasoning_effort": "medium", "thinking_budget": 4096 })
        );
    }

    #[test]
    fn high_effort_sends_no_budget_ceiling() {
        let controls = TurnModelControls {
            reasoning_effort: Some(ReasoningEffort::High),
            ..TurnModelControls::default()
        };

        assert_eq!(
            controls.provider_options(),
            json!({ "enable_thinking": true, "reasoning_effort": "high" }),
            "a ceiling would contradict the ask; the server's own default is better"
        );
    }

    #[test]
    fn off_disables_thinking_rather_than_budgeting_it_to_nothing() {
        let controls = TurnModelControls {
            reasoning_effort: Some(ReasoningEffort::Off),
            ..TurnModelControls::default()
        };

        assert_eq!(
            controls.provider_options(),
            json!({ "enable_thinking": false }),
            "a model told to think with no room can still emit an empty block"
        );
    }

    #[test]
    fn wire_values_cover_the_names_a_composer_would_send() {
        assert_eq!(
            ReasoningEffort::from_wire("quick"),
            Some(ReasoningEffort::Off)
        );
        assert_eq!(
            ReasoningEffort::from_wire("Reasoning"),
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(
            ReasoningEffort::from_wire(" HIGH "),
            Some(ReasoningEffort::High)
        );
        assert_eq!(ReasoningEffort::from_wire("sideways"), None);
    }

    #[test]
    fn sampling_knobs_are_independent_of_the_reasoning_ask() {
        let controls = TurnModelControls {
            temperature: Some(0.2),
            top_p: Some(0.9),
            max_tokens: Some(512),
            reasoning_effort: None,
        };

        assert!(!controls.is_empty());
        assert_eq!(
            controls.provider_options(),
            Value::Null,
            "sampling rides the typed request fields, not provider options"
        );
    }
}
