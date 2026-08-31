//! The Vercel adapter.
//!
//! [`Vercel`] implements [`Host`] against Vercel's REST API: sites are
//! projects, deployments are file uploads followed by a build, databases are
//! marketplace stores connected to a project, and analytics come from the web
//! analytics query API.
//!
//! # Deploying without Git
//!
//! Vercel's usual path is a Git integration. This adapter uses the other one: it
//! uploads each file to `POST /v2/files` keyed by its SHA-1 digest, then creates
//! a deployment referencing those digests. That is what makes a workspace with
//! no repository behind it deployable, and it is why the digest algorithm is
//! SHA-1 — Vercel's `x-vercel-digest` header defines it.
//!
//! # Databases
//!
//! Vercel does not run databases; its marketplace partners do. Provisioning one
//! therefore means finding an installed integration whose product serves the
//! requested [`DatabaseKind`](crate::DatabaseKind), creating a store from it, and connecting that
//! store to the project — at which point Vercel injects the connection
//! variables into the project's environment. This crate never sees them, which
//! is why [`Host::attach_database`] returns names rather than values.

use std::fmt;

use async_trait::async_trait;
use reqwest::Method;
use serde_json::Value;

use crate::host::Host;
use crate::host::types::{
    AnalyticsBucket, AnalyticsQuery, AnalyticsSummary, Database, DatabaseSpec, DeployRequest,
    Deployment, DeploymentTarget, Domain, EnvVar, EnvVarRecord, Framework, Site, SiteSpec,
};
use crate::providers::ProviderKind;
use crate::{Credentials, Error, Result};

use self::http::{DEFAULT_BASE_URL, Http};
use self::wire::{
    AnalyticsEnvelope, Configuration, ConnectResource, CreateDeployment, CreateDomain,
    CreateEnvVar, CreateProject, CreateStore, DeploymentBody, Deployments, DomainBody, Domains,
    Envs, Products, Project, ProjectSettings, Projects, StoreEnvelope, UploadedFile,
};

mod http;
mod wire;

/// A Vercel account, reached through the unified hosting model.
#[derive(Debug)]
pub struct Vercel {
    http: Http,
}

impl Vercel {
    /// Connects to Vercel's public API with `credentials`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`] when the HTTP client cannot be built.
    pub fn new(credentials: Credentials) -> Result<Self> {
        Self::with_base_url(credentials, DEFAULT_BASE_URL)
    }

    /// Connects to a different API root.
    ///
    /// This exists for the test suite, which runs the adapter against a local
    /// mock of the REST API, and for a deployment that reaches Vercel through an
    /// egress proxy.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InsecureBaseUrl`] when `base_url` is `http://` against a
    /// non-loopback host — this constructor sends the bearer credential to
    /// every request `base_url` produces, the same as
    /// [`connect_to`](crate::providers::connect_to), and applies the same
    /// check. Returns [`Error::Transport`] when the HTTP client cannot be
    /// built.
    pub fn with_base_url(credentials: Credentials, base_url: impl Into<String>) -> Result<Self> {
        let base_url = base_url.into();
        if !crate::providers::is_secure_base_url(&base_url) {
            return Err(Error::InsecureBaseUrl { base_url });
        }

        Ok(Self {
            http: Http::new(credentials, base_url)?,
        })
    }

    /// Resolves a site name to the project identifier some endpoints require.
    ///
    /// Most project endpoints accept a name or an identifier; the promote and
    /// connect endpoints take only the identifier.
    async fn project_id(&self, site: &str) -> Result<String> {
        self.find_site(site)
            .await?
            .map(|site| site.id)
            .ok_or_else(|| Error::NotFound {
                provider: ProviderKind::Vercel.as_str().to_owned(),
                resource: format!("project {site}"),
            })
    }

    /// Uploads one deployment file, keyed by its SHA-1 digest.
    async fn upload(&self, digest: &str, contents: &[u8]) -> Result<()> {
        let builder = self
            .http
            .request(Method::POST, "/v2/files", &[])
            .header("x-vercel-digest", digest)
            .header("content-type", "application/octet-stream")
            .body(contents.to_vec());

        self.http.discard(builder, "deployment file").await
    }

    /// Finds an installed integration and product that can serve `spec`.
    ///
    /// Returns the installation identifier and the product identifier, in that
    /// order.
    async fn find_product(&self, spec: &DatabaseSpec) -> Result<(String, String)> {
        let configurations: Vec<Configuration> = self
            .http
            .get_json(
                "/v1/integrations/configurations",
                &[("view", "account".to_owned())],
                "integrations",
            )
            .await?;

        for configuration in configurations {
            let products: Products = self
                .http
                .get_json(
                    &format!(
                        "/v1/integrations/configuration/{}/products",
                        configuration.id
                    ),
                    &[],
                    "integration products",
                )
                .await?;

            let matched = products.products.into_iter().find(|product| {
                match spec.product.as_deref() {
                    // A pinned product is matched exactly, by slug or by id, and
                    // never by the kind's hints: pinning exists to overrule them.
                    Some(pinned) => {
                        product.slug.eq_ignore_ascii_case(pinned) || product.id == pinned
                    }
                    None => product.serves(&spec.kind, configuration.slug.as_deref()),
                }
            });

            if let Some(product) = matched {
                return Ok((configuration.id, product.id));
            }
        }

        Err(Error::NoDatabaseProduct {
            provider: ProviderKind::Vercel.as_str().to_owned(),
            kind: spec
                .product
                .clone()
                .unwrap_or_else(|| spec.kind.as_str().to_owned()),
        })
    }

    /// Reads the breakdown rows for an analytics query.
    async fn breakdown(&self, query: &AnalyticsQuery) -> Result<Vec<AnalyticsBucket>> {
        let Some(dimension) = query.breakdown else {
            return Ok(Vec::new());
        };

        let envelope: AnalyticsEnvelope = self
            .http
            .get_json(
                "/v1/query/web-analytics/visits/aggregate",
                &[
                    ("projectId", query.site.clone()),
                    ("by", dimension.as_str().to_owned()),
                    ("since", query.since_ms.to_string()),
                    ("until", query.until_ms.to_string()),
                    ("limit", query.limit.to_string()),
                ],
                "analytics breakdown",
            )
            .await?;

        Ok(buckets(dimension.as_str(), &envelope.data))
    }
}

#[async_trait]
impl Host for Vercel {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Vercel
    }

    async fn create_site(&self, spec: &SiteSpec) -> Result<Site> {
        spec.validate()?;

        let body = CreateProject {
            name: spec.name.trim(),
            framework: framework_param(&spec.framework),
        };
        let project: Project = self
            .http
            .post_json("/v11/projects", &[], &body, "project")
            .await?;

        Ok(project.into_site())
    }

    async fn find_site(&self, name: &str) -> Result<Option<Site>> {
        let builder = self.http.request(
            Method::GET,
            &format!("/v9/projects/{}", encode_segment(name)),
            &[],
        );
        let project: Option<Project> = self.http.optional_json(builder, "project").await?;

        Ok(project.map(Project::into_site))
    }

    async fn list_sites(&self, limit: u32) -> Result<Vec<Site>> {
        let projects: Projects = self
            .http
            .get_json(
                "/v10/projects",
                &[("limit", limit.to_string())],
                "project list",
            )
            .await?;

        Ok(projects
            .projects
            .into_iter()
            .map(Project::into_site)
            .collect())
    }

    async fn set_env(&self, site: &str, vars: &[EnvVar]) -> Result<()> {
        let mut body = Vec::with_capacity(vars.len());
        for var in vars {
            var.validate()?;
            body.push(CreateEnvVar {
                key: var.key.trim().to_owned(),
                value: var.value.clone(),
                r#type: if var.secret { "sensitive" } else { "encrypted" },
                target: env_targets(&var.targets),
            });
        }

        if body.is_empty() {
            return Ok(());
        }

        self.http
            .post_discard(
                &format!("/v10/projects/{}/env", encode_segment(site)),
                &[("upsert", "true".to_owned())],
                &body,
                "environment variables",
            )
            .await
    }

    async fn list_env(&self, site: &str) -> Result<Vec<EnvVarRecord>> {
        let envs: Envs = self
            .http
            .get_json(
                &format!("/v10/projects/{}/env", encode_segment(site)),
                &[],
                "environment variables",
            )
            .await?;

        Ok(envs
            .envs
            .into_iter()
            .map(wire::EnvBody::into_record)
            .collect())
    }

    async fn provision_database(&self, spec: &DatabaseSpec) -> Result<Database> {
        spec.validate()?;

        let (installation, product) = self.find_product(spec).await?;
        let body = CreateStore {
            name: spec.name.trim(),
            integration_configuration_id: &installation,
            integration_product_id_or_slug: &product,
        };

        let envelope: StoreEnvelope = self
            .http
            .post_json(
                "/v1/storage/stores/integration/direct",
                &[],
                &body,
                "database",
            )
            .await?;

        let store = envelope.store.ok_or_else(|| Error::Decode {
            provider: ProviderKind::Vercel.as_str().to_owned(),
            resource: "database".to_owned(),
            reason: "the response carried no store".to_owned(),
        })?;

        let mut database = store.into_database(spec.name.trim(), spec.kind.clone());
        if database.installation_id.is_none() {
            database.installation_id = Some(installation);
        }

        if database.status == "error" {
            return Err(Error::DatabaseNotReady {
                name: database.name,
                status: database.status,
            });
        }

        Ok(database)
    }

    async fn attach_database(&self, database: &Database, site: &str) -> Result<Vec<String>> {
        let installation = database
            .installation_id
            .as_deref()
            .ok_or_else(|| Error::NotFound {
                provider: ProviderKind::Vercel.as_str().to_owned(),
                resource: format!("marketplace installation for database {}", database.name),
            })?;

        let project = self.project_id(site).await?;
        let body = ConnectResource {
            project_id: &project,
            env_var_environments: vec!["production", "preview", "development"],
        };

        self.http
            .post_discard(
                &format!(
                    "/v1/integrations/installations/{installation}/resources/{}/connections",
                    database.id
                ),
                &[],
                &body,
                "database connection",
            )
            .await?;

        Ok(database.secret_keys.clone())
    }

    async fn deploy(&self, request: &DeployRequest) -> Result<Deployment> {
        request.validate()?;

        let mut files = Vec::with_capacity(request.bundle.len());
        for file in request.bundle.files() {
            let sha = digest(file.contents());
            self.upload(&sha, file.contents()).await?;
            files.push(UploadedFile {
                file: file.path().to_owned(),
                sha,
                size: file.len(),
            });
        }

        let site = request.site.trim();
        let body = CreateDeployment {
            name: site,
            files,
            // A preview deployment omits the field. Vercel reads an absent
            // target as a preview, and rejects the literal string.
            target: match request.target {
                DeploymentTarget::Production => Some("production"),
                DeploymentTarget::Preview => None,
            },
            project_settings: ProjectSettings {
                framework: framework_param(&request.framework),
            },
        };

        let deployment: DeploymentBody = self
            .http
            .post_json(
                "/v13/deployments",
                &[
                    ("forceNew", "1".to_owned()),
                    ("skipAutoDetectionConfirmation", "1".to_owned()),
                ],
                &body,
                "deployment",
            )
            .await?;

        Ok(deployment.into_deployment(site))
    }

    async fn deployment(&self, id: &str) -> Result<Deployment> {
        let deployment: DeploymentBody = self
            .http
            .get_json(
                &format!("/v13/deployments/{}", encode_segment(id)),
                &[],
                "deployment",
            )
            .await?;

        Ok(deployment.into_deployment(""))
    }

    async fn list_deployments(&self, site: &str, limit: u32) -> Result<Vec<Deployment>> {
        let deployments: Deployments = self
            .http
            .get_json(
                "/v7/deployments",
                &[("projectId", site.to_owned()), ("limit", limit.to_string())],
                "deployment list",
            )
            .await?;

        Ok(deployments
            .deployments
            .into_iter()
            .map(|deployment| deployment.into_deployment(site))
            .collect())
    }

    async fn promote(&self, site: &str, deployment: &str) -> Result<()> {
        let project = self.project_id(site).await?;
        let builder = self.http.request(
            Method::POST,
            &format!(
                "/v10/projects/{}/promote/{}",
                encode_segment(&project),
                encode_segment(deployment)
            ),
            &[],
        );

        self.http.discard(builder, "promotion").await
    }

    async fn add_domain(&self, site: &str, domain: &str) -> Result<Domain> {
        let name = domain.trim();
        if name.is_empty() {
            return Err(Error::EmptyDomain);
        }

        let added: DomainBody = self
            .http
            .post_json(
                &format!("/v10/projects/{}/domains", encode_segment(site)),
                &[],
                &CreateDomain { name },
                "domain",
            )
            .await?;

        Ok(added.into_domain(site))
    }

    async fn list_domains(&self, site: &str) -> Result<Vec<Domain>> {
        let domains: Domains = self
            .http
            .get_json(
                &format!("/v9/projects/{}/domains", encode_segment(site)),
                &[],
                "domain list",
            )
            .await?;

        Ok(domains
            .domains
            .into_iter()
            .map(|domain| domain.into_domain(site))
            .collect())
    }

    async fn analytics(&self, query: &AnalyticsQuery) -> Result<AnalyticsSummary> {
        query.validate()?;

        let totals: AnalyticsEnvelope = self
            .http
            .get_json(
                "/v1/query/web-analytics/visits/count",
                &[
                    ("projectId", query.site.clone()),
                    ("since", query.since_ms.to_string()),
                    ("until", query.until_ms.to_string()),
                ],
                "analytics",
            )
            .await?;

        Ok(AnalyticsSummary {
            site: query.site.clone(),
            since_ms: query.since_ms,
            until_ms: query.until_ms,
            visitors: totals.data.get("visitors").and_then(Value::as_u64),
            pageviews: totals.data.get("pageviews").and_then(Value::as_u64),
            breakdown: self.breakdown(query).await?,
        })
    }
}

/// The `framework` value Vercel wants for a unified framework.
///
/// Static output has no framework on Vercel: the field is null, and sending
/// "static" is rejected.
fn framework_param(framework: &Framework) -> Option<&str> {
    match framework {
        Framework::Static => None,
        other => Some(other.as_str()),
    }
}

/// The environments an environment variable applies to.
///
/// An empty list means every environment, `development` included — a variable
/// the local `next dev` cannot see is a variable that works everywhere except
/// on the machine where it is being written.
fn env_targets(targets: &[DeploymentTarget]) -> Vec<&'static str> {
    if targets.is_empty() {
        return vec!["production", "preview", "development"];
    }

    let mut names: Vec<&'static str> = Vec::with_capacity(targets.len());
    for target in targets {
        let name = match target {
            DeploymentTarget::Production => "production",
            DeploymentTarget::Preview => "preview",
        };
        // A plain `Vec::dedup` only catches adjacent repeats; a caller-supplied
        // target list is not guaranteed sorted, so this checks the whole list
        // built so far instead.
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// The characters a Vercel API path segment does not need escaped: RFC 3986's
/// unreserved set, which is what a site name, deployment id, or domain is
/// ordinarily made of.
///
/// Everything else — `/`, `?`, `#` included — becomes a percent-escape, so a
/// caller-supplied identifier cannot add a path segment, a query string, or a
/// `..` of its own to an authenticated request.
const PATH_SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encodes `value` for use as one path segment.
fn encode_segment(value: &str) -> impl fmt::Display + '_ {
    percent_encoding::utf8_percent_encode(value, PATH_SEGMENT)
}

/// The SHA-1 digest of a deployment file, hex encoded, as `x-vercel-digest`
/// requires.
fn digest(contents: &[u8]) -> String {
    use sha1::{Digest as _, Sha1};

    let mut hasher = Sha1::new();
    hasher.update(contents);
    hex::encode(hasher.finalize())
}

/// Reads analytics breakdown rows out of the provider's untyped `data`.
///
/// Every numeric field becomes a metric, because providers do not agree on what
/// they count and dropping the ones this crate did not anticipate would be
/// silently losing the answer.
fn buckets(dimension: &str, data: &Value) -> Vec<AnalyticsBucket> {
    data.as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let row = row.as_object()?;
                    let metrics = row
                        .iter()
                        .filter_map(|(key, value)| Some((key.clone(), value.as_f64()?)))
                        .collect();

                    Some(AnalyticsBucket {
                        label: row
                            .get(dimension)
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        metrics,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod test;
