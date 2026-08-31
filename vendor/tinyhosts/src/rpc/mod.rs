//! One JSON request in, one JSON result out.
//!
//! Everything above this crate is in another process: `OpenCompany`'s front end
//! takes an API key from a form, `OpenHuman`'s agent decides to ship a site, and
//! neither one links against this library. [`execute_json`] is the boundary they
//! meet at, and the `TinyBus` adapter is a thin wrapper over it.
//!
//! # The credential travels with the request
//!
//! A hosting account belongs to a user, not to the process, so [`Request`]
//! carries the credential rather than reading it from a global. A request that
//! omits it falls back to the environment, which is what a self-hosted single
//! tenant wants. The credential is read out of the envelope and never written
//! back into a result — see [`Credentials`].

use serde::{Deserialize, Serialize};

use crate::host::types::{
    AnalyticsQuery, AnalyticsSummary, Database, DatabaseSpec, DeployRequest, Deployment, Domain,
    EnvVar, EnvVarRecord, Site, SiteSpec,
};
use crate::launch::types::{Launch, LaunchPlan};
use crate::providers::{ProviderKind, connect_to};
use crate::{Credentials, Error, Result};

/// A request to act on one hosting account.
#[derive(Debug, Deserialize)]
pub struct Request {
    /// Which provider to act on. Defaults to [`ProviderKind::Vercel`].
    #[serde(default)]
    pub provider: ProviderKind,
    /// The account's credential. Omitted, it is read from the environment.
    #[serde(default)]
    pub credentials: Option<Credentials>,
    /// An alternate API root for the provider.
    ///
    /// Set it when the provider is reached through an egress proxy. Omitted, the
    /// provider's own root is used.
    #[serde(default)]
    pub base_url: Option<String>,
    /// What to do.
    #[serde(flatten)]
    pub operation: Operation,
}

/// One thing a request can ask for.
///
/// The variants are exactly the [`Host`](crate::Host) surface plus
/// [`launch`](crate::launch()), so the bus exposes no more authority than the
/// library does.
#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Operation {
    /// Run a whole launch: site, database, environment, domains, deployment.
    Launch {
        /// The plan to run.
        ///
        /// Boxed because it carries a whole bundle: unboxed, every other
        /// variant of this enum would be as large as an application.
        plan: Box<LaunchPlan>,
    },
    /// Create a site.
    CreateSite {
        /// What the site should be.
        spec: SiteSpec,
    },
    /// Find a site by name, or report that there is none.
    FindSite {
        /// The site's name or identifier.
        site: String,
    },
    /// List sites, newest first.
    ListSites {
        /// How many to return.
        #[serde(default = "default_limit")]
        limit: u32,
    },
    /// Set environment variables on a site.
    SetEnv {
        /// The site's name or identifier.
        site: String,
        /// The variables to set.
        vars: Vec<EnvVar>,
    },
    /// List a site's environment variables, without their values.
    ListEnv {
        /// The site's name or identifier.
        site: String,
    },
    /// Provision a managed database.
    ProvisionDatabase {
        /// What the database should be.
        spec: DatabaseSpec,
    },
    /// Connect a database to a site.
    AttachDatabase {
        /// The database, as [`Operation::ProvisionDatabase`] returned it.
        database: Database,
        /// The site's name or identifier.
        site: String,
    },
    /// Upload a bundle and start a deployment.
    Deploy {
        /// The deployment to start. Boxed for the same reason as
        /// [`Operation::Launch`]'s plan.
        request: Box<DeployRequest>,
    },
    /// Read a deployment's current state.
    Deployment {
        /// The deployment's identifier.
        id: String,
    },
    /// List a site's deployments, newest first.
    ListDeployments {
        /// The site's name or identifier.
        site: String,
        /// How many to return.
        #[serde(default = "default_limit")]
        limit: u32,
    },
    /// Point production traffic at an existing deployment.
    Promote {
        /// The site's name or identifier.
        site: String,
        /// The deployment's identifier.
        deployment: String,
    },
    /// Add a custom domain to a site.
    AddDomain {
        /// The site's name or identifier.
        site: String,
        /// The domain to add.
        domain: String,
    },
    /// List a site's domains.
    ListDomains {
        /// The site's name or identifier.
        site: String,
    },
    /// Report the traffic a site served.
    Analytics {
        /// The window to report on.
        query: AnalyticsQuery,
    },
}

const fn default_limit() -> u32 {
    20
}

/// What an operation produced.
///
/// The envelope is adjacently tagged — `{"result": "...", "value": ...}` — so a
/// list result and a record result have the same shape on the wire, and a reader
/// can dispatch on one field.
#[derive(Debug, Serialize)]
#[serde(tag = "result", content = "value", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Outcome {
    /// A completed launch.
    ///
    /// Boxed because it carries a whole site, database and deployment: unboxed,
    /// every other variant would be as large as the largest one.
    Launch(Box<Launch>),
    /// One site.
    Site(Site),
    /// A site that does not exist.
    NoSite,
    /// Several sites.
    Sites(Vec<Site>),
    /// One deployment.
    Deployment(Deployment),
    /// Several deployments.
    Deployments(Vec<Deployment>),
    /// A site's environment variables, without their values.
    Env(Vec<EnvVarRecord>),
    /// One database.
    Database(Database),
    /// The environment variable names a database injected.
    EnvKeys(Vec<String>),
    /// One domain.
    Domain(Domain),
    /// Several domains.
    Domains(Vec<Domain>),
    /// A traffic report.
    Analytics(AnalyticsSummary),
    /// An operation that produced nothing but succeeded.
    Done,
}

/// Runs one request.
///
/// # Errors
///
/// Returns [`Error::MissingApiKey`] when the request carries no credential and
/// the environment holds none, [`Error::UnknownProvider`] when this build has no
/// adapter for the named provider, or whatever the operation itself returns.
pub async fn execute(request: Request) -> Result<Outcome> {
    let credentials = match request.credentials {
        Some(credentials) => credentials,
        None => request.provider.credentials_from_env()?,
    };
    let host = connect_to(request.provider, credentials, request.base_url.as_deref())?;

    match request.operation {
        Operation::Launch { plan } => crate::launch::launch(host.as_ref(), &plan)
            .await
            .map(|launched| Outcome::Launch(Box::new(launched))),
        Operation::CreateSite { spec } => host.create_site(&spec).await.map(Outcome::Site),
        Operation::FindSite { site } => Ok(match host.find_site(&site).await? {
            Some(site) => Outcome::Site(site),
            None => Outcome::NoSite,
        }),
        Operation::ListSites { limit } => host.list_sites(limit).await.map(Outcome::Sites),
        Operation::SetEnv { site, vars } => {
            host.set_env(&site, &vars).await.map(|()| Outcome::Done)
        }
        Operation::ListEnv { site } => host.list_env(&site).await.map(Outcome::Env),
        Operation::ProvisionDatabase { spec } => {
            host.provision_database(&spec).await.map(Outcome::Database)
        }
        Operation::AttachDatabase { database, site } => host
            .attach_database(&database, &site)
            .await
            .map(Outcome::EnvKeys),
        Operation::Deploy { request } => host.deploy(&request).await.map(Outcome::Deployment),
        Operation::Deployment { id } => host.deployment(&id).await.map(Outcome::Deployment),
        Operation::ListDeployments { site, limit } => host
            .list_deployments(&site, limit)
            .await
            .map(Outcome::Deployments),
        Operation::Promote { site, deployment } => host
            .promote(&site, &deployment)
            .await
            .map(|()| Outcome::Done),
        Operation::AddDomain { site, domain } => {
            host.add_domain(&site, &domain).await.map(Outcome::Domain)
        }
        Operation::ListDomains { site } => host.list_domains(&site).await.map(Outcome::Domains),
        Operation::Analytics { query } => host.analytics(&query).await.map(Outcome::Analytics),
    }
}

/// Runs one request given as JSON, returning its result as JSON.
///
/// # Errors
///
/// Returns [`Error::Envelope`] when the request is not a [`Request`] or the
/// result cannot be serialized, and otherwise whatever [`execute`] returns.
pub async fn execute_json(request: &str) -> Result<String> {
    let request: Request = serde_json::from_str(request).map_err(|error| Error::Envelope {
        reason: error.to_string(),
    })?;

    let outcome = execute(request).await?;
    serde_json::to_string(&outcome).map_err(|error| Error::Envelope {
        reason: error.to_string(),
    })
}

/// The providers this build can connect to, as their slugs.
///
/// A caller uses this to populate a provider picker without hard-coding what a
/// given build was compiled with.
#[must_use]
pub fn providers() -> Vec<&'static str> {
    let mut available = Vec::new();
    if cfg!(feature = "vercel") {
        available.push(ProviderKind::Vercel.as_str());
    }
    available
}

#[cfg(test)]
mod test;
