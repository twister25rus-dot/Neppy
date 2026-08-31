//! Tests for the `TinyBus` module adapter and its declared surface.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Interface};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::{HostingService, INTERFACE, OBJECT_PATH, setup};

/// A client connected to a bus with the module already serving on it.
///
/// The service connection is returned alongside the client: dropping it drops
/// the name it claimed, and every call would then fail with `NameHasNoOwner`.
async fn connect() -> tinybus::Result<(Connection, Connection)> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let service = Connection::connect(bus.connect().await?).await?;
    setup(service.clone()).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    Ok((service, client))
}

#[test]
fn declared_methods_match_the_dispatch_table() {
    let methods = HostingService
        .members()
        .into_iter()
        .map(|member| member.to_string())
        .collect::<Vec<_>>();

    assert_eq!(methods, ["Execute", "Providers"]);
}

#[tokio::test]
async fn the_module_reports_the_providers_this_build_has() -> tinybus::Result<()> {
    let (_service, client) = connect().await?;
    let proxy = client.proxy(INTERFACE, OBJECT_PATH, INTERFACE)?;

    let providers: String = proxy.call("Providers", ()).await?;

    assert_eq!(providers, r#"["vercel"]"#);
    Ok(())
}

#[tokio::test]
async fn the_module_runs_a_hosting_request() -> tinybus::Result<()> {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v10/projects"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"projects": [{"id": "prj_1", "name": "shop"}]})),
        )
        .mount(&server)
        .await;

    let (_service, client) = connect().await?;
    let proxy = client.proxy(INTERFACE, OBJECT_PATH, INTERFACE)?;

    let request = json!({
        "operation": "list_sites",
        "credentials": {"api_key": "token"},
        "base_url": server.uri(),
    })
    .to_string();
    let response: String = proxy.call("Execute", (request,)).await?;

    assert!(response.contains(r#""result":"sites""#), "{response}");
    assert!(response.contains("prj_1"), "{response}");
    Ok(())
}

#[tokio::test]
async fn a_failed_request_becomes_a_bus_error_carrying_the_reason() -> tinybus::Result<()> {
    let (_service, client) = connect().await?;
    let proxy = client.proxy(INTERFACE, OBJECT_PATH, INTERFACE)?;

    let result = proxy
        .call::<String>("Execute", ("{ not an envelope }".to_owned(),))
        .await;

    let Err(error) = result else {
        return Err(tinybus::Error::failed(
            "a malformed envelope unexpectedly succeeded",
        ));
    };
    assert!(
        error
            .to_string()
            .contains("cannot decode the request envelope"),
        "{error}"
    );
    Ok(())
}
