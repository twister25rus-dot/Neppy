//! The HTTP plumbing shared by every Vercel call.
//!
//! One place owns the base URL, the bearer token, the `teamId` query parameter
//! every request needs when a team is in play, and — most importantly — the
//! mapping from an HTTP status to an [`Error`] variant. A per-call mapping is
//! how a 403 ends up reported as a decoding failure in one code path and a
//! missing resource in another.

use std::fmt;
use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::providers::ProviderKind;
use crate::{Credentials, Error, Result};

/// Vercel's public API root.
pub(crate) const DEFAULT_BASE_URL: &str = "https://api.vercel.com";

/// How long a TCP handshake may take before this crate gives up on it.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// How long one request may run, upload included.
///
/// Vercel's own guidance and this crate's own docs agree a bundle is the
/// largest payload it moves, so this is generous rather than tight: a
/// `reqwest` client otherwise has no timeout at all, which turns a stalled
/// connection into a hang this crate's caller can never see or cancel.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// An authenticated Vercel API client.
pub(crate) struct Http {
    client: Client,
    base_url: String,
    token: String,
    team: Option<String>,
}

/// Prints the base URL and the team, never the token.
impl fmt::Debug for Http {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Http")
            .field("base_url", &self.base_url)
            .field("team", &self.team)
            .field("client", &self.client)
            .field("token", &"<redacted>")
            .finish()
    }
}

impl Http {
    /// Builds a client for `credentials` against `base_url`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`] when the HTTP client cannot be built, which
    /// in practice means no usable TLS backend.
    pub(crate) fn new(credentials: Credentials, base_url: impl Into<String>) -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("tinyhosts/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| Error::Transport {
                provider: provider(),
                reason: error.to_string(),
            })?;

        let (token, team) = credentials.into_parts();
        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            token,
            team,
        })
    }

    /// Starts a request, applying the bearer token and the team scope.
    pub(crate) fn request(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, String)],
    ) -> RequestBuilder {
        let mut builder = self
            .client
            .request(method, format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .query(query);

        if let Some(team) = &self.team {
            builder = builder.query(&[("teamId", team)]);
        }
        builder
    }

    /// Sends a request and decodes a JSON body.
    ///
    /// # Errors
    ///
    /// Returns the mapped provider error, or [`Error::Decode`] when the body is
    /// not the shape this crate expects.
    pub(crate) async fn json<T: DeserializeOwned>(
        &self,
        builder: RequestBuilder,
        resource: &str,
    ) -> Result<T> {
        let response = self.send(builder, resource).await?;
        let bytes = response.bytes().await.map_err(|error| Error::Transport {
            provider: provider(),
            reason: error.to_string(),
        })?;

        serde_json::from_slice(&bytes).map_err(|error| Error::Decode {
            provider: provider(),
            resource: resource.to_owned(),
            reason: error.to_string(),
        })
    }

    /// Sends a request, decoding a JSON body but treating "missing" as `None`.
    ///
    /// # Errors
    ///
    /// Returns the mapped provider error for anything but a 404, or
    /// [`Error::Decode`] for an unexpected body.
    pub(crate) async fn optional_json<T: DeserializeOwned>(
        &self,
        builder: RequestBuilder,
        resource: &str,
    ) -> Result<Option<T>> {
        match self.json(builder, resource).await {
            Ok(value) => Ok(Some(value)),
            Err(Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Sends a request and discards a successful body.
    ///
    /// # Errors
    ///
    /// Returns the mapped provider error.
    pub(crate) async fn discard(&self, builder: RequestBuilder, resource: &str) -> Result<()> {
        self.send(builder, resource).await.map(|_| ())
    }

    /// Sends a JSON body and decodes a JSON response.
    ///
    /// # Errors
    ///
    /// Returns the mapped provider error, or [`Error::Decode`].
    pub(crate) async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: &B,
        resource: &str,
    ) -> Result<T> {
        let builder = self.request(Method::POST, path, query).json(body);
        self.json(builder, resource).await
    }

    /// Sends a JSON body and discards the response.
    ///
    /// # Errors
    ///
    /// Returns the mapped provider error.
    pub(crate) async fn post_discard<B: Serialize>(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: &B,
        resource: &str,
    ) -> Result<()> {
        let builder = self.request(Method::POST, path, query).json(body);
        self.discard(builder, resource).await
    }

    /// Reads a JSON resource.
    ///
    /// # Errors
    ///
    /// Returns the mapped provider error, or [`Error::Decode`].
    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        resource: &str,
    ) -> Result<T> {
        let builder = self.request(Method::GET, path, query);
        self.json(builder, resource).await
    }

    /// Sends a request and maps a failed status onto an [`Error`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`] when the request never completed, and
    /// otherwise the variant matching the status: 401 [`Error::Unauthorized`],
    /// 403 [`Error::Forbidden`], 404 [`Error::NotFound`], 429
    /// [`Error::RateLimited`], anything else [`Error::Api`].
    async fn send(&self, builder: RequestBuilder, resource: &str) -> Result<Response> {
        let response = builder.send().await.map_err(|error| Error::Transport {
            provider: provider(),
            reason: error.to_string(),
        })?;

        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }

        let message = message_of(response).await;
        Err(match status {
            StatusCode::UNAUTHORIZED => Error::Unauthorized {
                provider: provider(),
            },
            StatusCode::FORBIDDEN => Error::Forbidden {
                provider: provider(),
                resource: resource.to_owned(),
            },
            StatusCode::NOT_FOUND => Error::NotFound {
                provider: provider(),
                resource: resource.to_owned(),
            },
            StatusCode::TOO_MANY_REQUESTS => Error::RateLimited {
                provider: provider(),
            },
            other => Error::Api {
                provider: provider(),
                status: other.as_u16(),
                resource: resource.to_owned(),
                message,
            },
        })
    }
}

/// Reads the message out of a failed response.
///
/// Vercel wraps failures as `{"error": {"code": ..., "message": ...}}`. A body
/// that is not that shape is reported verbatim, because an HTML error page from
/// a proxy in front of the API is exactly the kind of thing a reader needs to
/// see rather than have summarized as "unexpected response".
async fn message_of(response: Response) -> String {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if let Ok(envelope) = serde_json::from_str::<ErrorEnvelope>(&body) {
        return envelope.error.message;
    }

    let trimmed = body.trim();
    if trimmed.is_empty() {
        return status.canonical_reason().unwrap_or("unknown").to_owned();
    }
    trimmed.chars().take(400).collect()
}

/// Vercel's error body.
#[derive(serde::Deserialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(serde::Deserialize)]
struct ErrorDetail {
    message: String,
}

/// The provider name every error from this module carries.
fn provider() -> String {
    ProviderKind::Vercel.as_str().to_owned()
}
