//! Unified inference domain.
//!
//! This module is the canonical home for all inference concerns:
//! - `local/`    — Ollama / LM Studio / Piper runtime management
//!                 (was `src/neppy/local_ai/`)
//! - `provider/` — native chat models, cloud/local routing, auth and errors
//!                 (was `src/neppy/providers/`)
//! - `voice/`    — transcription (STT) and TTS inference implementations
//!                 (moved from `src/neppy/voice/`)
//! - `http/`     — OpenAI-compatible `/v1/chat/completions` endpoint
//!
//! The RPC surface is `inference.*`; old `local_ai_*` RPC names are resolved
//! by the legacy alias layer for backwards compatibility.

/// `true` when the crate was compiled with the `inference` feature (the
/// default), i.e. the `cpal` audio-device stack is linked. Lets tests and
/// callers distinguish a slim/headless build from the desktop build without
/// naming gated symbols. When `false`, `cpal` is dropped from the dependency
/// graph (verify with `cargo tree -i cpal`) and the microphone-permission probe
/// reports `Unknown`.
pub const INFERENCE_COMPILED_IN: bool = cfg!(feature = "inference");

pub mod auth_error_registry;
pub mod device;
pub mod embeddings;
pub mod http;
pub mod local;
pub mod model_context;
pub mod model_ids;
pub mod openai_oauth;
pub mod ops;
pub mod parse;
pub mod paths;
pub mod presets;
pub mod provider;
mod schemas;
pub mod sentiment;
pub mod temperature;
pub mod tokenjuice;
pub mod turn_controls;
pub mod types;
pub mod vision_models;
pub mod voice;

pub use ops as rpc;
pub use schemas::{
    all_controller_schemas as all_inference_controller_schemas,
    all_registered_controllers as all_inference_registered_controllers, INFERENCE_AGENT_CHAT,
};

// Re-export the types that external callers (voice, agent, etc.) import from inference
pub use device::DeviceProfile;
pub use local::all_local_assistant_controller_schemas;
pub use local::all_local_assistant_registered_controllers;
pub use local::all_local_inference_controller_schemas;
pub use local::all_local_inference_registered_controllers;
pub use local::all_mlx_controller_schemas;
pub use local::all_mlx_registered_controllers;
pub use local::resume_interrupted_on_boot as resume_local_assistant_on_boot;
pub use model_context::context_window_for_model;
pub use presets::{ModelPreset, ModelTier, VisionMode};
pub use sentiment::SentimentResult;
pub use types::{
    LocalAiAssetStatus, LocalAiAssetsStatus, LocalAiDownloadProgressItem, LocalAiDownloadsProgress,
    LocalAiEmbeddingResult, LocalAiSpeechResult, LocalAiStatus, LocalAiTtsResult,
};

// Test helpers (re-exported for sibling test files that use inference_test_guard)
#[cfg(test)]
pub(crate) fn inference_test_guard() -> std::sync::MutexGuard<'static, ()> {
    local::inference_test_guard()
}

/// Test guard that makes the *shared* install root hermetic.
///
/// `paths::shared_root_dir` only honours `config.workspace_dir` when
/// `NEPPY_WORKSPACE` is set; otherwise it resolves to the developer's real
/// `~/.neppy`, so a test that writes Piper/Ollama/model stubs through
/// `workspace_*_dir(&config)` writes (and its cleanup deletes) real user files.
///
/// `lock_and_set` points the variable at `root` for the guard's lifetime and
/// holds both process-wide locks involved, always in this order:
/// `config::TEST_ENV_LOCK` (every other test that sets `NEPPY_WORKSPACE`)
/// then [`inference_test_guard`] (the install status/slot lock). The previous
/// value is restored on drop, including on unwind.
///
/// Not re-entrant: a test holding this guard must not also call
/// [`inference_test_guard`] itself.
#[cfg(test)]
pub(crate) struct HermeticSharedRoot {
    previous: Option<std::ffi::OsString>,
    // Fields drop in declaration order, after `Drop::drop` has restored the env.
    _inference: std::sync::MutexGuard<'static, ()>,
    _env: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl HermeticSharedRoot {
    pub(crate) fn lock_and_set(root: &std::path::Path) -> Self {
        let env = crate::neppy::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let inference = inference_test_guard();
        let previous = crate::neppy::util::env::var_os("NEPPY_WORKSPACE");
        std::env::set_var("NEPPY_WORKSPACE", root);
        Self {
            previous,
            _inference: inference,
            _env: env,
        }
    }
}

#[cfg(test)]
impl Drop for HermeticSharedRoot {
    fn drop(&mut self) {
        match self.previous.as_ref() {
            Some(previous) => std::env::set_var("NEPPY_WORKSPACE", previous),
            None => crate::neppy::util::env::remove_var("NEPPY_WORKSPACE"),
        }
    }
}
