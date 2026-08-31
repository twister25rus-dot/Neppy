//! Tests for the JSON request envelope.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

async fn mount(server: &MockServer, verb: &str, route: &str, status: u16, body: Value) {
    Mock::given(method(verb))
        .and(path(route.to_owned()))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// Runs one operation against a mock provider and returns the parsed result.
async fn run(server: &MockServer, operation: Value) -> Value {
    let mut request = operation;
    request["provider"] = json!("vercel");
    request["credentials"] = json!({"api_key": "token"});
    request["base_url"] = json!(server.uri());

    let response = execute_json(&request.to_string()).await.unwrap();
    serde_json::from_str(&response).unwrap()
}

#[tokio::test]
async fn creates_a_site() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/v11/projects",
        200,
        json!({"id": "prj_1", "name": "shop"}),
    )
    .await;

    let result = run(
        &server,
        json!({"operation": "create_site", "spec": {"name": "shop"}}),
    )
    .await;

    assert_eq!(result["result"], "site");
    assert_eq!(result["value"]["id"], "prj_1");
}

#[tokio::test]
async fn finds_a_site_or_reports_that_there_is_none() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v9/projects/shop",
        200,
        json!({"id": "prj_1", "name": "shop"}),
    )
    .await;
    mount(&server, "GET", "/v9/projects/ghost", 404, json!({})).await;

    let found = run(&server, json!({"operation": "find_site", "site": "shop"})).await;
    assert_eq!(found["result"], "site");

    let missing = run(&server, json!({"operation": "find_site", "site": "ghost"})).await;
    assert_eq!(missing["result"], "no_site");
}

#[tokio::test]
async fn lists_sites_with_a_default_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v10/projects"))
        .and(wiremock::matchers::query_param("limit", "20"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"projects": [{"id": "prj_1", "name": "shop"}]})),
        )
        .mount(&server)
        .await;

    let result = run(&server, json!({"operation": "list_sites"})).await;

    assert_eq!(result["result"], "sites");
    assert_eq!(result["value"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn runs_a_whole_launch() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v9/projects/shop",
        200,
        json!({"id": "prj_1", "name": "shop"}),
    )
    .await;
    mount(&server, "POST", "/v2/files", 200, json!({})).await;
    mount(
        &server,
        "POST",
        "/v13/deployments",
        200,
        json!({"id": "dpl_1", "name": "shop", "url": "shop.vercel.app", "readyState": "QUEUED"}),
    )
    .await;

    let result = run(
        &server,
        json!({
            "operation": "launch",
            "plan": {
                "site": {"name": "shop"},
                "bundle": [{"path": "package.json", "contents": "e30="}],
            },
        }),
    )
    .await;

    assert_eq!(result["result"], "launch");
    assert_eq!(result["value"]["deployment"]["id"], "dpl_1");
    assert_eq!(result["value"]["created_site"], false);
}

#[tokio::test]
async fn deploys_a_bundle() {
    let server = MockServer::start().await;
    mount(&server, "POST", "/v2/files", 200, json!({})).await;
    mount(
        &server,
        "POST",
        "/v13/deployments",
        200,
        json!({"id": "dpl_1", "name": "shop", "readyState": "BUILDING"}),
    )
    .await;

    let result = run(
        &server,
        json!({
            "operation": "deploy",
            "request": {
                "site": "shop",
                "bundle": [{"path": "package.json", "contents": "e30="}],
            },
        }),
    )
    .await;

    assert_eq!(result["result"], "deployment");
    assert_eq!(result["value"]["status"], "building");
}

#[tokio::test]
async fn reads_and_lists_deployments() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v13/deployments/dpl_1",
        200,
        json!({"id": "dpl_1", "readyState": "READY"}),
    )
    .await;
    mount(
        &server,
        "GET",
        "/v7/deployments",
        200,
        json!({"deployments": [{"uid": "dpl_1", "state": "READY"}]}),
    )
    .await;

    let one = run(&server, json!({"operation": "deployment", "id": "dpl_1"})).await;
    assert_eq!(one["result"], "deployment");

    let many = run(
        &server,
        json!({"operation": "list_deployments", "site": "shop", "limit": 5}),
    )
    .await;
    assert_eq!(many["result"], "deployments");
    assert_eq!(many["value"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn sets_and_lists_environment_variables() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/v10/projects/shop/env",
        201,
        json!({"failed": []}),
    )
    .await;
    mount(
        &server,
        "GET",
        "/v10/projects/shop/env",
        200,
        json!({"envs": [{"id": "env_1", "key": "K", "target": ["production"]}]}),
    )
    .await;

    let set = run(
        &server,
        json!({
            "operation": "set_env",
            "site": "shop",
            "vars": [{"key": "K", "value": "V"}],
        }),
    )
    .await;
    assert_eq!(set["result"], "done");

    let listed = run(&server, json!({"operation": "list_env", "site": "shop"})).await;
    assert_eq!(listed["result"], "env");
    assert_eq!(listed["value"][0]["key"], "K");
}

#[tokio::test]
async fn provisions_and_attaches_a_database() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v1/integrations/configurations",
        200,
        json!([{"id": "icfg_db", "slug": "neon"}]),
    )
    .await;
    mount(
        &server,
        "GET",
        "/v1/integrations/configuration/icfg_db/products",
        200,
        json!({"products": [{"id": "iap_pg", "slug": "postgres", "primaryProtocol": "storage"}]}),
    )
    .await;
    mount(
        &server,
        "POST",
        "/v1/storage/stores/integration/direct",
        200,
        json!({"store": {
            "id": "store_1",
            "name": "shop-db",
            "status": "available",
            "secrets": [{"name": "DATABASE_URL", "length": 1}],
            "product": {"slug": "postgres", "integrationConfigurationId": "icfg_db"},
        }}),
    )
    .await;
    mount(
        &server,
        "GET",
        "/v9/projects/shop",
        200,
        json!({"id": "prj_1", "name": "shop"}),
    )
    .await;
    mount(
        &server,
        "POST",
        "/v1/integrations/installations/icfg_db/resources/store_1/connections",
        201,
        json!({}),
    )
    .await;

    let provisioned = run(
        &server,
        json!({"operation": "provision_database", "spec": {"name": "shop-db"}}),
    )
    .await;
    assert_eq!(provisioned["result"], "database");

    let attached = run(
        &server,
        json!({
            "operation": "attach_database",
            "site": "shop",
            "database": provisioned["value"],
        }),
    )
    .await;
    assert_eq!(attached["result"], "env_keys");
    assert_eq!(attached["value"][0], "DATABASE_URL");
}

#[tokio::test]
async fn promotes_a_deployment() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v9/projects/shop",
        200,
        json!({"id": "prj_1", "name": "shop"}),
    )
    .await;
    mount(
        &server,
        "POST",
        "/v10/projects/prj_1/promote/dpl_1",
        200,
        json!({}),
    )
    .await;

    let result = run(
        &server,
        json!({"operation": "promote", "site": "shop", "deployment": "dpl_1"}),
    )
    .await;

    assert_eq!(result["result"], "done");
}

#[tokio::test]
async fn adds_and_lists_domains() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/v10/projects/shop/domains",
        200,
        json!({"name": "shop.com", "verified": true}),
    )
    .await;
    mount(
        &server,
        "GET",
        "/v9/projects/shop/domains",
        200,
        json!({"domains": [{"name": "shop.com", "verified": true}]}),
    )
    .await;

    let added = run(
        &server,
        json!({"operation": "add_domain", "site": "shop", "domain": "shop.com"}),
    )
    .await;
    assert_eq!(added["result"], "domain");

    let listed = run(
        &server,
        json!({"operation": "list_domains", "site": "shop"}),
    )
    .await;
    assert_eq!(listed["result"], "domains");
}

#[tokio::test]
async fn reports_analytics() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v1/query/web-analytics/visits/count",
        200,
        json!({"data": {"visitors": 3, "pageviews": 9}}),
    )
    .await;

    let result = run(
        &server,
        json!({
            "operation": "analytics",
            "query": {"site": "shop", "since_ms": 1, "until_ms": 2},
        }),
    )
    .await;

    assert_eq!(result["result"], "analytics");
    assert_eq!(result["value"]["pageviews"], 9);
}

#[tokio::test]
async fn a_request_that_is_not_an_envelope_is_rejected_as_one() {
    let error = execute_json("{ not json }").await.unwrap_err();

    assert!(matches!(error, Error::Envelope { .. }), "{error:?}");
}

#[tokio::test]
async fn an_unknown_operation_is_rejected() {
    let error = execute_json(r#"{"operation":"delete_everything"}"#)
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Envelope { .. }), "{error:?}");
}

#[tokio::test]
async fn a_provider_error_reaches_the_caller_unchanged() {
    let server = MockServer::start().await;
    mount(&server, "GET", "/v10/projects", 401, json!({})).await;

    let request = json!({
        "operation": "list_sites",
        "credentials": {"api_key": "token"},
        "base_url": server.uri(),
    });
    let error = execute_json(&request.to_string()).await.unwrap_err();

    assert_eq!(
        error,
        Error::Unauthorized {
            provider: "vercel".to_owned()
        }
    );
}

#[tokio::test]
async fn a_request_without_a_credential_falls_back_to_the_environment() {
    // Whether the environment holds a key depends on the machine; both outcomes
    // are correct, and running the call is what exercises the fallback.
    let result =
        execute_json(r#"{"operation":"list_sites","base_url":"http://127.0.0.1:1"}"#).await;

    match result {
        Ok(_) => panic!("port 1 cannot have answered"),
        Err(error) => assert!(
            matches!(error, Error::MissingApiKey { .. } | Error::Transport { .. }),
            "{error:?}"
        ),
    }
}

#[test]
fn the_available_providers_are_listed() {
    assert_eq!(providers(), ["vercel"]);
}
