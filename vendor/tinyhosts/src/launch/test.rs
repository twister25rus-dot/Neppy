//! Tests for the launch flow.
//!
//! The flow is tested against a mock of the provider API rather than a fake
//! [`Host`], because the thing worth testing is the order the real calls go out
//! in — and a hand-written double would only ever confirm the order it was
//! written to expect.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::bundle::Bundle;
use crate::host::types::{DatabaseSpec, DeploymentStatus, DeploymentTarget, EnvVar, SiteSpec};
use crate::providers::vercel::Vercel;
use crate::{Credentials, Error};

fn bundle() -> Bundle {
    let mut bundle = Bundle::new();
    bundle.insert("package.json", b"{}".to_vec()).unwrap();
    bundle
}

fn host(server: &MockServer) -> Vercel {
    Vercel::with_base_url(Credentials::new("token").unwrap(), server.uri()).unwrap()
}

async fn mount(server: &MockServer, verb: &str, route: &str, status: u16, body: Value) {
    Mock::given(method(verb))
        .and(path(route.to_owned()))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

/// Every response a full launch needs, with the site absent to begin with.
async fn mount_launch(server: &MockServer) {
    // The site is absent for the first lookup and present afterwards, which is
    // what actually happens once the launch has created it.
    Mock::given(method("GET"))
        .and(path("/v9/projects/shop"))
        .respond_with(ResponseTemplate::new(404))
        .up_to_n_times(1)
        .mount(server)
        .await;
    mount(
        server,
        "GET",
        "/v9/projects/shop",
        200,
        json!({"id": "prj_1", "name": "shop"}),
    )
    .await;
    mount(
        server,
        "POST",
        "/v11/projects",
        200,
        json!({"id": "prj_1", "name": "shop", "framework": "nextjs"}),
    )
    .await;
    mount(
        server,
        "GET",
        "/v1/integrations/configurations",
        200,
        json!([{"id": "icfg_db", "slug": "neon"}]),
    )
    .await;
    mount(
        server,
        "GET",
        "/v1/integrations/configuration/icfg_db/products",
        200,
        json!({"products": [{"id": "iap_pg", "slug": "serverless", "primaryProtocol": "storage"}]}),
    )
    .await;
    mount(
        server,
        "POST",
        "/v1/storage/stores/integration/direct",
        200,
        json!({"store": {
            "id": "store_1",
            "name": "shop-db",
            "status": "available",
            "secrets": [{"name": "DATABASE_URL", "length": 60}],
            "product": {"slug": "serverless", "integrationConfigurationId": "icfg_db"},
        }}),
    )
    .await;
    mount(
        server,
        "POST",
        "/v1/integrations/installations/icfg_db/resources/store_1/connections",
        201,
        json!({}),
    )
    .await;
    mount(
        server,
        "POST",
        "/v10/projects/shop/env",
        201,
        json!({"failed": []}),
    )
    .await;
    mount(
        server,
        "POST",
        "/v10/projects/shop/domains",
        200,
        json!({"name": "shop.com", "verified": false}),
    )
    .await;
    mount(server, "POST", "/v2/files", 200, json!({})).await;
    mount(
        server,
        "POST",
        "/v13/deployments",
        200,
        json!({
            "id": "dpl_1",
            "name": "shop",
            "url": "shop.vercel.app",
            "readyState": "QUEUED",
            "target": "production",
        }),
    )
    .await;
}

#[tokio::test]
async fn a_launch_creates_provisions_configures_and_deploys_in_that_order() {
    let server = MockServer::start().await;
    mount_launch(&server).await;

    let plan = LaunchPlan::new(SiteSpec::new("shop"), bundle())
        .with_database(DatabaseSpec::new("shop-db"))
        .with_env(vec![EnvVar::new("NEXT_PUBLIC_NAME", "Shop")])
        .with_domains(vec!["shop.com".to_owned()])
        .into_production();

    let result = launch(&host(&server), &plan).await.unwrap();

    assert!(result.created_site);
    assert_eq!(result.site.id, "prj_1");
    assert_eq!(result.database.as_ref().unwrap().id, "store_1");
    assert_eq!(result.database_env_keys, ["DATABASE_URL"]);
    assert_eq!(result.domains[0].name, "shop.com");
    assert_eq!(result.deployment.status, DeploymentStatus::Queued);
    assert_eq!(result.url(), Some("https://shop.vercel.app"));

    let routes: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();

    assert_eq!(
        routes,
        [
            // The site first, created because it was not there.
            "/v9/projects/shop",
            "/v11/projects",
            // Then the database, provisioned and connected...
            "/v1/integrations/configurations",
            "/v1/integrations/configuration/icfg_db/products",
            "/v1/storage/stores/integration/direct",
            // Connecting the store to the project resolves the project's id.
            "/v9/projects/shop",
            "/v1/integrations/installations/icfg_db/resources/store_1/connections",
            // ...before the caller's own variables, which may override it.
            "/v10/projects/shop/env",
            // The domain is attached before the deployment goes live.
            "/v10/projects/shop/domains",
            // The build is last: it reads everything above at build time.
            "/v2/files",
            "/v13/deployments",
        ]
    );
}

#[tokio::test]
async fn an_existing_site_is_reused_rather_than_recreated() {
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
        json!({"id": "dpl_2", "name": "shop", "readyState": "BUILDING"}),
    )
    .await;

    let plan = LaunchPlan::new(SiteSpec::new("shop"), bundle());
    let result = launch(&host(&server), &plan).await.unwrap();

    assert!(!result.created_site);
    assert!(result.database.is_none());
    assert!(result.database_env_keys.is_empty());
    assert!(result.domains.is_empty());
    assert_eq!(result.url(), None);

    let routes: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();
    assert_eq!(
        routes,
        ["/v9/projects/shop", "/v2/files", "/v13/deployments"]
    );
}

#[tokio::test]
async fn a_failing_step_stops_the_launch_where_it_failed() {
    let server = MockServer::start().await;
    mount(&server, "GET", "/v9/projects/shop", 404, json!({})).await;
    mount(
        &server,
        "POST",
        "/v11/projects",
        409,
        json!({"error": {"code": "taken", "message": "name already in use"}}),
    )
    .await;

    let plan = LaunchPlan::new(SiteSpec::new("shop"), bundle());
    let error = launch(&host(&server), &plan).await.unwrap_err();

    assert!(matches!(error, Error::Api { status: 409, .. }), "{error:?}");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn an_invalid_plan_never_reaches_the_provider() {
    let server = MockServer::start().await;
    let plan = LaunchPlan::new(SiteSpec::new("shop"), Bundle::new());

    assert_eq!(
        launch(&host(&server), &plan).await.unwrap_err(),
        Error::EmptyBundle
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[test]
fn a_plan_defaults_to_a_preview_with_nothing_attached() {
    let plan = LaunchPlan::new(SiteSpec::new("shop"), bundle());

    assert_eq!(plan.target, DeploymentTarget::Preview);
    assert!(plan.database.is_none());
    assert!(plan.env.is_empty());
    assert!(plan.domains.is_empty());
    assert!(plan.validate().is_ok());
}

#[test]
fn a_plan_validates_everything_it_carries() {
    let base = LaunchPlan::new(SiteSpec::new("shop"), bundle());

    assert_eq!(
        LaunchPlan::new(SiteSpec::new(" "), bundle())
            .validate()
            .unwrap_err(),
        Error::EmptySiteName
    );
    assert_eq!(
        LaunchPlan::new(SiteSpec::new("shop"), Bundle::new())
            .validate()
            .unwrap_err(),
        Error::EmptyBundle
    );
    assert_eq!(
        base.clone()
            .with_database(DatabaseSpec::new(" "))
            .validate()
            .unwrap_err(),
        Error::EmptySiteName
    );
    assert_eq!(
        base.clone()
            .with_env(vec![EnvVar::new(" ", "x")])
            .validate()
            .unwrap_err(),
        Error::EmptyEnvKey
    );
    assert_eq!(
        base.with_domains(vec!["  ".to_owned()])
            .validate()
            .unwrap_err(),
        Error::EmptyDomain
    );
}

#[test]
fn a_plan_round_trips_through_json() {
    let plan = LaunchPlan::new(SiteSpec::new("shop"), bundle())
        .with_database(DatabaseSpec::new("shop-db"))
        .into_production();

    let json = serde_json::to_string(&plan).unwrap();

    assert_eq!(serde_json::from_str::<LaunchPlan>(&json).unwrap(), plan);
}
