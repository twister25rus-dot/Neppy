//! Migration 10 → 11: point managed role providers at the runtime that exists.
//!
//! Neppy is a fork cut off from the hosted backend, but the settings panel
//! still writes the managed provider verbatim for a role left on its default
//! (`chat_provider = "openhuman"`, and the same for every other workload).
//! Those roles built a client for `DEFAULT_API_BASE_URL`, where nothing
//! listens in this build, and every turn died mid-stream with a connection
//! error that had nothing to do with the turn.
//!
//! The factory now redirects such a role at construction time, so chat works
//! either way. This migration makes the *stored* config say what actually
//! runs: the settings panel, the model picker and the logs name the real
//! provider (`mlx:…` / `ollama:…`) instead of a backend that does not exist.
//!
//! ## Behaviour
//!
//! - Pure in-memory mutation of `Config`. The caller (`migrations::run_pending`)
//!   persists the result via `Config::save()` and bumps `schema_version`.
//! - Rewrites only fields whose value is exactly the managed provider. A BYOK
//!   slug, a local runtime string, and an unset (`None`) route are all left
//!   alone — `None` already resolves through the redirect.
//! - No-op when nothing reachable is configured (no usable provider entry and
//!   no local model). Inventing a route the user never chose would replace an
//!   honest "configure a provider" error with a confusing 404 from a runtime
//!   that was never set up.
//! - Idempotent: a second run finds no managed values left to rewrite.

use crate::neppy::config::Config;
use crate::neppy::inference::provider::factory::{
    neppy_active_provider_string, PROVIDER_OPENHUMAN,
};

/// Counters returned by [`run`] for diagnostics.
#[derive(Debug, Default, Clone)]
pub struct MigrationStats {
    /// Number of role provider fields rewritten off the managed backend.
    pub roles_rerouted: usize,
    /// The provider string the rewritten roles now carry, if any ran.
    pub target: Option<String>,
}

/// Rewrite every managed role provider to the active local/BYOK provider.
///
/// Synchronous — pure config mutation, no I/O. Caller persists via
/// `Config::save()` once `schema_version` is also bumped.
pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let Some(target) = neppy_active_provider_string(config) else {
        log::debug!(
            "[migrations][route-managed-roles] no reachable provider is configured — \
             leaving managed roles alone so the existing setup error still surfaces"
        );
        return Ok(MigrationStats::default());
    };

    let mut stats = MigrationStats::default();
    {
        // Every workload route the settings panel can park on the managed
        // backend. `embeddings_provider` is deliberately absent: embeddings are
        // dimension-bound to the memory tree (fixed at 1024), so repointing them
        // at a chat model would silently invalidate stored vectors.
        let roles: [(&str, &mut Option<String>); 9] = [
            ("chat", &mut config.chat_provider),
            ("reasoning", &mut config.reasoning_provider),
            ("agentic", &mut config.agentic_provider),
            ("coding", &mut config.coding_provider),
            ("vision", &mut config.vision_provider),
            ("memory", &mut config.memory_provider),
            ("heartbeat", &mut config.heartbeat_provider),
            ("learning", &mut config.learning_provider),
            ("subconscious", &mut config.subconscious_provider),
        ];
        for (name, slot) in roles {
            if slot.as_deref().map(str::trim) == Some(PROVIDER_OPENHUMAN) {
                log::info!(
                    "[migrations][route-managed-roles] {name}_provider: \
                     '{PROVIDER_OPENHUMAN}' -> '{target}'"
                );
                *slot = Some(target.clone());
                stats.roles_rerouted += 1;
            }
        }
    }

    if stats.roles_rerouted > 0 {
        stats.target = Some(target);
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config whose roles are parked on the managed backend, with a local
    /// runtime available to redirect onto — the shape every Neppy install has
    /// until the user routes a workload by hand.
    fn managed_config() -> Config {
        let mut config = Config::default();
        config.chat_provider = Some(PROVIDER_OPENHUMAN.to_string());
        config.reasoning_provider = Some(PROVIDER_OPENHUMAN.to_string());
        config.vision_provider = Some(PROVIDER_OPENHUMAN.to_string());
        config.local_ai.provider = "mlx".to_string();
        config.local_ai.chat_model_id = "LiquidAI/LFM2.5-1.2B-Instruct-MLX-4bit".to_string();
        config
    }

    #[test]
    fn reroutes_managed_roles_to_the_local_runtime() {
        let mut config = managed_config();

        let stats = run(&mut config).expect("migration should succeed");

        assert_eq!(stats.roles_rerouted, 3);
        assert_eq!(
            stats.target.as_deref(),
            Some("mlx:LiquidAI/LFM2.5-1.2B-Instruct-MLX-4bit")
        );
        assert_eq!(
            config.chat_provider.as_deref(),
            Some("mlx:LiquidAI/LFM2.5-1.2B-Instruct-MLX-4bit")
        );
        assert_eq!(
            config.reasoning_provider.as_deref(),
            Some("mlx:LiquidAI/LFM2.5-1.2B-Instruct-MLX-4bit")
        );
    }

    #[test]
    fn prefers_the_model_ticked_into_the_mlx_server_chat_slot() {
        // The MLX panel writes the chat slot on the server block; that is a
        // newer statement of "the MLX chat model" than the local-AI ids.
        let mut config = managed_config();
        config.mlx.servers[0].model = "ornith-ai/Ornith-1.5-9B-MLX-8bit".to_string();

        run(&mut config).expect("migration should succeed");

        assert_eq!(
            config.chat_provider.as_deref(),
            Some("mlx:ornith-ai/Ornith-1.5-9B-MLX-8bit")
        );
    }

    #[test]
    fn leaves_byok_and_local_routes_alone() {
        let mut config = managed_config();
        config.reasoning_provider = Some("openai:gpt-4o".to_string());
        config.coding_provider = Some("ollama:qwen3:8b".to_string());
        config.agentic_provider = None;

        let stats = run(&mut config).expect("migration should succeed");

        assert_eq!(stats.roles_rerouted, 2, "only chat + vision were managed");
        assert_eq!(config.reasoning_provider.as_deref(), Some("openai:gpt-4o"));
        assert_eq!(config.coding_provider.as_deref(), Some("ollama:qwen3:8b"));
        assert_eq!(config.agentic_provider, None);
    }

    #[test]
    fn is_a_no_op_when_nothing_reachable_is_configured() {
        let mut config = Config::default();
        config.chat_provider = Some(PROVIDER_OPENHUMAN.to_string());
        config.local_ai.chat_model_id = String::new();
        config.local_ai.model_id = String::new();

        let stats = run(&mut config).expect("migration should succeed");

        assert_eq!(stats.roles_rerouted, 0);
        assert_eq!(
            config.chat_provider.as_deref(),
            Some(PROVIDER_OPENHUMAN),
            "an unreachable rewrite would hide the real setup error"
        );
    }

    #[test]
    fn is_idempotent() {
        let mut config = managed_config();

        run(&mut config).expect("first run");
        let second = run(&mut config).expect("second run");

        assert_eq!(second.roles_rerouted, 0);
        assert_eq!(second.target, None);
    }
}
