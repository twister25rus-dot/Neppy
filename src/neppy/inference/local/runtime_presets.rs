//! Local-model *runtime* presets: what a user picks instead of seven dials.
//!
//! Distinct from [`inference::presets`](crate::neppy::inference::presets), which
//! recommends *which model* to install for a machine. This decides how the model
//! already chosen is run, per request.
//!
//! The settings that make a local model behave well — context window, KV-cache
//! precision, reasoning effort, output ceiling — interact, and the right
//! combination depends on the machine as much as the task. Asking a user to
//! choose each one independently asks them to hold that whole model in their
//! head, and the usual outcome is a context so large the KV cache evicts the
//! conversation, or a budget so small the answer is cut off.
//!
//! So the surface is six named intents, and each one resolves to a complete,
//! coherent set. `Auto` is the default and the only one that inspects the
//! request: see [`AutoInputs`].
//!
//! Presets are advice, not a contract. Every resolved setting is still bounded
//! by what the model can actually do — a preset asking for 64K on a model whose
//! native limit is 8K resolves to 8K, because exceeding it is not a degraded
//! answer but a failed request.

use serde::{Deserialize, Serialize};

/// How hard the model should think.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningLevel {
    Low,
    Normal,
    High,
    Maximum,
}

/// KV-cache precision. Lower costs memory, not much quality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KvPrecision {
    /// Cheapest; for long context or when memory is tight.
    FourBit,
    /// The default: the quality difference from fp16 is not worth the memory.
    EightBit,
    /// Only where quality is worth the cost.
    Fp16,
}

/// What to give up first when memory runs short.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryPolicy {
    Low,
    Balanced,
    Adaptive,
    ContextPriority,
    QualityPriority,
}

/// The six choices a user actually sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    /// Decide per request. The default, and the only adaptive one.
    #[default]
    Auto,
    Fast,
    Balanced,
    Deep,
    LongContext,
    MaximumQuality,
}

/// A preset resolved into the settings a request actually carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedSettings {
    pub reasoning: ReasoningLevel,
    pub context_tokens: u32,
    pub kv_cache: bool,
    pub kv_precision: KvPrecision,
    pub max_output_tokens: u32,
    pub concurrency: u8,
    pub memory_policy: MemoryPolicy,
}

/// What `Auto` looks at. Everything here is an estimate; none of it is trusted
/// beyond ordering decisions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoInputs {
    /// Rough size of what is being sent, in tokens.
    pub prompt_tokens: u32,
    /// Whether the task looks like one that benefits from thinking — coding,
    /// analysis, multi-step work — as judged by the caller.
    pub complex_task: bool,
    /// Free memory in GiB, when it could be measured.
    pub available_memory_gib: Option<f64>,
    /// The model's own context limit. Nothing may exceed it.
    pub model_context_limit: u32,
}

/// Below this, memory is tight enough to start giving things up.
const MEMORY_PRESSURE_GIB: f64 = 8.0;
/// A prompt past this is "long context" regardless of what was asked for.
const LONG_CONTEXT_TOKENS: u32 = 24_000;

impl Preset {
    /// Every preset, in the order the UI lists them.
    pub const ALL: [Preset; 6] = [
        Preset::Auto,
        Preset::Fast,
        Preset::Balanced,
        Preset::Deep,
        Preset::LongContext,
        Preset::MaximumQuality,
    ];

    /// The wire/config name.
    pub fn as_str(self) -> &'static str {
        match self {
            Preset::Auto => "auto",
            Preset::Fast => "fast",
            Preset::Balanced => "balanced",
            Preset::Deep => "deep",
            Preset::LongContext => "long_context",
            Preset::MaximumQuality => "maximum_quality",
        }
    }

    /// Parse a stored or wire value, tolerating the spellings a UI may send.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value
            .trim()
            .to_ascii_lowercase()
            .replace([' ', '-'], "_")
            .as_str()
        {
            "auto" => Some(Preset::Auto),
            "fast" => Some(Preset::Fast),
            "balanced" => Some(Preset::Balanced),
            "deep" => Some(Preset::Deep),
            "long_context" => Some(Preset::LongContext),
            "maximum_quality" | "max_quality" => Some(Preset::MaximumQuality),
            _ => None,
        }
    }

    /// Resolve to concrete settings.
    ///
    /// Every result is clamped to the model's own context limit, because a
    /// preset that asks for more than the model has does not degrade — it
    /// fails.
    pub fn resolve(self, inputs: AutoInputs) -> ResolvedSettings {
        let settings = match self {
            Preset::Auto => return resolve_auto(inputs),
            Preset::Fast => ResolvedSettings {
                reasoning: ReasoningLevel::Low,
                context_tokens: 8_192,
                kv_cache: true,
                kv_precision: KvPrecision::FourBit,
                max_output_tokens: 2_048,
                concurrency: 1,
                memory_policy: MemoryPolicy::Low,
            },
            Preset::Balanced => ResolvedSettings {
                reasoning: ReasoningLevel::Normal,
                context_tokens: 16_384,
                kv_cache: true,
                kv_precision: KvPrecision::EightBit,
                max_output_tokens: 4_096,
                concurrency: 1,
                memory_policy: MemoryPolicy::Balanced,
            },
            Preset::Deep => ResolvedSettings {
                reasoning: ReasoningLevel::High,
                context_tokens: 32_768,
                kv_cache: true,
                kv_precision: KvPrecision::EightBit,
                max_output_tokens: 8_192,
                concurrency: 1,
                memory_policy: MemoryPolicy::Balanced,
            },
            Preset::LongContext => ResolvedSettings {
                reasoning: ReasoningLevel::Normal,
                // "64K or the model's max" — the clamp below settles which.
                context_tokens: 65_536,
                kv_cache: true,
                kv_precision: KvPrecision::FourBit,
                max_output_tokens: 4_096,
                concurrency: 1,
                memory_policy: MemoryPolicy::ContextPriority,
            },
            Preset::MaximumQuality => ResolvedSettings {
                reasoning: ReasoningLevel::Maximum,
                context_tokens: 32_768,
                kv_cache: true,
                kv_precision: KvPrecision::Fp16,
                max_output_tokens: 8_192,
                concurrency: 1,
                memory_policy: MemoryPolicy::QualityPriority,
            },
        };
        clamp_to_model(settings, inputs)
    }
}

/// The Auto router: pick per request rather than per preference.
///
/// The rules are ordered the way the trade-offs actually bite. Context is
/// decided first because it is bounded by the model and by what was sent;
/// reasoning is raised only where it pays; and memory pressure is applied last,
/// because it can override either of the first two and needs to see them.
fn resolve_auto(inputs: AutoInputs) -> ResolvedSettings {
    let long_prompt = inputs.prompt_tokens >= LONG_CONTEXT_TOKENS;
    let tight = inputs
        .available_memory_gib
        .is_some_and(|gib| gib < MEMORY_PRESSURE_GIB);

    // Enough room for what was sent plus an answer, without reserving the whole
    // window for a prompt that does not need it.
    let wanted_context = if long_prompt {
        65_536
    } else if inputs.complex_task {
        32_768
    } else {
        16_384
    };

    // Thinking costs tokens and time, so it is raised only for work that uses
    // it — never merely because a prompt is large.
    let reasoning = if inputs.complex_task {
        ReasoningLevel::High
    } else {
        ReasoningLevel::Normal
    };

    // 8-bit by default; 4-bit where the cache is about to be the problem.
    let kv_precision = if tight || long_prompt {
        KvPrecision::FourBit
    } else {
        KvPrecision::EightBit
    };

    let max_output_tokens = if inputs.complex_task { 8_192 } else { 4_096 };

    let settings = ResolvedSettings {
        reasoning,
        context_tokens: wanted_context,
        kv_cache: true,
        kv_precision,
        max_output_tokens,
        concurrency: 1,
        memory_policy: MemoryPolicy::Adaptive,
    };
    let settings = clamp_to_model(settings, inputs);

    if tight {
        // Under pressure, give up reasoning before context: a truncated prompt
        // changes the answer, while less thinking only makes it shallower.
        ResolvedSettings {
            reasoning: match settings.reasoning {
                ReasoningLevel::Maximum | ReasoningLevel::High => ReasoningLevel::Normal,
                other => other,
            },
            kv_precision: KvPrecision::FourBit,
            ..settings
        }
    } else {
        settings
    }
}

/// Clamp to what the model can actually do, and keep room for the answer.
fn clamp_to_model(settings: ResolvedSettings, inputs: AutoInputs) -> ResolvedSettings {
    let limit = inputs.model_context_limit.max(1);
    let context_tokens = settings.context_tokens.min(limit);

    // The output budget has to fit inside the window alongside the prompt, or
    // generation stops mid-answer. Never below a floor that can still say
    // something useful.
    let room_for_output = context_tokens.saturating_sub(inputs.prompt_tokens);
    let max_output_tokens = settings
        .max_output_tokens
        .min(room_for_output.max(512))
        .max(512);

    ResolvedSettings {
        context_tokens,
        max_output_tokens,
        ..settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> AutoInputs {
        AutoInputs {
            prompt_tokens: 1_000,
            complex_task: false,
            available_memory_gib: Some(32.0),
            model_context_limit: 131_072,
        }
    }

    #[test]
    fn every_preset_round_trips_through_its_wire_name() {
        for preset in Preset::ALL {
            assert_eq!(Preset::from_wire(preset.as_str()), Some(preset));
        }
        assert_eq!(Preset::from_wire("Long Context"), Some(Preset::LongContext));
        assert_eq!(
            Preset::from_wire("max-quality"),
            Some(Preset::MaximumQuality)
        );
        assert_eq!(Preset::from_wire("sideways"), None);
        assert_eq!(
            Preset::default(),
            Preset::Auto,
            "Auto is the default preset"
        );
    }

    #[test]
    fn a_preset_never_exceeds_the_models_own_context_limit() {
        // Long Context asks for 64K; an 8K model has 8K, and asking for more is
        // a failed request rather than a worse answer.
        let small_model = AutoInputs {
            model_context_limit: 8_192,
            ..inputs()
        };

        for preset in Preset::ALL {
            let resolved = preset.resolve(small_model);
            assert!(
                resolved.context_tokens <= 8_192,
                "{preset:?} resolved to {} on an 8K model",
                resolved.context_tokens
            );
        }
    }

    #[test]
    fn the_output_budget_leaves_room_inside_the_window() {
        // A 30K prompt in a 32K window cannot also have an 8K answer.
        let resolved = Preset::Deep.resolve(AutoInputs {
            prompt_tokens: 30_000,
            model_context_limit: 32_768,
            ..inputs()
        });

        assert!(
            resolved.context_tokens >= resolved.max_output_tokens + 1,
            "output budget must fit beside the prompt"
        );
        assert!(
            resolved.max_output_tokens >= 512,
            "never below a usable floor"
        );
    }

    #[test]
    fn fast_is_cheap_and_maximum_quality_is_not() {
        let fast = Preset::Fast.resolve(inputs());
        let best = Preset::MaximumQuality.resolve(inputs());

        assert_eq!(fast.reasoning, ReasoningLevel::Low);
        assert_eq!(fast.kv_precision, KvPrecision::FourBit);
        assert_eq!(best.reasoning, ReasoningLevel::Maximum);
        assert_eq!(best.kv_precision, KvPrecision::Fp16);
        assert!(best.context_tokens > fast.context_tokens);
    }

    #[test]
    fn auto_raises_reasoning_only_for_work_that_uses_it() {
        let simple = Preset::Auto.resolve(inputs());
        let complex = Preset::Auto.resolve(AutoInputs {
            complex_task: true,
            ..inputs()
        });

        assert_eq!(simple.reasoning, ReasoningLevel::Normal);
        assert_eq!(complex.reasoning, ReasoningLevel::High);
        assert!(complex.context_tokens > simple.context_tokens);
    }

    #[test]
    fn auto_widens_the_window_for_a_long_prompt_without_calling_it_complex() {
        let long = Preset::Auto.resolve(AutoInputs {
            prompt_tokens: 40_000,
            ..inputs()
        });

        assert!(long.context_tokens >= 40_000, "the prompt has to fit");
        assert_eq!(
            long.reasoning,
            ReasoningLevel::Normal,
            "a big prompt is not by itself a hard task"
        );
        assert_eq!(
            long.kv_precision,
            KvPrecision::FourBit,
            "cheaper cache when long"
        );
    }

    #[test]
    fn auto_gives_up_thinking_before_context_when_memory_is_tight() {
        let tight = Preset::Auto.resolve(AutoInputs {
            complex_task: true,
            available_memory_gib: Some(4.0),
            ..inputs()
        });
        let roomy = Preset::Auto.resolve(AutoInputs {
            complex_task: true,
            ..inputs()
        });

        assert_eq!(
            tight.reasoning,
            ReasoningLevel::Normal,
            "less thinking is a shallower answer; a truncated prompt is a different one"
        );
        assert_eq!(tight.context_tokens, roomy.context_tokens);
        assert_eq!(tight.kv_precision, KvPrecision::FourBit);
    }

    #[test]
    fn unknown_memory_is_not_treated_as_pressure() {
        // A machine that could not report free memory must not be throttled on
        // a guess.
        let unknown = Preset::Auto.resolve(AutoInputs {
            complex_task: true,
            available_memory_gib: None,
            ..inputs()
        });

        assert_eq!(unknown.reasoning, ReasoningLevel::High);
    }

    #[test]
    fn concurrency_stays_at_one_for_local_models() {
        for preset in Preset::ALL {
            assert_eq!(preset.resolve(inputs()).concurrency, 1);
        }
    }
}
