//! Managed MLX runtime.
//!
//! Neppy supervises MLX servers as child processes: it resolves the binary,
//! builds the argument vector from `[[mlx.server]]` config, spawns, health
//! checks, and stops them.
//!
//! Split by concern, mirroring the neighbouring `ollama_admin` module:
//!
//! - `argv`   — pure `MlxServerConfig` → command line
//! - `binary` — locating `mlx_vlm.server` / `mlx_lm.server`
//! - `health`  — `/v1/models` probing and the per-server state machine
//! - `memory`  — admission control against a shared memory budget
//! - `pool`    — the supervisor: start/stop/status, resolved ports
//! - `process` — spawn, log capture, stop, orphan reclamation
//! - `service` — the `LocalAiService` methods bootstrap calls
//!
//! The unit of everything is one `[[mlx.server]]` block. The default config
//! declares a single `primary` block running `mlx_vlm.server`, which serves
//! chat, reasoning, vision, embeddings, rerank, STT and TTS from separate
//! model slots in one process.

pub(crate) mod argv;
pub(crate) mod binary;
pub(crate) mod health;
pub(crate) mod memory;
pub(crate) mod pool;
pub(crate) mod process;
mod service;

#[cfg(test)]
#[path = "../mlx_admin_tests.rs"]
mod tests;
