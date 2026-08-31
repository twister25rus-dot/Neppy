//! The hosting providers this crate can talk to.
//!
//! [`ProviderKind`] names a provider, says which environment variables hold its
//! credential, and [`connect`] turns a kind and a credential into a live
//! [`Host`]. A caller that goes through here never names a concrete adapter
//! type, which is what lets the provider be a configuration value rather than a
//! compile-time choice.
//!
//! A provider whose Cargo feature is off is still a [`ProviderKind`] — it can be
//! parsed, stored, and displayed — but [`connect`] refuses it with
//! [`Error::UnknownProvider`]. Failing at the connection is the useful place:
//! configuration is read long before a build's feature set is known.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::host::Host;
use crate::{Credentials, Error, Result};

#[cfg(feature = "vercel")]
pub mod vercel;

/// A hosting provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderKind {
    /// Vercel: <https://vercel.com>.
    #[default]
    Vercel,
}

impl ProviderKind {
    /// The provider's slug, as it appears in configuration and on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vercel => "vercel",
        }
    }

    /// The environment variables that may hold this provider's API key, in the
    /// order they are searched.
    ///
    /// The `TINYHOSTS_`-prefixed name comes first so a host running several
    /// tools can give this crate its own token without disturbing the
    /// provider's own CLI.
    #[must_use]
    pub const fn api_key_variables(self) -> &'static [&'static str] {
        match self {
            Self::Vercel => &["TINYHOSTS_VERCEL_TOKEN", "VERCEL_TOKEN"],
        }
    }

    /// The environment variables that may hold the team to act as.
    #[must_use]
    pub const fn team_variables(self) -> &'static [&'static str] {
        match self {
            Self::Vercel => &["TINYHOSTS_VERCEL_TEAM_ID", "VERCEL_TEAM_ID"],
        }
    }

    /// Reads this provider's credential from the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MissingApiKey`], naming every variable it searched, when
    /// none of them holds a non-blank value.
    pub fn credentials_from_env(self) -> Result<Credentials> {
        self.credentials_from(&|name| std::env::var(name).ok())
    }

    /// Reads this provider's credential through an arbitrary lookup.
    ///
    /// This is the testable form of [`credentials_from_env`](Self::credentials_from_env),
    /// and the one to use when the values come from somewhere other than the
    /// environment — a secret store, or a row in a database.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MissingApiKey`] when the lookup yields no non-blank API
    /// key.
    pub fn credentials_from(self, lookup: &impl Fn(&str) -> Option<String>) -> Result<Credentials> {
        let first_set = |names: &[&str]| {
            names
                .iter()
                .filter_map(|name| lookup(name))
                .find(|value| !value.trim().is_empty())
        };

        let api_key = first_set(self.api_key_variables()).ok_or_else(|| Error::MissingApiKey {
            provider: self.as_str().to_owned(),
            variables: self.api_key_variables().join(", "),
        })?;

        let credentials = Credentials::new(api_key)?;
        Ok(match first_set(self.team_variables()) {
            Some(team) => credentials.with_team(team),
            None => credentials,
        })
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ProviderKind {
    type Err = Error;

    /// Parses a provider slug, case-insensitively.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownProvider`] for anything else.
    fn from_str(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "vercel" => Ok(Self::Vercel),
            _ => Err(Error::UnknownProvider {
                name: name.to_owned(),
            }),
        }
    }
}

/// Connects to `kind` with `credentials`.
///
/// Nothing is sent: the returned host is a client, and the credential is not
/// checked until the first call that uses it.
///
/// # Errors
///
/// Returns [`Error::UnknownProvider`] when this build has no adapter for `kind`,
/// or a provider error when the client cannot be constructed.
pub fn connect(kind: ProviderKind, credentials: Credentials) -> Result<Box<dyn Host>> {
    connect_to(kind, credentials, None)
}

/// Connects to `kind`, optionally through a different API root.
///
/// `base_url` exists for a deployment that reaches the provider through an
/// egress proxy, and for a test suite that runs the adapter against a local mock
/// of the provider's API. `None` uses the provider's own root.
///
/// # Errors
///
/// Returns [`Error::InsecureBaseUrl`] when `base_url` is `http://` against a
/// non-loopback host, [`Error::UnknownProvider`] when this build has no adapter
/// for `kind`, or a provider error when the client cannot be constructed.
pub fn connect_to(
    kind: ProviderKind,
    credentials: Credentials,
    base_url: Option<&str>,
) -> Result<Box<dyn Host>> {
    match base_url {
        Some(base_url) if !is_secure_base_url(base_url) => {
            return Err(Error::InsecureBaseUrl {
                base_url: base_url.to_owned(),
            });
        }
        Some(_) | None => {}
    }

    match kind {
        #[cfg(feature = "vercel")]
        ProviderKind::Vercel => Ok(Box::new(match base_url {
            Some(base_url) => vercel::Vercel::with_base_url(credentials, base_url)?,
            None => vercel::Vercel::new(credentials)?,
        })),
        #[cfg(not(feature = "vercel"))]
        ProviderKind::Vercel => {
            let _ = (credentials, base_url);
            Err(Error::UnknownProvider {
                name: kind.as_str().to_owned(),
            })
        }
    }
}

/// Whether `base_url` may carry a bearer credential.
///
/// Accepts every `https://` root. Accepts `http://` only when its host is
/// `localhost`, `127.0.0.1`/`127.x.x.x`, or `::1` — a mock server's or an
/// egress proxy's loopback listener never leaves the machine, so plain HTTP
/// there costs nothing a caller could intercept.
///
/// `pub(crate)` because [`vercel::Vercel::with_base_url`] applies the same
/// check: it is a second, lower-level way to reach the same client
/// [`connect_to`] builds, and skipping the check there would just move the
/// cleartext-credential exposure this guards against to a different entry
/// point rather than closing it.
pub(crate) fn is_secure_base_url(base_url: &str) -> bool {
    if let Some(rest) = base_url.strip_prefix("https://") {
        return !rest.is_empty();
    }
    let Some(rest) = base_url.strip_prefix("http://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(host, _)| host);
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host.starts_with("127.")
}

/// Connects to `kind` with the credential in the process environment.
///
/// # Errors
///
/// Returns [`Error::MissingApiKey`] when the environment holds no key for
/// `kind`, or anything [`connect`] returns.
pub fn connect_from_env(kind: ProviderKind) -> Result<Box<dyn Host>> {
    connect(kind, kind.credentials_from_env()?)
}

#[cfg(test)]
mod test;
