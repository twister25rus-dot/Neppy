//! Tests for the Vercel adapter.
//!
//! Every test runs the real adapter — real request construction, real status
//! mapping, real response translation — against a local mock of Vercel's REST
//! API. Nothing here touches the network, and nothing here needs a token.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use wiremock::matchers::{body_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::bundle::Bundle;
use crate::host::types::{AnalyticsDimension, DatabaseKind, DeploymentStatus, Framework};

fn host(server: &MockServer) -> Vercel {
    Vercel::with_base_url(Credentials::new("token").unwrap(), server.uri()).unwrap()
}

fn bundle() -> Bundle {
    let mut bundle = Bundle::new();
    bundle.insert("package.json", b"{}".to_vec()).unwrap();
    bundle
}

/// Mounts one JSON response.
async fn mount(server: &MockServer, verb: &str, route: &str, status: u16, body: Value) {
    Mock::given(method(verb))
        .and(path(route.to_owned()))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn creates_a_next_js_project() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v11/projects"))
        .and(header("authorization", "Bearer token"))
        .and(body_json(json!({"name": "shop", "framework": "nextjs"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "prj_1",
            "name": "shop",
            "framework": "nextjs",
            "createdAt": 1_700_000_000_000_u64,
        })))
        .mount(&server)
        .await;

    let site = host(&server)
        .create_site(&SiteSpec::new("  shop  "))
        .await
        .unwrap();

    assert_eq!(site.id, "prj_1");
    assert_eq!(site.name, "shop");
    assert_eq!(site.framework, Some(Framework::NextJs));
    assert_eq!(site.created_at_ms, Some(1_700_000_000_000));
}

#[tokio::test]
async fn a_static_site_is_sent_without_a_framework() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v11/projects"))
        .and(body_json(json!({"name": "docs", "framework": Value::Null})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"id": "prj_2", "name": "docs", "framework": Value::Null})),
        )
        .mount(&server)
        .await;

    let site = host(&server)
        .create_site(&SiteSpec::new("docs").with_framework(Framework::Static))
        .await
        .unwrap();

    assert_eq!(site.framework, None);
}

#[tokio::test]
async fn an_unknown_framework_is_reported_as_the_provider_named_it() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v9/projects/docs",
        200,
        json!({"id": "prj_3", "name": "docs", "framework": "astro"}),
    )
    .await;

    let site = host(&server).find_site("docs").await.unwrap().unwrap();

    assert_eq!(site.framework, Some(Framework::Other("astro".to_owned())));
}

#[tokio::test]
async fn a_blank_site_name_never_reaches_the_provider() {
    let server = MockServer::start().await;

    let error = host(&server)
        .create_site(&SiteSpec::new(" "))
        .await
        .unwrap_err();

    assert_eq!(error, Error::EmptySiteName);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_missing_site_is_not_an_error() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v9/projects/absent",
        404,
        json!({"error": {"code": "not_found", "message": "not found"}}),
    )
    .await;

    assert!(host(&server).find_site("absent").await.unwrap().is_none());
}

#[tokio::test]
async fn a_path_shaped_site_name_cannot_redirect_the_request() {
    let server = MockServer::start().await;
    // If the raw name reached the URL unescaped, this would request
    // `/v9/projects/other/domains` instead — a route this test never mounts.
    mount(
        &server,
        "GET",
        "/v9/projects/..%2Fother%2Fdomains",
        200,
        json!({"id": "prj_4", "name": "../other/domains"}),
    )
    .await;

    let site = host(&server)
        .find_site("../other/domains")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(site.id, "prj_4");
}

#[tokio::test]
async fn lists_projects_up_to_a_limit() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v10/projects"))
        .and(query_param("limit", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "projects": [
                {"id": "prj_1", "name": "shop"},
                {"id": "prj_2", "name": "docs"},
            ]
        })))
        .mount(&server)
        .await;

    let sites = host(&server).list_sites(2).await.unwrap();

    assert_eq!(sites.len(), 2);
    assert_eq!(sites[1].name, "docs");
}

#[tokio::test]
async fn an_empty_project_list_decodes() {
    let server = MockServer::start().await;
    mount(&server, "GET", "/v10/projects", 200, json!({})).await;

    assert!(host(&server).list_sites(5).await.unwrap().is_empty());
}

#[tokio::test]
async fn sets_environment_variables_with_an_upsert() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v10/projects/shop/env"))
        .and(query_param("upsert", "true"))
        .and(body_json(json!([
            {
                "key": "DATABASE_URL",
                "value": "postgres://",
                "type": "encrypted",
                "target": ["production", "preview", "development"],
            },
            {
                "key": "STRIPE_KEY",
                "value": "sk_live",
                "type": "sensitive",
                "target": ["production"],
            },
        ])))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"failed": []})))
        .mount(&server)
        .await;

    host(&server)
        .set_env(
            "shop",
            &[
                EnvVar::new(" DATABASE_URL ", "postgres://"),
                EnvVar::new("STRIPE_KEY", "sk_live")
                    .with_targets(vec![DeploymentTarget::Production])
                    .secret(),
            ],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn setting_no_variables_sends_no_request() {
    let server = MockServer::start().await;

    host(&server).set_env("shop", &[]).await.unwrap();

    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_blank_variable_name_never_reaches_the_provider() {
    let server = MockServer::start().await;

    let error = host(&server)
        .set_env("shop", &[EnvVar::new(" ", "x")])
        .await
        .unwrap_err();

    assert_eq!(error, Error::EmptyEnvKey);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn lists_environment_variables_without_their_values() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v10/projects/shop/env",
        200,
        json!({"envs": [
            {"id": "env_1", "key": "DATABASE_URL", "target": ["production", "development"], "type": "encrypted"},
            {"id": "env_2", "key": "STRIPE_KEY", "target": "production", "type": "sensitive"},
            {"key": "LEGACY", "value": "leaked?"},
        ]}),
    )
    .await;

    let vars = host(&server).list_env("shop").await.unwrap();

    assert_eq!(vars[0].key, "DATABASE_URL");
    // `development` has no unified equivalent and is dropped.
    assert_eq!(vars[0].targets, vec![DeploymentTarget::Production]);
    assert!(!vars[0].secret);
    assert_eq!(vars[1].targets, vec![DeploymentTarget::Production]);
    assert!(vars[1].secret);
    assert_eq!(vars[2].id, "");
    assert!(vars[2].targets.is_empty());

    let rendered = serde_json::to_string(&vars).unwrap();
    assert!(!rendered.contains("leaked?"), "{rendered}");
}

#[tokio::test]
async fn uploads_every_file_then_creates_the_deployment() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v2/files"))
        .and(header_exists("x-vercel-digest"))
        .and(header("content-type", "application/octet-stream"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v13/deployments"))
        .and(query_param("forceNew", "1"))
        .and(query_param("skipAutoDetectionConfirmation", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "dpl_1",
            "name": "shop",
            "url": "shop-abc.vercel.app",
            "readyState": "BUILDING",
            "createdAt": 5_u64,
        })))
        .mount(&server)
        .await;

    let mut bundle = bundle();
    bundle.insert("app/page.tsx", b"page".to_vec()).unwrap();

    let deployment = host(&server)
        .deploy(&DeployRequest::new("shop", bundle))
        .await
        .unwrap();

    assert_eq!(deployment.id, "dpl_1");
    assert_eq!(deployment.site, "shop");
    // A bare host is returned as a URL a reader can open.
    assert_eq!(
        deployment.url.as_deref(),
        Some("https://shop-abc.vercel.app")
    );
    assert_eq!(deployment.status, DeploymentStatus::Building);
    assert_eq!(deployment.target, DeploymentTarget::Preview);
    assert_eq!(deployment.created_at_ms, Some(5));

    let requests = server.received_requests().await.unwrap();
    let created: Value = serde_json::from_slice(&requests[2].body).unwrap();
    assert_eq!(created["name"], "shop");
    assert_eq!(created["projectSettings"]["framework"], "nextjs");
    // A preview omits the target rather than naming it.
    assert!(created.get("target").is_none(), "{created}");
    let files = created["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["file"], "package.json");
    assert_eq!(files[0]["size"], 2);
    assert_eq!(files[0]["sha"].as_str().unwrap().len(), 40);
}

#[tokio::test]
async fn a_production_deployment_names_its_target() {
    let server = MockServer::start().await;
    mount(&server, "POST", "/v2/files", 200, json!({})).await;
    mount(
        &server,
        "POST",
        "/v13/deployments",
        200,
        json!({"id": "dpl_2", "url": "https://shop.com", "readyState": "READY", "target": "production"}),
    )
    .await;

    let deployment = host(&server)
        .deploy(&DeployRequest::new("shop", bundle()).with_target(DeploymentTarget::Production))
        .await
        .unwrap();

    assert_eq!(deployment.target, DeploymentTarget::Production);
    assert!(deployment.status.is_ready());
    // An absolute URL is left alone.
    assert_eq!(deployment.url.as_deref(), Some("https://shop.com"));

    let requests = server.received_requests().await.unwrap();
    let created: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert_eq!(created["target"], "production");
}

#[tokio::test]
async fn an_empty_bundle_never_reaches_the_provider() {
    let server = MockServer::start().await;

    let error = host(&server)
        .deploy(&DeployRequest::new("shop", Bundle::new()))
        .await
        .unwrap_err();

    assert_eq!(error, Error::EmptyBundle);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_failed_upload_stops_the_deployment() {
    let server = MockServer::start().await;
    mount(
        &server,
        "POST",
        "/v2/files",
        500,
        json!({"error": {"code": "oops", "message": "disk on fire"}}),
    )
    .await;

    let error = host(&server)
        .deploy(&DeployRequest::new("shop", bundle()))
        .await
        .unwrap_err();

    assert_eq!(
        error,
        Error::Api {
            provider: "vercel".to_owned(),
            status: 500,
            resource: "deployment file".to_owned(),
            message: "disk on fire".to_owned(),
        }
    );
}

#[tokio::test]
async fn reads_a_deployment_and_its_failure_message() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v13/deployments/dpl_9",
        200,
        json!({
            "id": "dpl_9",
            "name": "shop",
            "readyState": "ERROR",
            "errorMessage": "build failed",
        }),
    )
    .await;

    let deployment = host(&server).deployment("dpl_9").await.unwrap();

    assert_eq!(deployment.status, DeploymentStatus::Failed);
    assert!(deployment.status.is_terminal());
    assert_eq!(deployment.error_message.as_deref(), Some("build failed"));
    assert_eq!(deployment.url, None);
}

#[tokio::test]
async fn maps_every_provider_state() {
    let cases = [
        (json!("QUEUED"), DeploymentStatus::Queued),
        (json!("INITIALIZING"), DeploymentStatus::Building),
        (json!("BUILDING"), DeploymentStatus::Building),
        (json!("READY"), DeploymentStatus::Ready),
        (json!("ERROR"), DeploymentStatus::Failed),
        (json!("CANCELED"), DeploymentStatus::Canceled),
        (
            json!("BLOCKED"),
            DeploymentStatus::Other("BLOCKED".to_owned()),
        ),
        (Value::Null, DeploymentStatus::Queued),
    ];

    for (state, expected) in cases {
        let server = MockServer::start().await;
        mount(
            &server,
            "GET",
            "/v13/deployments/dpl_1",
            200,
            json!({"id": "dpl_1", "readyState": state}),
        )
        .await;

        let deployment = host(&server).deployment("dpl_1").await.unwrap();
        assert_eq!(deployment.status, expected, "state {state:?}");
        // A response with no name falls back to what the caller asked about.
        assert_eq!(deployment.site, "");
    }
}

#[tokio::test]
async fn lists_deployments_under_their_list_field_names() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v7/deployments"))
        .and(query_param("projectId", "shop"))
        .and(query_param("limit", "3"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "deployments": [
                {"uid": "dpl_2", "name": "shop", "state": "READY", "created": 2_u64, "url": "b.vercel.app"},
                {"uid": "dpl_1", "name": "shop", "state": "ERROR", "created": 1_u64, "url": "a.vercel.app"},
            ]
        })))
        .mount(&server)
        .await;

    let deployments = host(&server).list_deployments("shop", 3).await.unwrap();

    assert_eq!(deployments[0].id, "dpl_2");
    assert_eq!(deployments[0].status, DeploymentStatus::Ready);
    assert_eq!(deployments[0].created_at_ms, Some(2));
    assert_eq!(deployments[1].status, DeploymentStatus::Failed);
}

#[tokio::test]
async fn an_empty_deployment_list_decodes() {
    let server = MockServer::start().await;
    mount(&server, "GET", "/v7/deployments", 200, json!({})).await;

    assert!(
        host(&server)
            .list_deployments("shop", 1)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn promoting_resolves_the_project_first() {
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

    host(&server).promote("shop", "dpl_1").await.unwrap();
}

#[tokio::test]
async fn promoting_an_unknown_site_says_which_project_is_missing() {
    let server = MockServer::start().await;
    mount(&server, "GET", "/v9/projects/ghost", 404, json!({})).await;

    let error = host(&server).promote("ghost", "dpl_1").await.unwrap_err();

    assert_eq!(
        error,
        Error::NotFound {
            provider: "vercel".to_owned(),
            resource: "project ghost".to_owned(),
        }
    );
}

#[tokio::test]
async fn adds_a_domain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v10/projects/shop/domains"))
        .and(body_json(json!({"name": "shop.com"})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"name": "shop.com", "verified": false})),
        )
        .mount(&server)
        .await;

    let domain = host(&server)
        .add_domain("shop", " shop.com ")
        .await
        .unwrap();

    assert_eq!(domain.name, "shop.com");
    assert_eq!(domain.site, "shop");
    assert!(!domain.verified);
}

#[tokio::test]
async fn a_blank_domain_never_reaches_the_provider() {
    let server = MockServer::start().await;

    let error = host(&server).add_domain("shop", "  ").await.unwrap_err();

    assert_eq!(error, Error::EmptyDomain);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn lists_domains() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v9/projects/shop/domains",
        200,
        json!({"domains": [{"name": "shop.com", "verified": true}, {"name": "www.shop.com"}]}),
    )
    .await;

    let domains = host(&server).list_domains("shop").await.unwrap();

    assert!(domains[0].verified);
    assert!(!domains[1].verified);
    assert_eq!(domains[1].site, "shop");
}

/// The three responses a database provisioning walks through.
async fn mount_marketplace(server: &MockServer, product: Value, store: Value) {
    Mock::given(method("GET"))
        .and(path("/v1/integrations/configurations"))
        .and(query_param("view", "account"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "icfg_logs", "slug": "logtail"},
            {"id": "icfg_db", "slug": "neon"},
        ])))
        .mount(server)
        .await;
    mount(
        server,
        "GET",
        "/v1/integrations/configuration/icfg_logs/products",
        200,
        json!({"products": [
            {"id": "iap_logs", "slug": "log-drain", "name": "Log storage", "primaryProtocol": "logDrain"}
        ]}),
    )
    .await;
    mount(
        server,
        "GET",
        "/v1/integrations/configuration/icfg_db/products",
        200,
        json!({"products": [product]}),
    )
    .await;
    mount(
        server,
        "POST",
        "/v1/storage/stores/integration/direct",
        200,
        store,
    )
    .await;
}

#[tokio::test]
async fn provisions_a_postgres_from_an_installed_integration() {
    let server = MockServer::start().await;
    mount_marketplace(
        &server,
        json!({
            "id": "iap_serverless",
            "slug": "serverless-db",
            "name": "Serverless DB",
            "primaryProtocol": "storage",
        }),
        json!({"store": {
            "id": "store_1",
            "name": "shop-db",
            "status": "available",
            "secrets": [{"name": "DATABASE_URL", "length": 60}, {"name": "PGHOST", "length": 20}],
            "product": {
                "slug": "serverless-db",
                "name": "Serverless DB",
                "integrationConfigurationId": "icfg_db",
            },
        }}),
    )
    .await;

    let database = host(&server)
        .provision_database(&DatabaseSpec::new(" shop-db "))
        .await
        .unwrap();

    assert_eq!(database.id, "store_1");
    assert_eq!(database.name, "shop-db");
    assert_eq!(database.kind, DatabaseKind::Postgres);
    assert_eq!(database.product.as_deref(), Some("serverless-db"));
    assert_eq!(database.status, "available");
    assert_eq!(database.secret_keys, ["DATABASE_URL", "PGHOST"]);
    assert_eq!(database.installation_id.as_deref(), Some("icfg_db"));

    // The installation's own slug is what identified this product as a Postgres:
    // neither the product slug nor its name contains the protocol.
    let requests = server.received_requests().await.unwrap();
    let created: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    assert_eq!(created["integrationConfigurationId"], "icfg_db");
    assert_eq!(created["integrationProductIdOrSlug"], "iap_serverless");
    assert_eq!(created["name"], "shop-db");
}

#[tokio::test]
async fn a_pinned_product_overrules_the_kind() {
    let server = MockServer::start().await;
    mount_marketplace(
        &server,
        json!({"id": "iap_odd", "slug": "mystery-store", "name": "Mystery"}),
        json!({"store": {"id": "store_2", "name": "cache", "status": "initializing"}}),
    )
    .await;

    let database = host(&server)
        .provision_database(
            &DatabaseSpec::new("cache")
                .with_kind(DatabaseKind::Redis)
                .with_product("mystery-store"),
        )
        .await
        .unwrap();

    assert_eq!(database.status, "initializing");
    assert_eq!(database.kind, DatabaseKind::Redis);
    assert_eq!(database.product, None);
    // No product named an installation, so the one it was created through is
    // recorded instead — attaching it later needs that identifier.
    assert_eq!(database.installation_id.as_deref(), Some("icfg_db"));
}

#[tokio::test]
async fn a_store_identified_only_by_the_partner_still_has_an_id() {
    let server = MockServer::start().await;
    mount_marketplace(
        &server,
        json!({"id": "iap_pg", "slug": "postgres", "primaryProtocol": "storage"}),
        json!({"store": {"externalResourceId": "ext_1", "status": "available"}}),
    )
    .await;

    let database = host(&server)
        .provision_database(&DatabaseSpec::new("shop-db"))
        .await
        .unwrap();

    assert_eq!(database.id, "ext_1");
    assert_eq!(database.name, "shop-db");
}

#[tokio::test]
async fn no_matching_product_names_the_kind_that_could_not_be_served() {
    let server = MockServer::start().await;
    mount_marketplace(
        &server,
        json!({"id": "iap_x", "slug": "sentry", "name": "Errors", "primaryProtocol": "observability"}),
        json!({}),
    )
    .await;

    let error = host(&server)
        .provision_database(&DatabaseSpec::new("shop-db"))
        .await
        .unwrap_err();

    assert_eq!(
        error,
        Error::NoDatabaseProduct {
            provider: "vercel".to_owned(),
            kind: "postgres".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_pinned_product_that_is_not_installed_is_reported_by_name() {
    let server = MockServer::start().await;
    mount_marketplace(
        &server,
        json!({"id": "iap_pg", "slug": "postgres", "primaryProtocol": "storage"}),
        json!({}),
    )
    .await;

    let error = host(&server)
        .provision_database(&DatabaseSpec::new("shop-db").with_product("planetscale"))
        .await
        .unwrap_err();

    assert_eq!(
        error,
        Error::NoDatabaseProduct {
            provider: "vercel".to_owned(),
            kind: "planetscale".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_database_that_came_up_in_error_is_not_returned_as_working() {
    let server = MockServer::start().await;
    mount_marketplace(
        &server,
        json!({"id": "iap_pg", "slug": "postgres", "primaryProtocol": "storage"}),
        json!({"store": {"id": "store_3", "name": "shop-db", "status": "error"}}),
    )
    .await;

    let error = host(&server)
        .provision_database(&DatabaseSpec::new("shop-db"))
        .await
        .unwrap_err();

    assert_eq!(
        error,
        Error::DatabaseNotReady {
            name: "shop-db".to_owned(),
            status: "error".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_response_with_no_store_is_a_decoding_failure() {
    let server = MockServer::start().await;
    mount_marketplace(
        &server,
        json!({"id": "iap_pg", "slug": "postgres", "primaryProtocol": "storage"}),
        json!({"store": Value::Null}),
    )
    .await;

    let error = host(&server)
        .provision_database(&DatabaseSpec::new("shop-db"))
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Decode { .. }), "{error:?}");
}

#[tokio::test]
async fn a_blank_database_name_never_reaches_the_provider() {
    let server = MockServer::start().await;

    let error = host(&server)
        .provision_database(&DatabaseSpec::new(" "))
        .await
        .unwrap_err();

    assert_eq!(error, Error::EmptySiteName);
    assert!(server.received_requests().await.unwrap().is_empty());
}

fn database() -> Database {
    Database {
        id: "store_1".to_owned(),
        name: "shop-db".to_owned(),
        kind: DatabaseKind::Postgres,
        product: Some("serverless-db".to_owned()),
        status: "available".to_owned(),
        secret_keys: vec!["DATABASE_URL".to_owned()],
        installation_id: Some("icfg_db".to_owned()),
    }
}

#[tokio::test]
async fn attaching_a_database_returns_the_variables_the_site_receives() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v9/projects/shop",
        200,
        json!({"id": "prj_1", "name": "shop"}),
    )
    .await;
    Mock::given(method("POST"))
        .and(path(
            "/v1/integrations/installations/icfg_db/resources/store_1/connections",
        ))
        .and(body_json(json!({
            "projectId": "prj_1",
            "envVarEnvironments": ["production", "preview", "development"],
        })))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let keys = host(&server)
        .attach_database(&database(), "shop")
        .await
        .unwrap();

    assert_eq!(keys, ["DATABASE_URL"]);
}

#[tokio::test]
async fn a_database_with_no_installation_cannot_be_attached() {
    let server = MockServer::start().await;
    let mut database = database();
    database.installation_id = None;

    let error = host(&server)
        .attach_database(&database, "shop")
        .await
        .unwrap_err();

    assert_eq!(
        error,
        Error::NotFound {
            provider: "vercel".to_owned(),
            resource: "marketplace installation for database shop-db".to_owned(),
        }
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn reports_traffic_totals() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/query/web-analytics/visits/count"))
        .and(query_param("projectId", "shop"))
        .and(query_param("since", "1000"))
        .and(query_param("until", "2000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "version": 1,
            "data": {"visitors": 12, "pageviews": 40},
        })))
        .mount(&server)
        .await;

    let summary = host(&server)
        .analytics(&AnalyticsQuery::new("shop", 1000, 2000))
        .await
        .unwrap();

    assert_eq!(summary.site, "shop");
    assert_eq!(summary.visitors, Some(12));
    assert_eq!(summary.pageviews, Some(40));
    assert_eq!(summary.since_ms, 1000);
    assert_eq!(summary.until_ms, 2000);
    assert!(summary.breakdown.is_empty());
}

#[tokio::test]
async fn reports_a_breakdown_with_whatever_the_provider_counted() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v1/query/web-analytics/visits/count",
        200,
        json!({"data": {}}),
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/v1/query/web-analytics/visits/aggregate"))
        .and(query_param("by", "country"))
        .and(query_param("limit", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {"country": "NL", "pageviews": 30, "visitors": 9},
                {"country": "DE", "pageviews": 10.5},
                "not a row",
            ]
        })))
        .mount(&server)
        .await;

    let summary = host(&server)
        .analytics(
            &AnalyticsQuery::new("shop", 1000, 2000)
                .with_breakdown(AnalyticsDimension::Country)
                .with_limit(2),
        )
        .await
        .unwrap();

    assert_eq!(summary.visitors, None);
    assert_eq!(summary.breakdown.len(), 2);
    assert_eq!(summary.breakdown[0].label, "NL");
    assert!((summary.breakdown[0].metrics["pageviews"] - 30.0).abs() < f64::EPSILON);
    assert!((summary.breakdown[0].metrics["visitors"] - 9.0).abs() < f64::EPSILON);
    assert!((summary.breakdown[1].metrics["pageviews"] - 10.5).abs() < f64::EPSILON);
    assert!(!summary.breakdown[1].metrics.contains_key("visitors"));
}

#[tokio::test]
async fn a_breakdown_that_is_not_a_list_is_no_breakdown() {
    let server = MockServer::start().await;
    mount(
        &server,
        "GET",
        "/v1/query/web-analytics/visits/count",
        200,
        json!({"data": {"visitors": 1, "pageviews": 1}}),
    )
    .await;
    mount(
        &server,
        "GET",
        "/v1/query/web-analytics/visits/aggregate",
        200,
        json!({"data": {"unexpected": true}}),
    )
    .await;

    let summary = host(&server)
        .analytics(
            &AnalyticsQuery::new("shop", 1, 2).with_breakdown(AnalyticsDimension::RequestPath),
        )
        .await
        .unwrap();

    assert!(summary.breakdown.is_empty());
}

#[tokio::test]
async fn an_impossible_window_never_reaches_the_provider() {
    let server = MockServer::start().await;

    let error = host(&server)
        .analytics(&AnalyticsQuery::new("shop", 2, 1))
        .await
        .unwrap_err();

    assert_eq!(error, Error::InvalidAnalyticsWindow);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn every_failed_status_maps_to_its_own_error() {
    let cases = [
        (
            401,
            Error::Unauthorized {
                provider: "vercel".to_owned(),
            },
        ),
        (
            403,
            Error::Forbidden {
                provider: "vercel".to_owned(),
                resource: "project list".to_owned(),
            },
        ),
        (
            404,
            Error::NotFound {
                provider: "vercel".to_owned(),
                resource: "project list".to_owned(),
            },
        ),
        (
            429,
            Error::RateLimited {
                provider: "vercel".to_owned(),
            },
        ),
    ];

    for (status, expected) in cases {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v10/projects"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;

        assert_eq!(host(&server).list_sites(1).await.unwrap_err(), expected);
    }
}

#[tokio::test]
async fn a_failure_with_no_body_falls_back_to_the_status_text() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v10/projects"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let error = host(&server).list_sites(1).await.unwrap_err();

    assert_eq!(
        error,
        Error::Api {
            provider: "vercel".to_owned(),
            status: 503,
            resource: "project list".to_owned(),
            message: "Service Unavailable".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_failure_that_is_not_json_is_reported_verbatim() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v10/projects"))
        .respond_with(ResponseTemplate::new(502).set_body_string("<html>bad gateway</html>"))
        .mount(&server)
        .await;

    let error = host(&server).list_sites(1).await.unwrap_err();

    assert_eq!(
        error,
        Error::Api {
            provider: "vercel".to_owned(),
            status: 502,
            resource: "project list".to_owned(),
            message: "<html>bad gateway</html>".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_body_of_the_wrong_shape_is_a_decoding_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v10/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&server)
        .await;

    let error = host(&server).list_sites(1).await.unwrap_err();

    assert!(matches!(error, Error::Decode { .. }), "{error:?}");
}

#[tokio::test]
async fn a_request_that_never_arrives_is_a_transport_failure() {
    // Port 1 on the loopback interface refuses connections.
    let host =
        Vercel::with_base_url(Credentials::new("token").unwrap(), "http://127.0.0.1:1").unwrap();

    let error = host.list_sites(1).await.unwrap_err();

    assert!(matches!(error, Error::Transport { .. }), "{error:?}");
}

#[tokio::test]
async fn a_team_scope_is_applied_to_every_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v10/projects"))
        .and(query_param("teamId", "team_abc"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"projects": []})))
        .mount(&server)
        .await;

    let host = Vercel::with_base_url(
        Credentials::new("token").unwrap().with_team("team_abc"),
        // A trailing slash on the root must not become a double slash in a path.
        format!("{}/", server.uri()),
    )
    .unwrap();

    assert!(host.list_sites(1).await.unwrap().is_empty());
}

#[test]
fn the_client_reports_itself_without_its_token() {
    let host = Vercel::with_base_url(
        Credentials::new("super-secret").unwrap(),
        "http://127.0.0.1:1",
    )
    .unwrap();
    let rendered = format!("{host:?}");

    assert!(!rendered.contains("super-secret"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
    assert_eq!(host.kind(), ProviderKind::Vercel);
}

#[test]
fn with_base_url_rejects_plain_http_against_a_non_loopback_host() {
    let error = Vercel::with_base_url(Credentials::new("token").unwrap(), "http://x").unwrap_err();

    assert_eq!(
        error,
        Error::InsecureBaseUrl {
            base_url: "http://x".to_owned()
        }
    );
}

#[test]
fn the_digest_is_sha_1() {
    // The published SHA-1 of "abc". Vercel's `x-vercel-digest` defines the
    // algorithm, so this is a contract, not an implementation detail.
    assert_eq!(digest(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
}

#[test]
fn an_empty_target_list_means_every_environment() {
    assert_eq!(env_targets(&[]), ["production", "preview", "development"]);
    assert_eq!(env_targets(&[DeploymentTarget::Preview]), ["preview"]);
    assert_eq!(
        env_targets(&[DeploymentTarget::Production, DeploymentTarget::Production]),
        ["production"]
    );
    // Non-adjacent duplicates: a plain `Vec::dedup` would miss the repeated
    // `Production` here because `Preview` sits between the two occurrences.
    assert_eq!(
        env_targets(&[
            DeploymentTarget::Production,
            DeploymentTarget::Preview,
            DeploymentTarget::Production,
        ]),
        ["production", "preview"]
    );
}
