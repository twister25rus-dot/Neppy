//! One API for putting a Next.js application, and the database behind it, on a
//! real hosting provider.
//!
//! `TinyHosts` is the hosting category of the `TinyHumans` stack: `OpenHuman`
//! vendors it, `OpenCompany` inherits it from there, and a user who pastes a
//! provider API key into `OpenCompany` gets a live site out of a workspace. The unit of work is
//! deliberately the whole thing — a site, a managed database wired into it, the
//! environment it reads, a domain, and the traffic it served — because that is
//! what "host this" means, and a library that only uploads files leaves every
//! caller to reinvent the other four steps.
//!
//! # The shape
//!
//! - [`Host`] is the provider-agnostic interface. [`Vercel`] is the first
//!   implementation of it.
//! - [`ProviderKind`] and [`connect`] make the provider a configuration value
//!   rather than a compile-time choice.
//! - [`Bundle`] is the application's files as the provider will receive them.
//! - [`launch()`] runs the whole flow in the one order that works.
//! - [`rpc`] is the same surface as JSON, for the callers in another process,
//!   and the `TinyBus` module is a thin wrapper over it.
//!
//! # Example
//!
//! ```no_run
//! use tinyhosts::{Bundle, Credentials, DatabaseSpec, LaunchPlan, ProviderKind, SiteSpec, launch};
//!
//! # async fn ship() -> tinyhosts::Result<()> {
//! let host = tinyhosts::connect(ProviderKind::Vercel, Credentials::new("vercel-token")?)?;
//!
//! let plan = LaunchPlan::new(SiteSpec::new("shop"), Bundle::from_dir("./shop")?)
//!     .with_database(DatabaseSpec::new("shop-db"))
//!     .into_production();
//!
//! let result = launch(host.as_ref(), &plan).await?;
//! println!("building at {:?}", result.url());
//! # Ok(())
//! # }
//! ```
//!
//! The launch returns while the build is still running: poll
//! [`Host::deployment`] until its
//! [`status`](host::types::DeploymentStatus::is_terminal) settles.
//!
//! # What this crate does not do
//!
//! It does not hold a connection string, decrypt an environment variable, or
//! return a secret it was given. It does not wait, retry, or schedule — a
//! caller's patience is the caller's policy. And it does not pretend: a
//! capability a provider lacks is [`Error::Unsupported`], never a silent success.

pub mod bundle;
pub mod credentials;
pub mod error;
pub mod host;
pub mod launch;
pub mod providers;
pub mod rpc;

#[cfg(feature = "module")]
mod tinybus_module;

pub use bundle::{Bundle, EXCLUDED, SiteFile};
pub use credentials::Credentials;
pub use error::{Error, Result};
pub use host::Host;
pub use host::types::{
    AnalyticsBucket, AnalyticsDimension, AnalyticsQuery, AnalyticsSummary, Database, DatabaseKind,
    DatabaseSpec, DeployRequest, Deployment, DeploymentStatus, DeploymentTarget, Domain, EnvVar,
    EnvVarRecord, Framework, Site, SiteSpec,
};
pub use launch::launch;
pub use launch::types::{Launch, LaunchPlan};
pub use providers::{ProviderKind, connect, connect_from_env, connect_to};

#[cfg(feature = "vercel")]
pub use providers::vercel::Vercel;

// `rpc`'s own names — `Request`, `Operation`, `Outcome` — are only unambiguous
// next to each other, so they stay in their module rather than being flattened
// into the crate root.
pub use rpc::{execute, execute_json};
