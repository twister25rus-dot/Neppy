//! Binding, serving and shutting down the local backend.

use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::routes;
use super::state::LocalBackendState;
use super::LOG_PREFIX;
use crate::openhuman::config::Config;
use crate::openhuman::local_mode::resolve;

/// A running local backend. Dropping the handle does **not** stop the server —
/// call [`LocalBackendHandle::shutdown`] — because the runtime holds it for the
/// process lifetime and an implicit stop-on-drop would make an early return in
/// a caller silently take the control plane down.
pub struct LocalBackendHandle {
    addr: SocketAddr,
    base_url: String,
    session_token: String,
    shutdown: CancellationToken,
    join: JoinHandle<()>,
}

impl LocalBackendHandle {
    /// The address actually bound, which differs from the configured port when
    /// an ephemeral one was requested.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Base URL clients should use, e.g. `http://127.0.0.1:43117`.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The device session bearer this server accepts. Handed to in-process
    /// callers so they can authenticate without reading it back off the wire.
    pub fn session_token(&self) -> &str {
        &self.session_token
    }

    /// Stop serving and wait for the task to finish.
    pub async fn shutdown(self) {
        tracing::info!("{LOG_PREFIX} shutdown requested addr={}", self.addr);
        self.shutdown.cancel();
        resolve::clear_local_backend_base_url();
        if let Err(error) = self.join.await {
            tracing::warn!("{LOG_PREFIX} server task did not join cleanly: {error}");
        }
    }
}

/// Assemble the router.
///
/// Route families are `merge`d rather than `nest`ed because the hosted paths
/// they serve are absolute (`/auth/me`, `/teams/me/usage`) and nesting would
/// require every module to be written against a prefix that does not exist in
/// the contract they are matching.
pub(crate) fn build_router(state: LocalBackendState) -> Router {
    let proxy_inference = state.proxy_inference;

    let mut router = Router::new()
        .merge(routes::meta::router())
        .merge(routes::auth::router())
        .merge(routes::teams::router());

    if proxy_inference {
        router = router.merge(routes::inference::router());
    } else {
        tracing::info!(
            "{LOG_PREFIX} inference proxy disabled — /openai/v1/* will answer unsupported"
        );
    }

    router
        // Unserved hosted routes get the structured error, never a bare 404.
        .fallback(routes::unsupported::fallback)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            super::auth::require_bearer,
        ))
        .with_state(state)
}

/// Bind the local backend on loopback and start serving.
///
/// # Binding
///
/// `127.0.0.1` only, never `0.0.0.0`. The control plane answers `/auth/me` and
/// drives inference; exposing it on a LAN interface would hand both to anything
/// on the network that guesses the port.
///
/// # Port
///
/// From [`local_backend_port`](resolve::local_backend_port) — config with an env
/// override, `0` for an ephemeral port. The bound address is published through
/// [`set_local_backend_base_url`](resolve::set_local_backend_base_url) so
/// `effective_backend_api_url` resolves to it even in the ephemeral case.
pub async fn start(config: &Config) -> anyhow::Result<LocalBackendHandle> {
    let port = resolve::local_backend_port(config);
    let bind_target = format!("127.0.0.1:{port}");

    let listener = TcpListener::bind(&bind_target).await.map_err(|error| {
        anyhow::anyhow!(
            "failed to bind the local backend on {bind_target}: {error}. \
             Another process may already hold the port — set \
             `[local_mode] backend_port` (or OPENHUMAN_LOCAL_BACKEND_PORT) to a free one, \
             or 0 for any free port."
        )
    })?;
    let addr = listener.local_addr()?;
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let state = LocalBackendState::new(config.local_mode.proxy_inference);
    let session_token = state.session_token.to_string();
    let app = build_router(state);

    let shutdown = CancellationToken::new();
    let shutdown_signal = shutdown.clone();
    let join = tokio::spawn(async move {
        tracing::info!("{LOG_PREFIX} serving on {addr}");
        if let Err(error) = axum::serve(listener, app)
            .with_graceful_shutdown(async move { shutdown_signal.cancelled().await })
            .await
        {
            tracing::error!("{LOG_PREFIX} server exited with error: {error}");
        } else {
            tracing::info!("{LOG_PREFIX} server stopped cleanly");
        }
    });

    // Publish only after the listener is bound: a caller that resolves the
    // backend URL between publish and bind would get a connection refused, and
    // the whole point of publishing is that the address is reachable.
    resolve::set_local_backend_base_url(&base_url);

    Ok(LocalBackendHandle {
        addr,
        base_url,
        session_token,
        shutdown,
        join,
    })
}
