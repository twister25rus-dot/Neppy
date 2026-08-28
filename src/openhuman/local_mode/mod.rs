//! Local Mode — running OpenHuman with no dependency on the project's hosted
//! backend.
//!
//! # Why this module exists
//!
//! Most of OpenHuman is already local: the memory tree, the Obsidian mirror,
//! threads, skills, cron and the whole agent harness live in SQLite and files
//! under the workspace, and inference can already be pointed at Ollama /
//! LM Studio / vLLM. What was *not* local is the **control plane**:
//! sign-in, the session token every client carries, team usage, announcements
//! and the managed proxies for inference, embeddings and search. Those all
//! resolve through [`effective_backend_api_url`](crate::api::config::effective_backend_api_url)
//! to `api.tinyhumans.ai`.
//!
//! `PrivacyMode::LocalOnly` does not close that gap, and deliberately so — it
//! exempts the control plane (see
//! [`is_control_plane`](crate::openhuman::security::egress)) because blocking
//! sign-in buys no privacy. Local mode is the orthogonal switch: it *replaces*
//! the control plane rather than restricting what flows over it.
//!
//! # Shape
//!
//! | Piece | Role |
//! |---|---|
//! | [`is_local_mode`] | the single resolution point (config → env), used everywhere |
//! | [`backend`] | a loopback axum service implementing the hosted contract locally |
//! | [`services`] | the authoritative hosted-service → local-replacement inventory |
//! | [`defaults`] | per-subsystem local default resolution (embeddings, search, tracing) |
//!
//! Nothing here changes behaviour until local mode is on: [`is_local_mode`]
//! returns `false` for a default config with no env override, and every
//! consumer keys off it.
//!
//! # What cannot be reproduced locally
//!
//! Some hosted features are thin wrappers over a third-party SaaS whose
//! service we may not reimplement or proxy around (Composio's action
//! catalogue, Exa's index, Stripe billing, the Seedream/Seedance and Veo
//! media models, ElevenLabs realtime voice). Local mode does not pretend
//! those work: [`services`] records each one with the local alternative the
//! app *does* implement (MCP servers, SearXNG, no billing, a local
//! image-generation endpoint, in-process Whisper + Piper), and the local
//! backend answers their routes with a machine-readable
//! `local_mode_unsupported` error carrying that alternative — never a
//! fabricated success.

pub mod bootstrap;
pub mod defaults;
pub mod resolve;
pub mod services;

#[cfg(feature = "http-server")]
pub mod backend;

pub use bootstrap::bootstrap;
pub use resolve::{
    is_local_mode, is_local_mode_from_env, local_backend_base_url, local_mode_active,
    publish_local_mode, set_local_backend_base_url,
};
pub use services::{
    service_inventory, LocalReplacementKind, LocalServiceEntry, LocalServiceInventory,
};
