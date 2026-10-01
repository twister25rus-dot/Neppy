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
//! - `models`  — what the local model cache holds, and reclaiming it
//! - `pool`    — the supervisor: start/stop/status, resolved ports
//! - `process` — spawn, log capture, stop, orphan reclamation
//! - `reaper` — graceful stop of every spawned worker on shutdown, and reaping
//!   of workers a dead core left behind
//! - `service` — the `LocalAiService` methods bootstrap calls
//! - `pressure` — sudo-free memory readings and the pressure state machine
//! - `gate`    — the single-flight inference gate every `mlx:` call takes
//! - `worker`  — lazy start, crash restart budget, model switch, unload
//! - `watchdog` — the idle/pressure policy (`decide`) and its loop
//! - `metrics` — bounded sample/event rings and daily JSONL files
//! - `worker_rpc` — payloads for `mlx.worker_status` / `mlx.worker_metrics`
//!
//! The unit of everything is one `[[mlx.server]]` block. The default config
//! declares a single `primary` block running `mlx_vlm.server`, which serves
//! chat, reasoning, vision, embeddings, rerank, STT and TTS from separate
//! model slots in one process.

pub(crate) mod argv;
pub(crate) mod binary;
pub(crate) mod gate;
pub(crate) mod health;
pub(crate) mod memory;
pub(crate) mod metrics;
pub(crate) mod models;
pub(crate) mod pool;
pub(crate) mod pressure;
pub(crate) mod process;
pub(crate) mod reaper;
mod service;
pub(crate) mod watchdog;
pub(crate) mod worker;
pub(crate) mod worker_rpc;

#[cfg(test)]
#[path = "../mlx_admin_tests.rs"]
mod tests;
