//! `TinyBus` module entrypoint and bus-facing interface.
//!
//! This adapter keeps the hosting implementation independent from `TinyBus` while
//! exposing it as an installable, dynamically loaded integration. It is
//! deliberately thin: the two methods are the two things a bus caller needs, and
//! both delegate straight into [`crate::rpc`].
//!
//! `Execute` takes one JSON request and returns one JSON result rather than
//! mirroring each of the fourteen operations as its own bus method. The
//! operations share a credential, a provider and an error vocabulary, and
//! fourteen signatures would have to be kept in step with the Rust API, the
//! JSON envelope, and the manifest at once. The JSON envelope is the contract
//! either way — the front end that calls this does not link against the crate.

use tinybus::{Connection, Result as TinyBusResult};

const INTERFACE: &str = "ai.tinyhumans.tinyhosts.Hosting";
const OBJECT_PATH: &str = "/ai/tinyhumans/tinyhosts/Hosting";

struct HostingService;

#[tinybus::interface(name = "ai.tinyhumans.tinyhosts.Hosting")]
impl HostingService {
    /// Runs one JSON hosting request and returns its JSON result.
    async fn execute(&self, request: String) -> TinyBusResult<String> {
        crate::rpc::execute_json(&request)
            .await
            .map_err(|error| tinybus::Error::failed(error.to_string()))
    }

    /// Lists the provider slugs this build can connect to.
    async fn providers(&self) -> TinyBusResult<String> {
        std::future::ready(serde_json::to_string(&crate::rpc::providers()))
            .await
            .map_err(|error| tinybus::Error::failed(error.to_string()))
    }
}

async fn setup(connection: Connection) -> TinyBusResult<()> {
    connection
        .serve_at(OBJECT_PATH.try_into()?, HostingService)
        .await?;
    connection.request_name(INTERFACE).await?;
    Ok(())
}

tinybus_module::module_export! {
    setup = setup,
    worker_threads = 2,
    provides = ["ai.tinyhumans.tinyhosts.Hosting"],
    methods = ["Execute", "Providers"],
    signals = [],
    requires = [],
    optional = [],
    lazy = false,
}

#[cfg(test)]
mod test;
