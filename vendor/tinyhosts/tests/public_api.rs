//! Integration tests for the public crate surface.
//!
//! These tests link against the crate as a downstream consumer would: they can
//! only use what `src/lib.rs` re-exports. Treat them as the regression suite
//! for the crate's public contract — if a change breaks a test here, it is a
//! breaking change for users.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinyhosts::{
    Bundle, Credentials, DatabaseKind, DatabaseSpec, DeployRequest, DeploymentTarget, EnvVar,
    Error, Framework, LaunchPlan, ProviderKind, SiteSpec,
};

fn bundle() -> Bundle {
    let mut bundle = Bundle::new();
    bundle
        .insert("package.json", br#"{"name":"shop"}"#.to_vec())
        .unwrap();
    bundle
}

#[test]
fn a_consumer_can_describe_a_whole_launch() {
    let plan = LaunchPlan::new(SiteSpec::new("shop"), bundle())
        .with_database(DatabaseSpec::new("shop-db").with_kind(DatabaseKind::Postgres))
        .with_env(vec![EnvVar::new("NEXT_PUBLIC_NAME", "Shop").secret()])
        .with_domains(vec!["shop.example".to_owned()])
        .into_production();

    assert!(plan.validate().is_ok());
    assert_eq!(plan.target, DeploymentTarget::Production);
    assert_eq!(plan.site.framework, Framework::NextJs);
}

#[test]
fn a_consumer_can_connect_to_a_provider() {
    let host = tinyhosts::connect(
        ProviderKind::Vercel,
        Credentials::new("token").unwrap().with_team("team_abc"),
    )
    .unwrap();

    assert_eq!(host.kind(), ProviderKind::Vercel);
}

#[test]
fn a_consumer_can_reach_a_provider_through_a_proxy() {
    let host = tinyhosts::connect_to(
        ProviderKind::Vercel,
        Credentials::new("token").unwrap(),
        Some("https://vercel.internal.example"),
    )
    .unwrap();

    assert_eq!(host.kind(), ProviderKind::Vercel);
}

#[test]
fn failures_are_reported_as_typed_errors() {
    assert_eq!(Credentials::new("  ").unwrap_err(), Error::EmptyApiKey);
    assert_eq!(
        DeployRequest::new("shop", Bundle::new())
            .validate()
            .unwrap_err(),
        Error::EmptyBundle
    );
    assert_eq!(
        SiteSpec::new(" ").validate().unwrap_err(),
        Error::EmptySiteName
    );
}

#[test]
fn a_credential_is_never_rendered() {
    let credentials = Credentials::new("super-secret").unwrap();

    assert!(!format!("{credentials:?}").contains("super-secret"));
}

#[test]
fn a_bundle_reads_a_directory_and_skips_what_a_build_produces() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("package.json"), b"{}").unwrap();
    std::fs::create_dir(root.path().join("node_modules")).unwrap();
    std::fs::write(root.path().join("node_modules/x.js"), b"x").unwrap();

    let bundle = Bundle::from_dir(root.path()).unwrap();

    assert_eq!(bundle.len(), 1);
    assert_eq!(bundle.files()[0].path(), "package.json");
    assert!(tinyhosts::EXCLUDED.contains(&"node_modules"));
}

#[tokio::test]
async fn a_json_request_reaches_the_same_surface() {
    // Port 1 refuses connections, so this exercises the envelope and the
    // dispatch without depending on a provider being reachable.
    let request = serde_json::json!({
        "operation": "list_sites",
        "credentials": {"api_key": "token"},
        "base_url": "http://127.0.0.1:1",
    });

    let error = tinyhosts::execute_json(&request.to_string())
        .await
        .unwrap_err();

    assert!(matches!(error, Error::Transport { .. }), "{error:?}");
}

#[tokio::test]
async fn a_malformed_json_request_is_rejected_as_an_envelope() {
    let error = tinyhosts::execute_json("not json").await.unwrap_err();

    assert!(matches!(error, Error::Envelope { .. }), "{error:?}");
}

#[test]
fn the_build_reports_which_providers_it_has() {
    assert!(tinyhosts::rpc::providers().contains(&"vercel"));
}
