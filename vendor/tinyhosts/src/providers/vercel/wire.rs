//! Vercel's own request and response shapes, and their translation into the
//! unified model.
//!
//! Nothing here is public. The point of a separate module is that the provider's
//! vocabulary stops at its edge: `readyState`, `uid`, `icfg_`, `sensitive` and
//! `projectsMetadata` appear in this file and nowhere else in the crate.
//!
//! Response structs are permissive on purpose. Every field this crate does not
//! read is absent, and every field it reads is optional with a mapped default,
//! so a new field in a Vercel response cannot fail a deployment. Identifier
//! fields carry `alias` attributes because Vercel's list and detail endpoints
//! disagree about their names — `uid` against `id`, `state` against `readyState`.

use serde::{Deserialize, Serialize};

use crate::host::types::{
    Database, DatabaseKind, Deployment, DeploymentStatus, DeploymentTarget, Domain, EnvVarRecord,
    Framework, Site,
};

/// The body of `POST /v11/projects`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateProject<'a> {
    pub(super) name: &'a str,
    pub(super) framework: Option<&'a str>,
}

/// A project, as every project endpoint returns it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Project {
    pub(super) id: String,
    pub(super) name: String,
    #[serde(default)]
    pub(super) framework: Option<String>,
    #[serde(default)]
    pub(super) created_at: Option<u64>,
}

impl Project {
    /// Translates the project into the unified model.
    pub(super) fn into_site(self) -> Site {
        Site {
            id: self.id,
            name: self.name,
            framework: self.framework.as_deref().map(framework_of),
            created_at_ms: self.created_at,
        }
    }
}

/// The body of `GET /v10/projects`.
#[derive(Deserialize)]
pub(super) struct Projects {
    #[serde(default)]
    pub(super) projects: Vec<Project>,
}

/// One entry of the `files` array of `POST /v13/deployments`.
#[derive(Serialize)]
pub(super) struct UploadedFile {
    pub(super) file: String,
    pub(super) sha: String,
    pub(super) size: usize,
}

/// The build settings applied to a project on its first deployment.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProjectSettings<'a> {
    pub(super) framework: Option<&'a str>,
}

/// The body of `POST /v13/deployments`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateDeployment<'a> {
    pub(super) name: &'a str,
    pub(super) files: Vec<UploadedFile>,
    /// Omitted for a preview: Vercel reads a missing target as `preview`, and
    /// sending the string "preview" is not the same request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) target: Option<&'a str>,
    pub(super) project_settings: ProjectSettings<'a>,
}

/// A deployment, as both the create and the list endpoints return it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeploymentBody {
    #[serde(alias = "uid")]
    pub(super) id: String,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) url: Option<String>,
    #[serde(default, alias = "state")]
    pub(super) ready_state: Option<String>,
    #[serde(default)]
    pub(super) target: Option<String>,
    #[serde(default, alias = "created")]
    pub(super) created_at: Option<u64>,
    #[serde(default)]
    pub(super) error_message: Option<String>,
}

impl DeploymentBody {
    /// Translates the deployment into the unified model.
    ///
    /// `site` is the name the caller asked about, used when the response omits
    /// one.
    pub(super) fn into_deployment(self, site: &str) -> Deployment {
        Deployment {
            id: self.id,
            site: self.name.unwrap_or_else(|| site.to_owned()),
            // Vercel returns a bare host; every consumer wants a URL it can open.
            url: self.url.map(|host| {
                if host.starts_with("http://") || host.starts_with("https://") {
                    host
                } else {
                    format!("https://{host}")
                }
            }),
            status: status_of(self.ready_state.as_deref()),
            target: match self.target.as_deref() {
                Some("production") => DeploymentTarget::Production,
                _ => DeploymentTarget::Preview,
            },
            created_at_ms: self.created_at,
            error_message: self.error_message,
        }
    }
}

/// The body of `GET /v7/deployments`.
#[derive(Deserialize)]
pub(super) struct Deployments {
    #[serde(default)]
    pub(super) deployments: Vec<DeploymentBody>,
}

/// One entry of the `POST /v10/projects/{id}/env` array body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateEnvVar {
    pub(super) key: String,
    pub(super) value: String,
    /// `sensitive` is Vercel's write-only storage; `encrypted` is its default.
    pub(super) r#type: &'static str,
    pub(super) target: Vec<&'static str>,
}

/// A target field that Vercel returns as either a string or an array.
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Targets {
    One(String),
    Many(Vec<String>),
}

impl Targets {
    /// The targets as the unified model's list, dropping `development`, which
    /// has no unified equivalent — it is a local environment, not a deployment.
    fn into_targets(self) -> Vec<DeploymentTarget> {
        let names = match self {
            Self::One(name) => vec![name],
            Self::Many(names) => names,
        };
        names
            .into_iter()
            .filter_map(|name| match name.as_str() {
                "production" => Some(DeploymentTarget::Production),
                "preview" => Some(DeploymentTarget::Preview),
                _ => None,
            })
            .collect()
    }
}

/// An environment variable, as `GET /v10/projects/{id}/env` returns it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct EnvBody {
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) key: String,
    #[serde(default)]
    pub(super) target: Option<Targets>,
    #[serde(default)]
    pub(super) r#type: Option<String>,
}

impl EnvBody {
    /// Translates the variable into the unified model, without its value.
    pub(super) fn into_record(self) -> EnvVarRecord {
        EnvVarRecord {
            id: self.id.unwrap_or_default(),
            key: self.key,
            targets: self.target.map(Targets::into_targets).unwrap_or_default(),
            secret: self.r#type.as_deref() == Some("sensitive"),
        }
    }
}

/// The body of `GET /v10/projects/{id}/env`.
#[derive(Deserialize)]
pub(super) struct Envs {
    #[serde(default)]
    pub(super) envs: Vec<EnvBody>,
}

/// The body of `POST /v10/projects/{id}/domains`.
#[derive(Serialize)]
pub(super) struct CreateDomain<'a> {
    pub(super) name: &'a str,
}

/// A project domain.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DomainBody {
    pub(super) name: String,
    #[serde(default)]
    pub(super) verified: bool,
}

impl DomainBody {
    /// Translates the domain into the unified model.
    pub(super) fn into_domain(self, site: &str) -> Domain {
        Domain {
            name: self.name,
            site: site.to_owned(),
            verified: self.verified,
        }
    }
}

/// The body of `GET /v9/projects/{id}/domains`.
#[derive(Deserialize)]
pub(super) struct Domains {
    #[serde(default)]
    pub(super) domains: Vec<DomainBody>,
}

/// A marketplace installation on the account.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Configuration {
    pub(super) id: String,
    #[serde(default)]
    pub(super) slug: Option<String>,
}

/// A product one installation offers.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Product {
    pub(super) id: String,
    pub(super) slug: String,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) primary_protocol: Option<String>,
}

impl Product {
    /// Whether this product can serve `kind`.
    ///
    /// The match runs over the product's slug and name and the installation's
    /// slug, because a vendor's naming rarely contains the protocol its product
    /// speaks: the product is `serverless-postgres`, the installation is `neon`,
    /// and either one is enough to identify a Postgres.
    ///
    /// A product that declares a non-storage protocol is excluded outright. An
    /// observability integration should not be matched as a database because its
    /// name happens to contain "storage".
    pub(super) fn serves(&self, kind: &DatabaseKind, installation_slug: Option<&str>) -> bool {
        if self
            .primary_protocol
            .as_deref()
            .is_some_and(|protocol| protocol != "storage")
        {
            return false;
        }

        let haystack = format!(
            "{} {} {}",
            self.slug,
            self.name.as_deref().unwrap_or_default(),
            installation_slug.unwrap_or_default()
        )
        .to_ascii_lowercase();

        kind.product_hints()
            .iter()
            .any(|hint| haystack.contains(&hint.to_ascii_lowercase()))
    }
}

/// The body of `GET /v1/integrations/configuration/{id}/products`.
#[derive(Deserialize)]
pub(super) struct Products {
    #[serde(default)]
    pub(super) products: Vec<Product>,
}

/// The body of `POST /v1/storage/stores/integration/direct`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateStore<'a> {
    pub(super) name: &'a str,
    pub(super) integration_configuration_id: &'a str,
    pub(super) integration_product_id_or_slug: &'a str,
}

/// The product a store was created from.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct StoreProduct {
    #[serde(default)]
    pub(super) slug: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) integration_configuration_id: Option<String>,
}

/// One secret a connected project receives. Only its name is returned.
#[derive(Deserialize)]
pub(super) struct StoreSecret {
    pub(super) name: String,
}

/// A provisioned store.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Store {
    /// Vercel's own resource identifier, which is what a connection call needs.
    #[serde(default)]
    pub(super) id: Option<String>,
    /// The partner's identifier, present even when `id` is not.
    #[serde(default)]
    pub(super) external_resource_id: Option<String>,
    #[serde(default)]
    pub(super) name: Option<String>,
    #[serde(default)]
    pub(super) status: Option<String>,
    #[serde(default)]
    pub(super) secrets: Vec<StoreSecret>,
    #[serde(default)]
    pub(super) product: Option<StoreProduct>,
}

impl Store {
    /// Translates the store into the unified model.
    ///
    /// `id` is preferred over `external_resource_id`: the connection endpoint
    /// addresses the resource by Vercel's identifier, and the partner's is only
    /// a fallback so an inventory listing is never left without one.
    pub(super) fn into_database(self, requested: &str, kind: DatabaseKind) -> Database {
        let product = self.product;
        Database {
            id: self
                .id
                .or(self.external_resource_id)
                .unwrap_or_else(|| requested.to_owned()),
            name: self.name.unwrap_or_else(|| requested.to_owned()),
            kind,
            product: product
                .as_ref()
                .and_then(|product| product.slug.clone().or_else(|| product.name.clone())),
            status: self.status.unwrap_or_else(|| "unknown".to_owned()),
            secret_keys: self.secrets.into_iter().map(|secret| secret.name).collect(),
            installation_id: product.and_then(|product| product.integration_configuration_id),
        }
    }
}

/// The body of `POST /v1/storage/stores/integration/direct`'s response.
#[derive(Deserialize)]
pub(super) struct StoreEnvelope {
    #[serde(default)]
    pub(super) store: Option<Store>,
}

/// The body of a connection request.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ConnectResource<'a> {
    pub(super) project_id: &'a str,
    pub(super) env_var_environments: Vec<&'static str>,
}

/// A web analytics response. The `data` shape depends on the query, so it stays
/// untyped here and is read by the caller that knows which query it sent.
#[derive(Deserialize)]
pub(super) struct AnalyticsEnvelope {
    #[serde(default)]
    pub(super) data: serde_json::Value,
}

/// Maps Vercel's `readyState` onto the unified status.
fn status_of(state: Option<&str>) -> DeploymentStatus {
    match state {
        // A deployment Vercel has not described yet has been accepted but not
        // started, which is what queued means.
        Some("QUEUED") | None => DeploymentStatus::Queued,
        Some("INITIALIZING" | "BUILDING") => DeploymentStatus::Building,
        Some("READY") => DeploymentStatus::Ready,
        Some("ERROR") => DeploymentStatus::Failed,
        Some("CANCELED") => DeploymentStatus::Canceled,
        Some(other) => DeploymentStatus::Other(other.to_owned()),
    }
}

/// Maps Vercel's `framework` onto the unified framework.
fn framework_of(name: &str) -> Framework {
    match name {
        "nextjs" => Framework::NextJs,
        "static" => Framework::Static,
        other => Framework::Other(other.to_owned()),
    }
}
