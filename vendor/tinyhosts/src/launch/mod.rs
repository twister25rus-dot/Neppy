//! The standard way to put a Next.js application on the internet.
//!
//! [`launch`] is the whole flow in one call: make sure the site exists, give it
//! a database, set its environment, attach its domains, and deploy it. Callers
//! reach for the individual [`Host`] methods when they want one step; they reach
//! for this when they want a live URL.
//!
//! # The order is the point
//!
//! The steps do not commute, and getting them wrong produces a site that builds
//! successfully and does not work:
//!
//! 1. **The site**, created only if [`Host::find_site`] does not find it, so a
//!    relaunch redeploys rather than failing on a name that is taken.
//! 2. **The database**, provisioned and *then connected to the site*. Connecting
//!    is what puts `DATABASE_URL` into the site's environment.
//! 3. **The caller's environment variables**, after the database, so an explicit
//!    variable overrides one the database injected rather than the reverse.
//! 4. **The domains**, before the deployment, so a production deployment is
//!    aliased to them as it goes live instead of a request later.
//! 5. **The deployment**, last, because a Next.js build reads the environment at
//!    build time. A database attached after the build is a database the built
//!    pages cannot see.
//!
//! # Waiting
//!
//! [`launch`] returns as soon as the provider accepts the deployment, which is
//! before it has finished building. The returned [`Deployment`] carries the URL
//! the site will serve from and a non-terminal
//! [`status`](crate::host::types::DeploymentStatus); poll
//! [`Host::deployment`] until
//! [`is_terminal`](crate::host::types::DeploymentStatus::is_terminal). This crate
//! deliberately owns no timer and no retry loop: how long a caller is willing to
//! wait for a build is the caller's policy, and a hidden one is impossible to
//! cancel or report on.

use crate::Result;
use crate::host::Host;
use crate::host::types::{DeployRequest, Deployment};
use crate::launch::types::{Launch, LaunchPlan};

pub mod types;

/// Runs a launch plan against a host and returns everything it produced.
///
/// # Errors
///
/// Returns the first failure from any step. A launch is not transactional:
/// earlier steps stay done, which is deliberate — a database that was
/// provisioned before a build failed should still be there when the build is
/// retried, not deleted and paid for twice.
pub async fn launch(host: &dyn Host, plan: &LaunchPlan) -> Result<Launch> {
    plan.validate()?;

    let (site, created_site) = match host.find_site(&plan.site.name).await? {
        Some(site) => (site, false),
        None => (host.create_site(&plan.site).await?, true),
    };

    let mut database = None;
    let mut database_env_keys = Vec::new();
    if let Some(spec) = &plan.database {
        let provisioned = host.provision_database(spec).await?;
        database_env_keys = host.attach_database(&provisioned, &site.name).await?;
        database = Some(provisioned);
    }

    if !plan.env.is_empty() {
        host.set_env(&site.name, &plan.env).await?;
    }

    let mut domains = Vec::with_capacity(plan.domains.len());
    for domain in &plan.domains {
        domains.push(host.add_domain(&site.name, domain).await?);
    }

    let deployment: Deployment = host
        .deploy(
            &DeployRequest::new(&site.name, plan.bundle.clone())
                .with_framework(plan.site.framework.clone())
                .with_target(plan.target),
        )
        .await?;

    Ok(Launch {
        site,
        created_site,
        database,
        database_env_keys,
        domains,
        deployment,
    })
}

#[cfg(test)]
mod test;
