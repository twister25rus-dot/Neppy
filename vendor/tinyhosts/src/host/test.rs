//! Unit tests for the unified model's vocabulary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::Error;
use crate::bundle::Bundle;
use crate::host::types::{
    AnalyticsDimension, AnalyticsQuery, DatabaseKind, DatabaseSpec, DeployRequest, Deployment,
    DeploymentStatus, DeploymentTarget, EnvVar, Framework, SiteSpec,
};

fn bundle() -> Bundle {
    let mut bundle = Bundle::new();
    bundle.insert("package.json", b"{}".to_vec()).unwrap();
    bundle
}

#[test]
fn a_site_defaults_to_next_js() {
    let spec = SiteSpec::new("shop");

    assert_eq!(spec.framework, Framework::NextJs);
    assert_eq!(spec.framework.as_str(), "nextjs");
    assert!(spec.validate().is_ok());
}

#[test]
fn a_site_can_be_something_else() {
    let spec = SiteSpec::new("docs").with_framework(Framework::Other("astro".to_owned()));

    assert_eq!(spec.framework.as_str(), "astro");
    assert_eq!(Framework::Static.as_str(), "static");
    assert_eq!(Framework::default(), Framework::NextJs);
}

#[test]
fn a_blank_site_name_is_rejected() {
    assert_eq!(
        SiteSpec::new("   ").validate().unwrap_err(),
        Error::EmptySiteName
    );
}

#[test]
fn a_target_names_itself() {
    assert_eq!(DeploymentTarget::Preview.as_str(), "preview");
    assert_eq!(DeploymentTarget::Production.as_str(), "production");
    assert_eq!(DeploymentTarget::default(), DeploymentTarget::Preview);
}

#[test]
fn a_deploy_request_defaults_to_a_next_js_preview() {
    let request = DeployRequest::new("shop", bundle());

    assert_eq!(request.target, DeploymentTarget::Preview);
    assert_eq!(request.framework, Framework::NextJs);
    assert!(request.validate().is_ok());
}

#[test]
fn a_deploy_request_can_be_retargeted() {
    let request = DeployRequest::new("shop", bundle())
        .with_target(DeploymentTarget::Production)
        .with_framework(Framework::Static);

    assert_eq!(request.target, DeploymentTarget::Production);
    assert_eq!(request.framework, Framework::Static);
}

#[test]
fn a_deploy_request_needs_a_site_and_files() {
    assert_eq!(
        DeployRequest::new(" ", bundle()).validate().unwrap_err(),
        Error::EmptySiteName
    );
    assert_eq!(
        DeployRequest::new("shop", Bundle::new())
            .validate()
            .unwrap_err(),
        Error::EmptyBundle
    );
}

#[test]
fn only_a_settled_status_is_terminal() {
    for status in [
        DeploymentStatus::Ready,
        DeploymentStatus::Failed,
        DeploymentStatus::Canceled,
    ] {
        assert!(status.is_terminal(), "{status:?}");
    }

    for status in [
        DeploymentStatus::Queued,
        DeploymentStatus::Building,
        DeploymentStatus::Other("BLOCKED".to_owned()),
    ] {
        assert!(!status.is_terminal(), "{status:?}");
    }

    assert!(DeploymentStatus::Ready.is_ready());
    assert!(!DeploymentStatus::Building.is_ready());
}

#[test]
fn an_env_var_applies_everywhere_by_default() {
    let var = EnvVar::new("DATABASE_URL", "postgres://");

    assert!(var.targets.is_empty());
    assert!(!var.secret);
    assert!(var.validate().is_ok());
}

#[test]
fn an_env_var_can_be_scoped_and_hidden() {
    let var = EnvVar::new("STRIPE_KEY", "sk_live")
        .with_targets(vec![DeploymentTarget::Production])
        .secret();

    assert_eq!(var.targets, vec![DeploymentTarget::Production]);
    assert!(var.secret);
}

#[test]
fn a_blank_env_key_is_rejected() {
    assert_eq!(
        EnvVar::new("  ", "value").validate().unwrap_err(),
        Error::EmptyEnvKey
    );
}

#[test]
fn debug_prints_the_key_and_never_the_value() {
    let var = EnvVar::new("STRIPE_KEY", "sk_live_hunter2");
    let rendered = format!("{var:?}");

    assert!(rendered.contains("STRIPE_KEY"), "{rendered}");
    assert!(!rendered.contains("sk_live_hunter2"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}

#[test]
fn a_database_defaults_to_postgres() {
    let spec = DatabaseSpec::new("shop-db");

    assert_eq!(spec.kind, DatabaseKind::Postgres);
    assert_eq!(spec.product, None);
    assert!(spec.validate().is_ok());
}

#[test]
fn a_database_can_be_pinned_to_a_product() {
    let spec = DatabaseSpec::new("cache")
        .with_kind(DatabaseKind::Redis)
        .with_product("upstash-kv");

    assert_eq!(spec.kind, DatabaseKind::Redis);
    assert_eq!(spec.product.as_deref(), Some("upstash-kv"));
}

#[test]
fn a_blank_database_name_is_rejected() {
    assert_eq!(
        DatabaseSpec::new(" ").validate().unwrap_err(),
        Error::EmptySiteName
    );
}

#[test]
fn a_database_kind_knows_the_names_vendors_use() {
    assert_eq!(DatabaseKind::Postgres.as_str(), "postgres");
    assert_eq!(DatabaseKind::Redis.as_str(), "redis");
    assert_eq!(DatabaseKind::Blob.as_str(), "blob");
    assert_eq!(DatabaseKind::Other("mongo".to_owned()).as_str(), "mongo");

    assert!(DatabaseKind::Postgres.product_hints().contains(&"neon"));
    assert!(DatabaseKind::Redis.product_hints().contains(&"upstash"));
    assert!(DatabaseKind::Blob.product_hints().contains(&"blob"));
    assert_eq!(
        DatabaseKind::Other("mongo".to_owned()).product_hints(),
        vec!["mongo"]
    );
}

#[test]
fn an_analytics_query_defaults_to_ten_rows_and_no_breakdown() {
    let query = AnalyticsQuery::new("shop", 10, 20);

    assert_eq!(query.limit, 10);
    assert_eq!(query.breakdown, None);
    assert!(query.validate().is_ok());
}

#[test]
fn an_analytics_query_can_break_down_and_cap() {
    let query = AnalyticsQuery::new("shop", 10, 20)
        .with_breakdown(AnalyticsDimension::Country)
        .with_limit(50);

    assert_eq!(query.breakdown, Some(AnalyticsDimension::Country));
    assert_eq!(query.limit, 50);
}

#[test]
fn an_analytics_window_must_move_forward() {
    assert_eq!(
        AnalyticsQuery::new("shop", 20, 20).validate().unwrap_err(),
        Error::InvalidAnalyticsWindow
    );
    assert_eq!(
        AnalyticsQuery::new(" ", 10, 20).validate().unwrap_err(),
        Error::EmptySiteName
    );
}

#[test]
fn every_dimension_has_the_providers_spelling() {
    assert_eq!(AnalyticsDimension::Country.as_str(), "country");
    assert_eq!(AnalyticsDimension::DeviceType.as_str(), "deviceType");
    assert_eq!(AnalyticsDimension::RequestPath.as_str(), "requestPath");
    assert_eq!(
        AnalyticsDimension::ReferrerHostname.as_str(),
        "referrerHostname"
    );
    assert_eq!(AnalyticsDimension::BrowserName.as_str(), "browserName");
    assert_eq!(AnalyticsDimension::OsName.as_str(), "osName");
    assert_eq!(AnalyticsDimension::Route.as_str(), "route");
}

#[test]
fn a_deployment_round_trips_through_json() {
    let deployment = Deployment {
        id: "dpl_1".to_owned(),
        site: "shop".to_owned(),
        url: Some("https://shop.vercel.app".to_owned()),
        status: DeploymentStatus::Building,
        target: DeploymentTarget::Production,
        created_at_ms: Some(1),
        error_message: None,
    };

    let json = serde_json::to_string(&deployment).unwrap();
    assert_eq!(
        serde_json::from_str::<Deployment>(&json).unwrap(),
        deployment
    );
}

#[test]
fn a_status_this_crate_does_not_model_survives_a_round_trip() {
    let status = DeploymentStatus::Other("BLOCKED".to_owned());
    let json = serde_json::to_string(&status).unwrap();

    assert_eq!(
        serde_json::from_str::<DeploymentStatus>(&json).unwrap(),
        status
    );
}
