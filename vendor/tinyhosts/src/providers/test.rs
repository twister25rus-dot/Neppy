//! Unit tests for provider naming, credential lookup, and connection.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::str::FromStr as _;

use super::*;

fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect();

    move |name: &str| map.get(name).cloned()
}

#[test]
fn a_provider_names_itself() {
    assert_eq!(ProviderKind::Vercel.as_str(), "vercel");
    assert_eq!(ProviderKind::Vercel.to_string(), "vercel");
    assert_eq!(ProviderKind::default(), ProviderKind::Vercel);
}

#[test]
fn a_provider_parses_case_insensitively() {
    assert_eq!(
        ProviderKind::from_str("vercel").unwrap(),
        ProviderKind::Vercel
    );
    assert_eq!(
        ProviderKind::from_str("  VERCEL ").unwrap(),
        ProviderKind::Vercel
    );
}

#[test]
fn an_unknown_provider_is_rejected_by_name() {
    let error = ProviderKind::from_str("heroku").unwrap_err();

    assert_eq!(
        error,
        Error::UnknownProvider {
            name: "heroku".to_owned()
        }
    );
}

#[test]
fn the_prefixed_variable_wins() {
    let credentials = ProviderKind::Vercel
        .credentials_from(&lookup(&[
            ("TINYHOSTS_VERCEL_TOKEN", "ours"),
            ("VERCEL_TOKEN", "the-cli-token"),
        ]))
        .unwrap();

    assert_eq!(credentials.api_key(), "ours");
}

#[test]
fn the_providers_own_variable_is_a_fallback() {
    let credentials = ProviderKind::Vercel
        .credentials_from(&lookup(&[
            ("TINYHOSTS_VERCEL_TOKEN", "   "),
            ("VERCEL_TOKEN", "the-cli-token"),
            ("VERCEL_TEAM_ID", "team_abc"),
        ]))
        .unwrap();

    assert_eq!(credentials.api_key(), "the-cli-token");
    assert_eq!(credentials.team(), Some("team_abc"));
}

#[test]
fn a_missing_key_names_every_variable_it_searched() {
    let error = ProviderKind::Vercel
        .credentials_from(&lookup(&[]))
        .unwrap_err();

    assert_eq!(
        error,
        Error::MissingApiKey {
            provider: "vercel".to_owned(),
            variables: "TINYHOSTS_VERCEL_TOKEN, VERCEL_TOKEN".to_owned(),
        }
    );
}

#[test]
fn reading_the_process_environment_either_finds_a_key_or_says_so() {
    // The test suite must not depend on the machine it runs on: either outcome
    // is correct here, and running the call is what exercises the lookup.
    match ProviderKind::Vercel.credentials_from_env() {
        Ok(credentials) => assert!(!credentials.api_key().is_empty()),
        Err(error) => assert!(matches!(error, Error::MissingApiKey { .. })),
    }
}

#[test]
fn variable_lists_are_stable() {
    assert_eq!(
        ProviderKind::Vercel.api_key_variables(),
        &["TINYHOSTS_VERCEL_TOKEN", "VERCEL_TOKEN"]
    );
    assert_eq!(
        ProviderKind::Vercel.team_variables(),
        &["TINYHOSTS_VERCEL_TEAM_ID", "VERCEL_TEAM_ID"]
    );
}

#[test]
fn connecting_yields_a_host_for_the_named_provider() {
    let host = connect(ProviderKind::Vercel, Credentials::new("token").unwrap()).unwrap();

    assert_eq!(host.kind(), ProviderKind::Vercel);
}

#[test]
fn connecting_can_target_another_api_root() {
    let host = connect_to(
        ProviderKind::Vercel,
        Credentials::new("token").unwrap(),
        Some("http://127.0.0.1:1"),
    )
    .unwrap();

    assert_eq!(host.kind(), ProviderKind::Vercel);
}

#[test]
fn connecting_over_plain_http_to_a_non_loopback_host_is_rejected() {
    let error = connect_to(
        ProviderKind::Vercel,
        Credentials::new("token").unwrap(),
        Some("http://api.example.com"),
    )
    .unwrap_err();

    assert_eq!(
        error,
        Error::InsecureBaseUrl {
            base_url: "http://api.example.com".to_owned()
        }
    );
}

#[test]
fn connecting_over_https_to_any_host_is_allowed() {
    let host = connect_to(
        ProviderKind::Vercel,
        Credentials::new("token").unwrap(),
        Some("https://api.example.com"),
    )
    .unwrap();

    assert_eq!(host.kind(), ProviderKind::Vercel);
}

#[test]
fn connecting_over_plain_http_to_localhost_is_allowed() {
    let host = connect_to(
        ProviderKind::Vercel,
        Credentials::new("token").unwrap(),
        Some("http://localhost:1"),
    )
    .unwrap();

    assert_eq!(host.kind(), ProviderKind::Vercel);
}

#[test]
fn connecting_from_the_environment_either_works_or_reports_the_missing_key() {
    match connect_from_env(ProviderKind::Vercel) {
        Ok(host) => assert_eq!(host.kind(), ProviderKind::Vercel),
        Err(error) => assert!(matches!(error, Error::MissingApiKey { .. })),
    }
}
