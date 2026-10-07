//! Known model context-window sizes for pre-inference budgeting.
//!
//! Provider `/models` responses may include `context_length` / `context_window`,
//! but the agent harness must enforce limits **before** the first dispatch —
//! otherwise long histories produce upstream `400 Bad Request` errors when usage
//! metadata is not yet available.

use crate::neppy::config::{
    MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CHAT_V1, MODEL_CODING_V1, MODEL_REASONING_QUICK_V1,
    MODEL_REASONING_V1,
};

/// Conservative default for Neppy abstract tier models (tokens).
const TIER_LARGE_CONTEXT: u64 = 200_000;
/// Reasoning tier — backed by a 1M-context model.
const TIER_REASONING_CONTEXT: u64 = 1_000_000;
const TIER_STANDARD_CONTEXT: u64 = 128_000;
const TIER_LOCAL_CONTEXT: u64 = 8_192;

/// Last-resort context window (tokens) for a **local** provider when neither the
/// static model table nor the provider profile declares one.
///
/// Local runtimes (llama.cpp / vLLM via `LocalProviderKind::LocalOpenai`, whose
/// profile default is `None`) enforce the model's *loaded* `n_ctx` and reject an
/// over-budget prompt with a hard `400` (`n_keep >= n_ctx`) — issue #3550 /
/// Sentry TAURI-RUST-6V0. Returning `None` for those would **disable**
/// pre-dispatch trimming and let the overflow through. So instead of skipping,
/// we trim against this conservative floor: a slightly-too-aggressive trim is
/// strictly better than a guaranteed 400. The value is the smallest real local
/// profile default (Ollama / LM Studio = 8192; MLX has its own, larger guess in
/// `MLX_DEFAULT_CONTEXT_WINDOW`), chosen because any local runtime can hold at
/// least this much, while keeping the floor low enough to actually bound the
/// prompt. Only applied to local providers — cloud providers with an unknown
/// model keep `None` (their windows are large; a tiny floor would needlessly
/// truncate legitimate large-context requests).
///
/// This floor is a **guess**, used for *trimming only*. It is deliberately not
/// fed into the engine's hard un-evictable-prefix abort ("reload with a larger
/// context length") — that abort consults the provider's *authoritative*
/// [`crate::neppy::inference::provider::Provider::loaded_context_window`]
/// instead, so we never reject a request against a guessed window the real
/// loaded `n_ctx` would have accepted (Codex P1 review on PR #3771).
const CONSERVATIVE_LOCAL_CONTEXT_FLOOR: u64 = 4_096;
/// DeepSeek v4 Flash window (~1M tokens) — the shared backing for every managed
/// "flash" tier: `chat-v1`, its legacy alias `reasoning-quick-v1`, and
/// `summarization-v1`. Kept as a single constant so these tiers can't drift
/// apart again (issue #4706: `chat-v1` was pinned at `TIER_STANDARD_CONTEXT`
/// (128K) while `summarization-v1` was 1M, even though both resolve to DeepSeek
/// v4 Flash in the backend model registry). `extract_from_result` also relies on
/// this window to single-shot whole oversized payloads instead of chunking, so
/// it must reflect the real backing model's capacity.
const TIER_FLASH_CONTEXT: u64 = 1_000_000;

/// Resolve the context window (in tokens) for a model id or Neppy tier alias.
///
/// Returns `None` when the model is unknown — callers should skip pre-dispatch
/// trimming rather than guess.
pub fn context_window_for_model(model: &str) -> Option<u64> {
    let normalized = model.trim();
    if normalized.is_empty() {
        return None;
    }

    if let Some(window) = tier_context_window(normalized) {
        return Some(window);
    }

    if let Some(price) = crate::neppy::platform::cost::catalog::lookup(normalized) {
        tracing::debug!(
            model = normalized,
            catalog_model = price.model_id,
            context_window = price.context_window,
            "[model_context] matched cost catalog row"
        );
        return Some(u64::from(price.context_window));
    }

    if let Some(window) = tinyagents::harness::model::context_window_for_model_id(normalized) {
        tracing::debug!(
            model = normalized,
            context_window = window,
            "[model_context] matched tinyagents model context hint"
        );
        return Some(window);
    }

    // The crate's `context_window_for_model_id` resolves the canonical o1/o3
    // ids (`o1`, `o1-mini`, `openai/o1-preview`, …) but does not match an
    // `o1`/`o3` token embedded mid-name (e.g. `ollama/mistral-for-o1-benchmark`).
    // Neppy keeps that segment heuristic host-side to preserve the
    // pre-port behavior (regression guard from PR #2100) until the crate
    // matcher covers internal segments.
    if let Some(window) = o1_o3_segment_context(normalized) {
        tracing::debug!(
            model = normalized,
            context_window = window,
            "[model_context] matched host o1/o3 segment heuristic"
        );
        return Some(window);
    }

    None
}

/// Match a bounded `o1`/`o3` segment anywhere in the model id and map it to the
/// OpenAI reasoning-model 200K window. The token must be delimited by a
/// non-alphanumeric boundary (or string start/end) on both sides so substrings
/// like `solo1-7b`, `proto3-chat`, or `octo3thing` do **not** over-match.
fn o1_o3_segment_context(model: &str) -> Option<u64> {
    const O_SERIES_CONTEXT: u64 = 200_000;
    let bytes = model.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i + 1 < n {
        let is_o = bytes[i] == b'o' || bytes[i] == b'O';
        if is_o && (bytes[i + 1] == b'1' || bytes[i + 1] == b'3') {
            let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            let after = i + 2;
            let after_ok = after >= n || !bytes[after].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return Some(O_SERIES_CONTEXT);
            }
        }
        i += 1;
    }
    None
}

fn tier_context_window(model: &str) -> Option<u64> {
    match model {
        MODEL_REASONING_V1 => Some(TIER_REASONING_CONTEXT),
        MODEL_AGENTIC_V1 | MODEL_CODING_V1 => Some(TIER_LARGE_CONTEXT),
        "summarization-v1" => Some(TIER_FLASH_CONTEXT),
        // Burst tier advertises a 128k window on the managed backend. Matched on
        // the `burst-v1` alias before any substring fallbacks below.
        MODEL_BURST_V1 => Some(TIER_STANDARD_CONTEXT),
        // `chat-v1` (and its legacy alias `reasoning-quick-v1`) are backed by
        // DeepSeek v4 Flash — the same ~1M model as `summarization-v1`, not a
        // 128K model (issue #4706). Share `TIER_FLASH_CONTEXT` so the three
        // flash tiers stay in lockstep.
        MODEL_CHAT_V1 | MODEL_REASONING_QUICK_V1 | "chat" => Some(TIER_FLASH_CONTEXT),
        m if m.starts_with("gemma") || m.contains(":1b") || m.contains("270m") => {
            Some(TIER_LOCAL_CONTEXT)
        }
        _ => None,
    }
}

/// Resolve context window with local provider profile fallback.
///
/// When `context_window_for_model` returns `None` (unknown model name —
/// common for local models like `qwen3:14b`, `phi3:mini`, etc.) this
/// function falls back to the provider profile's declared default context
/// window. This ensures preflight trimming still works for local models
/// even when the exact model name isn't in the static pattern table.
///
/// For a **local** provider this never returns `None`: if the profile itself
/// declares no default (e.g. `LocalProviderKind::LocalOpenai` = llama.cpp /
/// vLLM), it falls back once more to [`CONSERVATIVE_LOCAL_CONTEXT_FLOOR`] so
/// trimming still engages instead of being silently skipped and overflowing
/// the runtime `n_ctx` (issue #3550 / Sentry TAURI-RUST-6V0). `None` is only
/// returned when `local_kind` is `None` (cloud provider, unknown model) —
/// where over-trimming a large window is worse than skipping the trim.
/// Like [`context_window_for_model_with_local_fallback`], but for a LOCAL
/// provider the user's configured `local_ai.num_ctx` wins.
///
/// The profile defaults are static guesses (Ollama declares 8192), and they were
/// the *only* input to the trim budget — so a 32k-context local model was still
/// trimmed against 8192 while `num_ctx` sat in the config doing nothing. A Phase
/// E run showed the consequence directly: `message_trim … tokens_after=22243
/// budget=7373` on a model that had 32768 available, evicting messages that did
/// not need evicting.
///
/// `num_ctx` is the right source of truth here because it is what the runtime is
/// actually told to load — the same value the factory bakes into the Ollama
/// request — so it beats both the static table and the profile default.
pub fn context_window_for_local_with_configured_num_ctx(
    model: &str,
    local_kind: Option<crate::neppy::inference::local::profile::LocalProviderKind>,
    configured_num_ctx: Option<u32>,
) -> Option<u64> {
    if local_kind.is_some() {
        if let Some(n) = configured_num_ctx.filter(|n| *n > 0) {
            tracing::debug!(
                model,
                context_window = n,
                "[model_context] using configured local_ai.num_ctx as the context window"
            );
            return Some(u64::from(n));
        }
    }
    context_window_for_model_with_local_fallback(model, local_kind)
}

pub fn context_window_for_model_with_local_fallback(
    model: &str,
    local_kind: Option<crate::neppy::inference::local::profile::LocalProviderKind>,
) -> Option<u64> {
    if let Some(window) = context_window_for_model(model) {
        return Some(window);
    }
    // Fall back to the local provider profile's default context window.
    if let Some(kind) = local_kind {
        let profile = crate::neppy::inference::local::profile::profile_for_kind(kind);
        if let Some(default_ctx) = profile.default_context_window {
            tracing::debug!(
                model,
                provider = kind.as_str(),
                context_window = default_ctx,
                "[model_context] using local provider profile default context window"
            );
            return Some(default_ctx);
        }
        // Local provider with no declared default (llama.cpp / vLLM). Never
        // return `None` here — that would disable pre-dispatch trimming and let
        // the prompt overflow the runtime `n_ctx` (the TAURI-RUST-6V0 400). Trim
        // against a conservative floor instead.
        tracing::debug!(
            model,
            provider = kind.as_str(),
            context_window = CONSERVATIVE_LOCAL_CONTEXT_FLOOR,
            "[model_context] local provider has no profile default; using conservative context floor"
        );
        return Some(CONSERVATIVE_LOCAL_CONTEXT_FLOOR);
    }
    None
}

/// Where an MLX context window came from (for the debug log and tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MlxWindowSource {
    /// `[[mlx.server]].context_window`.
    Config,
    /// The model's own `config.json` (HF cache or a local model directory).
    ModelConfig,
    /// The static table / profile default.
    Default,
}

impl MlxWindowSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::ModelConfig => "model_config",
            Self::Default => "default",
        }
    }
}

/// Positive `config.json` lookups, keyed by the resolved file path. Only hits are
/// cached: a miss is a couple of `stat`s and the model may be downloaded later.
fn model_config_cache() -> &'static std::sync::Mutex<std::collections::HashMap<String, u64>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, u64>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Context window declared by a Hugging Face `config.json` body: the larger of
/// `max_position_embeddings` and the rope-scaled window
/// (`rope_scaling.original_max_position_embeddings * factor`), looked up at the
/// top level and under `text_config` (multimodal checkpoints nest it there).
pub(crate) fn window_from_model_config(json: &serde_json::Value) -> Option<u64> {
    fn from_section(section: &serde_json::Value) -> Option<u64> {
        let base = section
            .get("max_position_embeddings")
            .and_then(|v| v.as_u64())
            .filter(|n| *n > 0);
        let scaled = ["rope_scaling", "rope_parameters"]
            .iter()
            .filter_map(|key| section.get(*key))
            .find_map(|rope| {
                let factor = rope.get("factor").and_then(|v| v.as_f64())?;
                let original = rope
                    .get("original_max_position_embeddings")
                    .and_then(|v| v.as_u64())?;
                (factor > 1.0 && original > 0).then_some((original as f64 * factor) as u64)
            });
        match (base, scaled) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }
    json.get("text_config")
        .and_then(from_section)
        .or_else(|| from_section(json))
}

/// Locate the model's `config.json`: a local model directory, else the newest
/// snapshot of its repo in the HF cache under `hub_dir`.
fn find_model_config_path(model: &str, hub_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let id = model.trim();
    let id = id.strip_prefix("mlx:").unwrap_or(id).trim();
    if id.is_empty() {
        return None;
    }
    let local = std::path::Path::new(id).join("config.json");
    if local.is_file() {
        return Some(local);
    }
    let snapshots = hub_dir
        .join(format!("models--{}", id.replace('/', "--")))
        .join("snapshots");
    std::fs::read_dir(snapshots)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("config.json"))
        .filter(|path| path.is_file())
        .max_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        })
}

fn model_config_window(model: &str, hub_dir: &std::path::Path) -> Option<u64> {
    let path = find_model_config_path(model, hub_dir)?;
    let key = path.to_string_lossy().into_owned();
    if let Some(hit) = model_config_cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).copied())
    {
        return Some(hit);
    }
    let body = std::fs::read_to_string(&path).ok()?;
    let window = window_from_model_config(&serde_json::from_str(&body).ok()?)?;
    if let Ok(mut cache) = model_config_cache().lock() {
        cache.insert(key, window);
    }
    Some(window)
}

/// Context window for an MLX-served model: the explicit
/// `[[mlx.server]].context_window`, else the model's own `config.json`, else the
/// static table / MLX default — capped by the server's `max_kv_size` when set.
///
/// Pure over its inputs (`hub_dir` is injected) so it is testable without
/// touching the real HF cache.
/// Ceiling for an MLX window read from the model's own `config.json`.
pub const MLX_AUTO_CONTEXT_CAP: u64 = 65_536;

pub fn mlx_context_window(
    model: &str,
    configured_window: u32,
    max_kv_size: u32,
    hub_dir: &std::path::Path,
) -> (u64, MlxWindowSource) {
    let (mut window, source) = if configured_window > 0 {
        (u64::from(configured_window), MlxWindowSource::Config)
    } else if let Some(w) = model_config_window(model, hub_dir) {
        // A model's advertised maximum (often 128k-256k) is not what a local
        // Mac can prefill quickly or hold in KV memory; an explicit
        // `context_window` is the way to go above this.
        (w.min(MLX_AUTO_CONTEXT_CAP), MlxWindowSource::ModelConfig)
    } else {
        let fallback = context_window_for_model(model)
            .unwrap_or(crate::neppy::inference::local::profile::MLX_DEFAULT_CONTEXT_WINDOW);
        (fallback, MlxWindowSource::Default)
    };
    if max_kv_size > 0 {
        window = window.min(u64::from(max_kv_size));
    }
    tracing::debug!(
        model,
        window,
        max_kv_size,
        "[model_context] mlx window={window} source={}",
        source.as_str()
    );
    (window, source)
}

/// [`mlx_context_window`] against the live config and the real HF cache. The
/// server block serving `model` is used, falling back to the first block.
pub fn mlx_context_window_for_config(model: &str, config: &crate::neppy::config::Config) -> u64 {
    let id = model.trim();
    let id = id.strip_prefix("mlx:").unwrap_or(id).trim();
    let server = config
        .mlx
        .servers
        .iter()
        .find(|s| s.model.trim() == id)
        .or_else(|| config.mlx.servers.first());
    let (configured, max_kv) = server
        .map(|s| (s.context_window, s.max_kv_size))
        .unwrap_or((0, 0));
    let hub = crate::neppy::inference::local::service::mlx_admin::models::hf_hub_dir();
    mlx_context_window(model, configured, max_kv, &hub).0
}

/// Whether the model resolved for a chat hint/agent/profile accepts image input
/// according to the **user-configured** vision flag in `config.model_registry`.
///
/// This is the per-model override that lets a user mark a **custom / BYOK** model
/// as vision-capable (Settings → Advanced LLM → custom model → "Supports
/// vision"). Managed-backend models already advertise vision via
/// [`crate::neppy::inference::provider::Provider::supports_vision`]; this flag
/// covers OpenAI-compatible providers the backend can't introspect per-model.
/// Returns `false` for models the user has not flagged.
pub fn model_vision_enabled(model: &str, config: &crate::neppy::config::Config) -> bool {
    let normalized = model.trim();
    if normalized.is_empty() {
        return false;
    }
    let enabled = config
        .model_registry
        .iter()
        .any(|entry| entry.id == normalized && entry.vision);
    tracing::debug!(
        model = normalized,
        vision_enabled = enabled,
        "[model_context] resolved user-configured vision flag"
    );
    enabled
}

/// Whether a resolved model accepts image input. The single predicate shared by
/// the chat UI resolve and the server-side session/sub-agent gates.
///
/// - **Managed Neppy tiers** consult the hardcoded per-tier map
///   ([`crate::neppy::inference::provider::factory::oh_tier_supports_vision`]) —
///   the remote backend does not advertise per-tier capability, so the core owns
///   it. Currently only `reasoning-v1` is vision-capable.
/// - **Custom/BYOK models** consult the user-set `model_registry.vision` flag
///   ([`model_vision_enabled`]).
pub fn model_supports_vision(model: &str, config: &crate::neppy::config::Config) -> bool {
    crate::neppy::inference::provider::factory::oh_tier_supports_vision(model)
        || model_vision_enabled(model, config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_num_ctx_beats_the_static_profile_default_for_local() {
        use crate::neppy::inference::local::profile::LocalProviderKind;
        // Ollama's profile declares 8192. A Phase E run trimmed a 32k-context
        // model against that (budget=7373) while num_ctx=32768 sat unused.
        assert_eq!(
            context_window_for_local_with_configured_num_ctx(
                "qwythos-9b-abliterated-32k",
                Some(LocalProviderKind::Ollama),
                Some(32_768),
            ),
            Some(32_768)
        );
        // Unset / zero falls back to the previous behaviour.
        assert_eq!(
            context_window_for_local_with_configured_num_ctx(
                "qwythos-9b-abliterated-32k",
                Some(LocalProviderKind::Ollama),
                None,
            ),
            context_window_for_model_with_local_fallback(
                "qwythos-9b-abliterated-32k",
                Some(LocalProviderKind::Ollama)
            )
        );
        assert_eq!(
            context_window_for_local_with_configured_num_ctx(
                "qwythos-9b-abliterated-32k",
                Some(LocalProviderKind::Ollama),
                Some(0),
            ),
            context_window_for_model_with_local_fallback(
                "qwythos-9b-abliterated-32k",
                Some(LocalProviderKind::Ollama)
            )
        );
        // A CLOUD provider must be unaffected: num_ctx is a local-runtime knob.
        assert_eq!(
            context_window_for_local_with_configured_num_ctx("gpt-5", None, Some(32_768)),
            context_window_for_model_with_local_fallback("gpt-5", None)
        );
    }

    use crate::neppy::inference::local::profile::LocalProviderKind;

    #[test]
    fn local_fallback_uses_profile_default() {
        // Unknown model with Ollama profile → 8192 default
        assert_eq!(
            context_window_for_model_with_local_fallback(
                "qwen3:14b",
                Some(LocalProviderKind::Ollama)
            ),
            Some(8_192)
        );
        // Unknown model with MLX profile → 32768 default (4096 starved the harness)
        assert_eq!(
            context_window_for_model_with_local_fallback(
                "my-custom-model",
                Some(LocalProviderKind::Mlx)
            ),
            Some(32_768)
        );
        // Unknown model with no local provider → None
        assert_eq!(
            context_window_for_model_with_local_fallback("qwen3:14b", None),
            None
        );
        // Local provider whose profile declares no default (llama.cpp / vLLM via
        // LocalOpenai) → conservative floor, NOT None. None here would disable
        // pre-dispatch trimming and let the prompt overflow the runtime n_ctx
        // (the TAURI-RUST-6V0 400). Must stay bounded.
        assert_eq!(
            context_window_for_model_with_local_fallback(
                "some-unlisted-gguf",
                Some(LocalProviderKind::LocalOpenai)
            ),
            Some(super::CONSERVATIVE_LOCAL_CONTEXT_FLOOR)
        );
        // Known model ignores local fallback
        assert_eq!(
            context_window_for_model_with_local_fallback(
                "llama3:8b",
                Some(LocalProviderKind::Ollama)
            ),
            Some(128_000)
        );
    }

    #[test]
    fn tier_aliases_resolve() {
        assert_eq!(context_window_for_model("reasoning-v1"), Some(1_000_000));
        assert_eq!(context_window_for_model("agentic-v1"), Some(200_000));
        // chat-v1 is backed by DeepSeek v4 Flash (~1M), the same model as
        // summarization-v1 — not a 128K model (issue #4706).
        assert_eq!(context_window_for_model("chat-v1"), Some(1_000_000));
        // Burst tier — 128k on the managed backend. Matched on the alias, not
        // the local-gemma 8k substring arm.
        assert_eq!(context_window_for_model("burst-v1"), Some(128_000));
        // reasoning-quick-v1 is the legacy alias of chat-v1 (backend renamed it
        // 2026-05), so it resolves to the same ~1M flash window.
        assert_eq!(
            context_window_for_model("reasoning-quick-v1"),
            Some(1_000_000)
        );
        // summarization-v1 maps to a ~1M-token flash model so the extractor can
        // single-shot whole oversized payloads.
        assert_eq!(
            context_window_for_model("summarization-v1"),
            Some(1_000_000)
        );
        // The three flash-backed tiers share one window and must not drift.
        assert_eq!(
            context_window_for_model("chat-v1"),
            context_window_for_model("summarization-v1")
        );
    }

    #[test]
    fn copilot_haiku_resolves_to_200k() {
        assert_eq!(
            context_window_for_model("github_copilot/claude-haiku-4.5"),
            Some(200_000)
        );
    }

    #[test]
    fn unknown_model_returns_none() {
        assert_eq!(context_window_for_model("totally-unknown-model-xyz"), None);
    }

    #[test]
    fn empty_model_returns_none() {
        assert_eq!(context_window_for_model("   "), None);
    }

    #[test]
    fn model_vision_enabled_reads_registry_only() {
        use crate::neppy::config::schema::ModelRegistryEntry;
        let mut config = crate::neppy::config::Config::default();
        config.model_registry = vec![
            ModelRegistryEntry {
                id: "my-llava".into(),
                provider: "openai".into(),
                cost_per_1m_output: 0.0,
                vision: true,
                ..Default::default()
            },
            ModelRegistryEntry {
                id: "text-only".into(),
                provider: "openai".into(),
                cost_per_1m_output: 0.0,
                vision: false,
                ..Default::default()
            },
        ];
        assert!(model_vision_enabled("my-llava", &config));
        assert!(!model_vision_enabled("text-only", &config));
        assert!(!model_vision_enabled("unlisted", &config));
        assert!(!model_vision_enabled("   ", &config));
    }

    #[test]
    fn model_supports_vision_combines_tier_map_and_registry() {
        use crate::neppy::config::schema::ModelRegistryEntry;
        let mut config = crate::neppy::config::Config::default();
        config.model_registry = vec![ModelRegistryEntry {
            id: "my-llava".into(),
            provider: "openai".into(),
            cost_per_1m_output: 0.0,
            vision: true,
            ..Default::default()
        }];
        // `reasoning-v1` is the one vision-capable managed tier; the rest are not.
        assert!(model_supports_vision("reasoning-v1", &config));
        assert!(model_supports_vision("hint:reasoning", &config));
        assert!(!model_supports_vision("chat-v1", &config));
        assert!(!model_supports_vision("hint:chat", &config));
        assert!(!model_supports_vision("burst-v1", &config));
        assert!(!model_supports_vision("hint:burst", &config));
        // BYOK model flagged in the registry is vision-capable.
        assert!(model_supports_vision("my-llava", &config));
        // Unlisted custom model is not.
        assert!(!model_supports_vision("gpt-5", &config));
    }

    #[test]
    fn o1_o3_segment_match_does_not_overmatch() {
        // Real OpenAI o1/o3 model ids must still resolve.
        assert_eq!(context_window_for_model("o1"), Some(200_000));
        assert_eq!(context_window_for_model("o1-mini"), Some(200_000));
        assert_eq!(context_window_for_model("o3-mini"), Some(200_000));
        assert_eq!(context_window_for_model("openai/o1-preview"), Some(200_000));

        // Names that merely *contain* the substring "o1" / "o3" must NOT
        // inherit the 200K window (regression guard for PR #2100 review).
        assert_eq!(context_window_for_model("solo1-7b"), None);
        assert_eq!(context_window_for_model("proto3-chat"), None);
        assert_eq!(
            context_window_for_model("ollama/mistral-for-o1-benchmark"),
            Some(200_000),
            "`-o1-` segment should still match"
        );
        assert_eq!(context_window_for_model("octo3thing"), None);
    }

    #[test]
    fn mlx_window_config_wins_then_model_config_then_default_and_kv_caps() {
        let hub = tempfile::tempdir().unwrap();
        let snap = hub
            .path()
            .join("models--acme--Foo-9B-MLX-8bit/snapshots/abc123");
        std::fs::create_dir_all(&snap).unwrap();
        std::fs::write(
            snap.join("config.json"),
            r#"{"model_type":"x","text_config":{"max_position_embeddings":262144}}"#,
        )
        .unwrap();
        let model = "acme/Foo-9B-MLX-8bit";

        // Model config.json (nested text_config) is read, capped for a local Mac.
        assert_eq!(
            mlx_context_window(model, 0, 0, hub.path()),
            (MLX_AUTO_CONTEXT_CAP, MlxWindowSource::ModelConfig)
        );
        // An explicit config value may go above the automatic cap.
        assert_eq!(
            mlx_context_window(model, 131_072, 0, hub.path()),
            (131_072, MlxWindowSource::Config)
        );
        // An explicit config value wins over the model's own.
        assert_eq!(
            mlx_context_window(model, 16_384, 0, hub.path()),
            (16_384, MlxWindowSource::Config)
        );
        // max_kv_size caps whichever source produced the value.
        assert_eq!(mlx_context_window(model, 0, 40_000, hub.path()).0, 40_000);
        assert_eq!(
            mlx_context_window(model, 100_000, 40_000, hub.path()).0,
            40_000
        );
        // Unknown model, empty cache → 32768 default; a larger kv cap does not raise it.
        assert_eq!(
            mlx_context_window("nobody/unknown", 0, 0, hub.path()),
            (32_768, MlxWindowSource::Default)
        );
        assert_eq!(
            mlx_context_window("nobody/unknown", 0, 8_192, hub.path()).0,
            8_192
        );
        assert_eq!(
            mlx_context_window("nobody/unknown", 0, 99_999, hub.path()).0,
            32_768
        );
    }

    #[test]
    fn model_config_window_parses_top_level_nested_and_rope_scaled() {
        let parse = |s: &str| window_from_model_config(&serde_json::from_str(s).unwrap());
        assert_eq!(parse(r#"{"max_position_embeddings": 40960}"#), Some(40_960));
        assert_eq!(
            parse(r#"{"text_config":{"max_position_embeddings": 262144}}"#),
            Some(262_144)
        );
        // yarn: max_position_embeddings is the pre-scaling length.
        assert_eq!(
            parse(
                r#"{"max_position_embeddings": 32768,
                    "rope_scaling":{"type":"yarn","factor":4.0,"original_max_position_embeddings":32768}}"#
            ),
            Some(131_072)
        );
        // llama3-style: the declared length already exceeds orig*factor.
        assert_eq!(
            parse(
                r#"{"max_position_embeddings": 131072,
                    "rope_scaling":{"factor":8.0,"original_max_position_embeddings":8192}}"#
            ),
            Some(131_072)
        );
        assert_eq!(parse(r#"{"hidden_size": 4096}"#), None);
    }
}
