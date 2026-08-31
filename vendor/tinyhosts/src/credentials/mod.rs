//! The credential a hosting provider is called with.
//!
//! A user pastes an API key into `OpenCompany`; it reaches this crate as a
//! [`Credentials`], is attached to every outbound request, and is never
//! rendered. [`Credentials`] therefore has a hand-written [`Debug`] that
//! redacts the key, and deliberately implements [`serde::Deserialize`] without
//! [`serde::Serialize`] — a value can be read out of a request envelope, and
//! cannot be written back into a response, a log line, or a ledger.
//!
//! Which environment variables hold a key is a per-provider fact, so reading
//! one from the environment belongs to
//! [`ProviderKind`](crate::ProviderKind::credentials_from_env) rather than here.

use std::fmt;

use crate::{Error, Result};

/// An API key for one hosting provider account, and the team to act as.
///
/// # Examples
///
/// ```
/// # use tinyhosts::Credentials;
/// let credentials = Credentials::new("vercel-token")?.with_team("team_abc");
///
/// assert_eq!(credentials.api_key(), "vercel-token");
/// assert_eq!(credentials.team(), Some("team_abc"));
/// // The key never reaches a rendered string.
/// assert!(!format!("{credentials:?}").contains("vercel-token"));
/// # Ok::<(), tinyhosts::Error>(())
/// ```
#[derive(Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(try_from = "Wire")]
pub struct Credentials {
    api_key: String,
    team: Option<String>,
}

impl Credentials {
    /// Builds a credential from an API key, trimming surrounding whitespace.
    ///
    /// # Errors
    ///
    /// Returns [`Error::EmptyApiKey`] when `api_key` is empty or contains only
    /// whitespace.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into().trim().to_owned();
        if api_key.is_empty() {
            return Err(Error::EmptyApiKey);
        }

        Ok(Self {
            api_key,
            team: None,
        })
    }

    /// Acts on behalf of a team, organization, or account scope.
    ///
    /// A blank team is the same as none, so a form field left empty does not
    /// become a query parameter the provider rejects.
    #[must_use]
    pub fn with_team(mut self, team: impl Into<String>) -> Self {
        let team = team.into().trim().to_owned();
        self.team = if team.is_empty() { None } else { Some(team) };
        self
    }

    /// The API key, for the provider client that signs a request with it.
    #[must_use]
    pub fn api_key(&self) -> &str {
        &self.api_key
    }

    /// The team scope, when one was supplied.
    #[must_use]
    pub fn team(&self) -> Option<&str> {
        self.team.as_deref()
    }

    /// Consumes the credential into its API key and team.
    ///
    /// A provider client holds the key for its whole life, so it takes
    /// ownership rather than copying out of a borrow it would have to keep.
    #[must_use]
    pub fn into_parts(self) -> (String, Option<String>) {
        (self.api_key, self.team)
    }
}

/// Redacts the key. Everything that prints a credential goes through here.
impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("api_key", &"<redacted>")
            .field("team", &self.team)
            .finish()
    }
}

/// The deserialized form, so a value read from JSON is validated exactly like
/// one built through [`Credentials::new`].
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    api_key: String,
    #[serde(default)]
    team: Option<String>,
}

impl TryFrom<Wire> for Credentials {
    type Error = Error;

    fn try_from(wire: Wire) -> Result<Self> {
        let credentials = Self::new(wire.api_key)?;
        Ok(match wire.team {
            Some(team) => credentials.with_team(team),
            None => credentials,
        })
    }
}

#[cfg(test)]
mod test;
