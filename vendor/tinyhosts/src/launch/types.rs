//! What a launch asks for, and what it produced.

use serde::{Deserialize, Serialize};

use crate::Result;
use crate::bundle::Bundle;
use crate::host::types::{
    Database, DatabaseSpec, Deployment, DeploymentTarget, Domain, EnvVar, Site, SiteSpec,
};

/// Everything needed to put one application on the internet.
///
/// # Examples
///
/// ```
/// # use tinyhosts::{Bundle, DatabaseSpec, EnvVar, LaunchPlan, SiteSpec};
/// let mut bundle = Bundle::new();
/// bundle.insert("package.json", br#"{"name":"shop"}"#)?;
/// bundle.insert("app/page.tsx", b"export default () => <h1>Shop</h1>;")?;
///
/// let plan = LaunchPlan::new(SiteSpec::new("shop"), bundle)
///     .with_database(DatabaseSpec::new("shop-db"))
///     .with_env(vec![EnvVar::new("NEXT_PUBLIC_NAME", "Shop")])
///     .into_production();
///
/// assert!(plan.validate().is_ok());
/// # Ok::<(), tinyhosts::Error>(())
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchPlan {
    /// The site to deploy to, created if it is not already there.
    pub site: SiteSpec,
    /// The application's files.
    pub bundle: Bundle,
    /// A database to provision and connect, when the application needs one.
    #[serde(default)]
    pub database: Option<DatabaseSpec>,
    /// Environment variables to set before the build.
    #[serde(default)]
    pub env: Vec<EnvVar>,
    /// Custom domains to attach.
    #[serde(default)]
    pub domains: Vec<String>,
    /// Which environment the deployment serves.
    #[serde(default)]
    pub target: DeploymentTarget,
}

impl LaunchPlan {
    /// A preview launch of `bundle` as `site`, with no database and no domains.
    #[must_use]
    pub fn new(site: SiteSpec, bundle: Bundle) -> Self {
        Self {
            site,
            bundle,
            database: None,
            env: Vec::new(),
            domains: Vec::new(),
            target: DeploymentTarget::Preview,
        }
    }

    /// Provisions and connects a database as part of the launch.
    #[must_use]
    pub fn with_database(mut self, database: DatabaseSpec) -> Self {
        self.database = Some(database);
        self
    }

    /// Sets environment variables before the build.
    #[must_use]
    pub fn with_env(mut self, env: Vec<EnvVar>) -> Self {
        self.env = env;
        self
    }

    /// Attaches custom domains.
    #[must_use]
    pub fn with_domains(mut self, domains: Vec<String>) -> Self {
        self.domains = domains;
        self
    }

    /// Sends the deployment to production rather than to a preview URL.
    #[must_use]
    pub fn into_production(mut self) -> Self {
        self.target = DeploymentTarget::Production;
        self
    }

    /// Checks the plan before any of it runs.
    ///
    /// # Errors
    ///
    /// Returns the first failure from the site spec, the bundle, the database
    /// spec, an environment variable, or a domain: [`Error::EmptySiteName`],
    /// [`Error::EmptyBundle`], [`Error::EmptyEnvKey`], or [`Error::EmptyDomain`].
    ///
    /// [`Error::EmptySiteName`]: crate::Error::EmptySiteName
    /// [`Error::EmptyBundle`]: crate::Error::EmptyBundle
    /// [`Error::EmptyEnvKey`]: crate::Error::EmptyEnvKey
    /// [`Error::EmptyDomain`]: crate::Error::EmptyDomain
    pub fn validate(&self) -> Result<()> {
        self.site.validate()?;
        if self.bundle.is_empty() {
            return Err(crate::Error::EmptyBundle);
        }
        if let Some(database) = &self.database {
            database.validate()?;
        }
        for var in &self.env {
            var.validate()?;
        }
        for domain in &self.domains {
            if domain.trim().is_empty() {
                return Err(crate::Error::EmptyDomain);
            }
        }
        Ok(())
    }
}

/// What a launch produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Launch {
    /// The site the application lives on.
    pub site: Site,
    /// Whether this launch created the site, rather than finding it.
    pub created_site: bool,
    /// The database that was provisioned, when the plan asked for one.
    #[serde(default)]
    pub database: Option<Database>,
    /// The environment variable names the database injected into the site.
    #[serde(default)]
    pub database_env_keys: Vec<String>,
    /// The domains that were attached.
    #[serde(default)]
    pub domains: Vec<Domain>,
    /// The deployment, which is usually still building.
    pub deployment: Deployment,
}

impl Launch {
    /// The URL the application will serve from, once the deployment is ready.
    #[must_use]
    pub fn url(&self) -> Option<&str> {
        self.deployment.url.as_deref()
    }
}
