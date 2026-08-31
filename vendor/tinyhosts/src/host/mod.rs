//! The unified hosting interface.
//!
//! [`Host`] is the whole contract a provider adapter implements, and the whole
//! surface a caller needs. It is deliberately narrow: it covers shipping a
//! Next.js application and keeping it running — the site, its deployments, the
//! environment it reads, a database behind it, a domain in front of it, and the
//! traffic it served — and nothing else. Everything a provider offers beyond
//! that is reached through the provider's own client, not through here.
//!
//! Methods are grouped in the order a launch uses them, which is also the order
//! [`launch`](crate::launch()) calls them in.
//!
//! # Not every host does everything
//!
//! A capability a provider lacks returns [`Error::Unsupported`], naming the
//! provider and what it cannot do. That is the only honest answer: a stub that
//! returns `Ok` for a database it never created would be discovered at runtime
//! by an application whose `DATABASE_URL` is missing.
//!
//! [`Error::Unsupported`]: crate::Error::Unsupported

use async_trait::async_trait;

use crate::Result;
use crate::host::types::{
    AnalyticsQuery, AnalyticsSummary, Database, DatabaseSpec, DeployRequest, Deployment, Domain,
    EnvVar, EnvVarRecord, Site, SiteSpec,
};
use crate::providers::ProviderKind;

pub mod types;

/// One hosting provider account, reached through the unified model.
///
/// Implementations are cheap to clone and safe to share: each one is an HTTP
/// client plus a credential. Every method performs network I/O.
#[async_trait]
pub trait Host: Send + Sync + std::fmt::Debug {
    /// Which provider this is.
    fn kind(&self) -> ProviderKind;

    /// Creates a site.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptySiteName`](crate::Error::EmptySiteName) for a blank
    /// name, or a provider error — including one for a name already taken.
    async fn create_site(&self, spec: &SiteSpec) -> Result<Site>;

    /// Finds a site by name or identifier, returning `None` if there is none.
    ///
    /// # Errors
    ///
    /// Returns a provider error. A missing site is `Ok(None)`, not an error:
    /// "create it if it is not there" is the common path, and it should not be
    /// written by catching a failure.
    async fn find_site(&self, name: &str) -> Result<Option<Site>>;

    /// Lists sites, newest first, capped at `limit`.
    ///
    /// # Errors
    ///
    /// Returns a provider error.
    async fn list_sites(&self, limit: u32) -> Result<Vec<Site>>;

    /// Sets environment variables on a site, replacing any of the same name.
    ///
    /// Variables must be in place before the deployment that reads them: a
    /// Next.js build inlines what it can see at build time.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyEnvKey`](crate::Error::EmptyEnvKey) for a blank
    /// name, or a provider error.
    async fn set_env(&self, site: &str, vars: &[EnvVar]) -> Result<()>;

    /// Lists the environment variables on a site, without their values.
    ///
    /// # Errors
    ///
    /// Returns a provider error.
    async fn list_env(&self, site: &str) -> Result<Vec<EnvVarRecord>>;

    /// Provisions a managed database on the account.
    ///
    /// The database is not reachable from a site until
    /// [`attach_database`](Host::attach_database) connects the two.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoDatabaseProduct`](crate::Error::NoDatabaseProduct)
    /// when nothing on the account can serve the requested kind,
    /// [`Error::Unsupported`](crate::Error::Unsupported) when the provider has
    /// no managed databases at all, or a provider error.
    async fn provision_database(&self, spec: &DatabaseSpec) -> Result<Database>;

    /// Connects a database to a site and returns the variable names the site
    /// now receives.
    ///
    /// The values are the provider's to inject. This crate does not see a
    /// connection string, which is why the return is a list of names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`](crate::Error::Unsupported) when the
    /// provider cannot connect a database to a site, or a provider error.
    async fn attach_database(&self, database: &Database, site: &str) -> Result<Vec<String>>;

    /// Uploads a bundle and starts a deployment.
    ///
    /// The returned deployment has usually not finished building. Poll
    /// [`deployment`](Host::deployment) until
    /// [`DeploymentStatus::is_terminal`](types::DeploymentStatus::is_terminal).
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyBundle`](crate::Error::EmptyBundle) for an empty
    /// bundle, [`Error::EmptySiteName`](crate::Error::EmptySiteName) for a blank
    /// site, or a provider error.
    async fn deploy(&self, request: &DeployRequest) -> Result<Deployment>;

    /// Reads a deployment's current state.
    ///
    /// # Errors
    ///
    /// Returns a provider error, including
    /// [`Error::NotFound`](crate::Error::NotFound) for an unknown identifier.
    async fn deployment(&self, id: &str) -> Result<Deployment>;

    /// Lists a site's deployments, newest first, capped at `limit`.
    ///
    /// # Errors
    ///
    /// Returns a provider error.
    async fn list_deployments(&self, site: &str, limit: u32) -> Result<Vec<Deployment>>;

    /// Points the site's production traffic at an existing deployment.
    ///
    /// This is both the promote and the rollback: a rollback is a promote of an
    /// older deployment, and modelling it twice would suggest otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`](crate::Error::Unsupported) when the
    /// provider cannot repoint traffic without rebuilding, or a provider error.
    async fn promote(&self, site: &str, deployment: &str) -> Result<()>;

    /// Adds a custom domain to a site.
    ///
    /// A returned domain with `verified` false still needs its DNS records; the
    /// provider, not this crate, is the source of truth for what they are.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyDomain`](crate::Error::EmptyDomain) for a blank
    /// name, or a provider error.
    async fn add_domain(&self, site: &str, domain: &str) -> Result<Domain>;

    /// Lists a site's domains.
    ///
    /// # Errors
    ///
    /// Returns a provider error.
    async fn list_domains(&self, site: &str) -> Result<Vec<Domain>>;

    /// Reports the traffic a site served over a window.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidAnalyticsWindow`](crate::Error::InvalidAnalyticsWindow)
    /// for a window that does not move forward,
    /// [`Error::Unsupported`](crate::Error::Unsupported) when the provider has
    /// no analytics, or a provider error.
    async fn analytics(&self, query: &AnalyticsQuery) -> Result<AnalyticsSummary>;
}

#[cfg(test)]
mod test;
