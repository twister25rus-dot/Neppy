//! The local backend — a loopback service implementing the hosted control
//! plane so the rest of the app does not have to know the hosted one is gone.
//!
//! # Why a server and not a hundred call-site edits
//!
//! Every hosted round-trip in the core and the renderer already funnels through
//! one place: [`effective_backend_api_url`](crate::api::config::effective_backend_api_url).
//! Standing up a local service that speaks the same contract and repointing
//! that one function keeps `api::rest`, `IntegrationClient`, the renderer's
//! `fetch` calls, and every existing test honest — the UI and its features are
//! preserved *by construction* rather than by auditing several hundred call
//! sites for a conditional.
//!
//! # What it serves
//!
//! | Family | Behaviour |
//! |---|---|
//! | `/auth/*` | a device-local session; see [`routes::auth`] |
//! | `/teams/*` | a single-member local team over the on-device cost ledger |
//! | `/openai/v1/*` | forwarded to the configured local runtime |
//! | `/announcements`, `/version` | local, static answers |
//! | `/telemetry/*` | accepted and dropped (tracing is off-device by definition) |
//! | everything else | a structured [`LocalModeUnsupported`](routes::unsupported) error |
//!
//! That last row is the important one. A hosted route with no local
//! implementation answers `501` with the machine-readable id of the
//! [`services`](crate::openhuman::local_mode::services) entry that owns it and
//! the local alternative — never a fabricated `200`. An agent that receives
//! invented search results acts on them.
//!
//! # Security posture
//!
//! The listener binds loopback only and requires a bearer token on every route
//! but `/health` and `/version`. That is not theatre: any process on the
//! machine can reach a loopback port, and the browser can be induced to POST
//! to one cross-origin. See [`auth`] for what is accepted and why.

mod auth;
mod error;
mod identity;
mod routes;
mod server;
mod state;

pub use error::{LocalModeError, LocalModeErrorBody};
pub use identity::{local_session_token, LocalIdentity};
pub use server::{start, LocalBackendHandle};
pub use state::LocalBackendState;

/// Log prefix shared by every module in the local backend.
pub(crate) const LOG_PREFIX: &str = "[local-backend]";

#[cfg(test)]
mod tests;
