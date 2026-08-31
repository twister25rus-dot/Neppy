//! The provider-agnostic vocabulary every host is described in.
//!
//! These types are the standard: a site, a deployment of it, the environment it
//! reads, a managed database, a custom domain, and the traffic it served. A
//! provider adapter's whole job is translating its own API into them, so a
//! caller that can ship a Next.js application to one host can ship it to the
//! next without learning a second vocabulary.
//!
//! Records the provider produced ([`Site`], [`Deployment`], [`Database`]) carry
//! public fields and no invariants — they are whatever the provider said.
//! Requests the caller produces ([`SiteSpec`], [`DeployRequest`], [`EnvVar`],
//! [`AnalyticsQuery`]) carry a [`validate`](SiteSpec::validate) that the
//! adapter calls before spending a network round trip. Validation lives at the
//! point of use rather than in a constructor because every one of these types
//! also arrives by deserialization, where a constructor cannot intercept it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::bundle::Bundle;
use crate::{Error, Result};

/// The framework a site is built with.
///
/// The framework decides the build, so it is part of the site rather than of a
/// single deployment. [`Framework::NextJs`] is the default because it is what
/// this crate was built to ship.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Framework {
    /// A Next.js application, built by the provider from its source.
    #[default]
    NextJs,
    /// Pre-built static files, served as they are.
    Static,
    /// Anything else, named the way the provider names it.
    Other(String),
}

impl Framework {
    /// The provider-independent slug for this framework.
    ///
    /// It happens to match Vercel's `framework` values for the two named
    /// variants, which is why [`Framework::Other`] passes through untouched.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::NextJs => "nextjs",
            Self::Static => "static",
            Self::Other(name) => name,
        }
    }
}

/// Which environment a deployment serves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentTarget {
    /// A preview URL, not attached to the site's domains.
    #[default]
    Preview,
    /// The live site, attached to every domain it has.
    Production,
}

impl DeploymentTarget {
    /// The provider-independent slug for this target.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Production => "production",
        }
    }
}

/// What the caller wants a site to be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteSpec {
    /// The site's name, unique within the account.
    pub name: String,
    /// The framework the provider should build it with.
    #[serde(default)]
    pub framework: Framework,
}

impl SiteSpec {
    /// A Next.js site called `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            framework: Framework::NextJs,
        }
    }

    /// Builds the site with a different framework.
    #[must_use]
    pub fn with_framework(mut self, framework: Framework) -> Self {
        self.framework = framework;
        self
    }

    /// Checks the spec before an adapter spends a network round trip on it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptySiteName`] when the name is blank.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::EmptySiteName);
        }
        Ok(())
    }
}

/// A site that exists on a provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Site {
    /// The provider's identifier for it.
    pub id: String,
    /// Its name, which is what the deployment API keys on.
    pub name: String,
    /// The framework the provider believes it is built with, when it says.
    #[serde(default)]
    pub framework: Option<Framework>,
    /// When it was created, in milliseconds since the Unix epoch.
    #[serde(default)]
    pub created_at_ms: Option<u64>,
}

/// A request to deploy a bundle of files as a site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployRequest {
    /// The site to deploy to, by name.
    pub site: String,
    /// The framework to build with.
    #[serde(default)]
    pub framework: Framework,
    /// The environment this deployment serves.
    #[serde(default)]
    pub target: DeploymentTarget,
    /// The files to deploy.
    pub bundle: Bundle,
}

impl DeployRequest {
    /// A preview deployment of `bundle` to the Next.js site `site`.
    #[must_use]
    pub fn new(site: impl Into<String>, bundle: Bundle) -> Self {
        Self {
            site: site.into(),
            framework: Framework::NextJs,
            target: DeploymentTarget::Preview,
            bundle,
        }
    }

    /// Sends the deployment to `target` instead of a preview URL.
    #[must_use]
    pub fn with_target(mut self, target: DeploymentTarget) -> Self {
        self.target = target;
        self
    }

    /// Builds with a different framework.
    #[must_use]
    pub fn with_framework(mut self, framework: Framework) -> Self {
        self.framework = framework;
        self
    }

    /// Checks the request before an adapter starts uploading files.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptySiteName`] when the site name is blank, or
    /// [`Error::EmptyBundle`] when there is nothing to deploy.
    pub fn validate(&self) -> Result<()> {
        if self.site.trim().is_empty() {
            return Err(Error::EmptySiteName);
        }
        if self.bundle.is_empty() {
            return Err(Error::EmptyBundle);
        }
        Ok(())
    }
}

/// How far along a deployment is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DeploymentStatus {
    /// Accepted, not started.
    Queued,
    /// Building.
    Building,
    /// Live and serving.
    Ready,
    /// The build or the upload failed.
    Failed,
    /// Cancelled before it finished.
    Canceled,
    /// A provider state this crate does not model, named as the provider named
    /// it. Reported rather than mapped onto a state it may not mean.
    Other(String),
}

impl DeploymentStatus {
    /// Whether the deployment has stopped changing.
    ///
    /// A poller stops here. [`DeploymentStatus::Other`] counts as non-terminal:
    /// an unknown state is more likely a stage of the build than the end of it,
    /// and a poller that gives up early reports a live site as a failure.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Ready | Self::Failed | Self::Canceled)
    }

    /// Whether the deployment finished and is serving traffic.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// A deployment of a site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Deployment {
    /// The provider's identifier, which is what a status poll asks about.
    pub id: String,
    /// The site it belongs to, by name.
    pub site: String,
    /// Where it is served, as an absolute URL, once the provider assigns one.
    #[serde(default)]
    pub url: Option<String>,
    /// How far along it is.
    pub status: DeploymentStatus,
    /// Which environment it serves.
    #[serde(default)]
    pub target: DeploymentTarget,
    /// When it was created, in milliseconds since the Unix epoch.
    #[serde(default)]
    pub created_at_ms: Option<u64>,
    /// The provider's failure message, when it failed.
    #[serde(default)]
    pub error_message: Option<String>,
}

/// An environment variable to set on a site.
///
/// The value is write-only across this API: it goes out in a request and is
/// never returned, because a provider that hands back decrypted secrets on a
/// list call is a provider this crate would be leaking through.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    /// The variable's name.
    pub key: String,
    /// Its value.
    pub value: String,
    /// The environments it applies to. Empty means every environment.
    #[serde(default)]
    pub targets: Vec<DeploymentTarget>,
    /// Whether the provider should store it write-only.
    #[serde(default)]
    pub secret: bool,
}

/// Prints the key and targets, never the value.
///
/// `EnvVar` reaches [`Operation::SetEnv`](crate::rpc::Operation::SetEnv) and
/// [`LaunchPlan`](crate::launch::types::LaunchPlan), both of which derive
/// `Debug`; a derived `Debug` here would put a secret's plaintext value in
/// whatever log line renders one of those.
impl std::fmt::Debug for EnvVar {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnvVar")
            .field("key", &self.key)
            .field("value", &"<redacted>")
            .field("targets", &self.targets)
            .field("secret", &self.secret)
            .finish()
    }
}

impl EnvVar {
    /// A variable set in every environment.
    #[must_use]
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            targets: Vec::new(),
            secret: false,
        }
    }

    /// Restricts the variable to `targets`.
    #[must_use]
    pub fn with_targets(mut self, targets: Vec<DeploymentTarget>) -> Self {
        self.targets = targets;
        self
    }

    /// Marks the variable as a secret the provider should not read back.
    #[must_use]
    pub fn secret(mut self) -> Self {
        self.secret = true;
        self
    }

    /// Checks the variable before it is sent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyEnvKey`] when the name is blank.
    pub fn validate(&self) -> Result<()> {
        if self.key.trim().is_empty() {
            return Err(Error::EmptyEnvKey);
        }
        Ok(())
    }
}

/// An environment variable that exists on a site, without its value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVarRecord {
    /// The provider's identifier for it.
    pub id: String,
    /// The variable's name.
    pub key: String,
    /// The environments it applies to.
    #[serde(default)]
    pub targets: Vec<DeploymentTarget>,
    /// Whether the provider stores it write-only.
    #[serde(default)]
    pub secret: bool,
}

/// The kind of managed database to provision.
///
/// A kind is a protocol, not a product: which vendor supplies a Postgres is the
/// provider's business, and on Vercel it depends on which marketplace
/// integration the account has installed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DatabaseKind {
    /// A Postgres database. The default: it is what a Next.js application with
    /// an ORM expects to find.
    #[default]
    Postgres,
    /// A Redis-compatible key-value store.
    Redis,
    /// Blob or object storage.
    Blob,
    /// Another protocol, matched against the provider's product names.
    Other(String),
}

impl DatabaseKind {
    /// The provider-independent slug for this kind.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Postgres => "postgres",
            Self::Redis => "redis",
            Self::Blob => "blob",
            Self::Other(name) => name,
        }
    }

    /// Product-name fragments that identify this kind on a provider.
    ///
    /// A managed Postgres is rarely called "postgres" in a catalogue — it is
    /// Neon, or Supabase, or Prisma. Matching a kind to a product means
    /// matching against the names vendors actually use.
    #[must_use]
    pub fn product_hints(&self) -> Vec<&str> {
        match self {
            Self::Postgres => vec![
                "postgres",
                "neon",
                "supabase",
                "prisma-postgres",
                "timescale",
            ],
            Self::Redis => vec!["redis", "upstash", "kv", "valkey"],
            Self::Blob => vec!["blob", "storage", "bucket", "s3"],
            Self::Other(name) => vec![name],
        }
    }
}

/// What the caller wants a database to be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseSpec {
    /// The database's name within the account.
    pub name: String,
    /// The kind of database.
    #[serde(default)]
    pub kind: DatabaseKind,
    /// A specific provider product to use, overriding the kind's matching.
    ///
    /// Set this when an account has several products that could serve the kind
    /// and the choice matters — it is the only escape hatch from
    /// [`DatabaseKind::product_hints`].
    #[serde(default)]
    pub product: Option<String>,
}

impl DatabaseSpec {
    /// A Postgres database called `name`.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            kind: DatabaseKind::Postgres,
            product: None,
        }
    }

    /// Provisions a different kind of database.
    #[must_use]
    pub fn with_kind(mut self, kind: DatabaseKind) -> Self {
        self.kind = kind;
        self
    }

    /// Pins the provider product instead of matching on the kind.
    #[must_use]
    pub fn with_product(mut self, product: impl Into<String>) -> Self {
        self.product = Some(product.into());
        self
    }

    /// Checks the spec before an adapter provisions anything.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptySiteName`] when the name is blank. A database and
    /// a site share the rule and the variant: both are named resources on the
    /// account.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::EmptySiteName);
        }
        Ok(())
    }
}

/// A managed database that exists on a provider.
///
/// `secret_keys` names the environment variables a connected site receives —
/// `DATABASE_URL` and friends — without their values. The values are the
/// provider's to inject; this crate never holds a connection string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Database {
    /// The provider's identifier for the database.
    pub id: String,
    /// Its name.
    pub name: String,
    /// The kind it serves.
    pub kind: DatabaseKind,
    /// The provider product behind it, when the provider names one.
    #[serde(default)]
    pub product: Option<String>,
    /// The provider's status for it, verbatim.
    pub status: String,
    /// The names of the environment variables a connected site receives.
    #[serde(default)]
    pub secret_keys: Vec<String>,
    /// The scope a later connection call needs, when the provider requires one.
    ///
    /// On Vercel this is the marketplace installation the store belongs to;
    /// connecting the store to a project needs both identifiers.
    #[serde(default)]
    pub installation_id: Option<String>,
}

/// A custom domain on a site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Domain {
    /// The domain name.
    pub name: String,
    /// The site it points at.
    pub site: String,
    /// Whether the provider has verified ownership.
    pub verified: bool,
}

/// A dimension to break analytics down by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AnalyticsDimension {
    /// The visitor's country.
    Country,
    /// Desktop, mobile, or tablet.
    DeviceType,
    /// The requested path.
    RequestPath,
    /// The referring host.
    ReferrerHostname,
    /// The visitor's browser.
    BrowserName,
    /// The visitor's operating system.
    OsName,
    /// The matched application route.
    Route,
}

impl AnalyticsDimension {
    /// The provider-independent slug for this dimension.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Country => "country",
            Self::DeviceType => "deviceType",
            Self::RequestPath => "requestPath",
            Self::ReferrerHostname => "referrerHostname",
            Self::BrowserName => "browserName",
            Self::OsName => "osName",
            Self::Route => "route",
        }
    }
}

/// A window of traffic to report on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalyticsQuery {
    /// The site to report on, by name or identifier.
    pub site: String,
    /// The start of the window, in milliseconds since the Unix epoch.
    pub since_ms: u64,
    /// The end of the window, in milliseconds since the Unix epoch.
    pub until_ms: u64,
    /// A dimension to break the totals down by, when one is wanted.
    #[serde(default)]
    pub breakdown: Option<AnalyticsDimension>,
    /// How many rows the breakdown may return.
    #[serde(default = "default_analytics_limit")]
    pub limit: u32,
}

const fn default_analytics_limit() -> u32 {
    10
}

impl AnalyticsQuery {
    /// A window between two epoch-millisecond timestamps.
    #[must_use]
    pub fn new(site: impl Into<String>, since_ms: u64, until_ms: u64) -> Self {
        Self {
            site: site.into(),
            since_ms,
            until_ms,
            breakdown: None,
            limit: default_analytics_limit(),
        }
    }

    /// Breaks the totals down by `dimension`.
    #[must_use]
    pub fn with_breakdown(mut self, dimension: AnalyticsDimension) -> Self {
        self.breakdown = Some(dimension);
        self
    }

    /// Returns at most `limit` breakdown rows.
    #[must_use]
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = limit;
        self
    }

    /// Checks the query before an adapter sends it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptySiteName`] when the site is blank, or
    /// [`Error::InvalidAnalyticsWindow`] when the window does not move forward.
    pub fn validate(&self) -> Result<()> {
        if self.site.trim().is_empty() {
            return Err(Error::EmptySiteName);
        }
        if self.until_ms <= self.since_ms {
            return Err(Error::InvalidAnalyticsWindow);
        }
        Ok(())
    }
}

/// One row of an analytics breakdown.
///
/// `metrics` holds whatever numbers the provider returned for the row rather
/// than a fixed pair of fields. Providers do not agree on what they count, and
/// a struct with `pageviews` and `visitors` would either drop a provider's
/// numbers or invent them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsBucket {
    /// The dimension value this row is for.
    pub label: String,
    /// The metrics the provider reported for it.
    pub metrics: BTreeMap<String, f64>,
}

/// Traffic a site served over a window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalyticsSummary {
    /// The site the numbers are for.
    pub site: String,
    /// The window's start, in milliseconds since the Unix epoch.
    pub since_ms: u64,
    /// The window's end, in milliseconds since the Unix epoch.
    pub until_ms: u64,
    /// Distinct visitors, when the provider counts them.
    #[serde(default)]
    pub visitors: Option<u64>,
    /// Page views, when the provider counts them.
    #[serde(default)]
    pub pageviews: Option<u64>,
    /// The requested breakdown, when one was asked for.
    #[serde(default)]
    pub breakdown: Vec<AnalyticsBucket>,
}
