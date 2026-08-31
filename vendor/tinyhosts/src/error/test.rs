//! Unit tests for the crate-wide error type.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn renders_a_human_readable_message() {
    assert_eq!(Error::EmptyApiKey.to_string(), "api key must not be empty");
}

#[test]
fn a_missing_key_names_every_variable_it_searched() {
    let error = Error::MissingApiKey {
        provider: "vercel".to_owned(),
        variables: "TINYHOSTS_VERCEL_TOKEN, VERCEL_TOKEN".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "no api key for vercel: set one of TINYHOSTS_VERCEL_TOKEN, VERCEL_TOKEN"
    );
}

#[test]
fn a_provider_failure_names_the_provider_and_the_resource() {
    let error = Error::Api {
        provider: "vercel".to_owned(),
        status: 500,
        resource: "deployment".to_owned(),
        message: "internal".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "vercel returned 500 for deployment: internal"
    );
}

#[test]
fn an_unsupported_capability_completes_the_sentence() {
    let error = Error::Unsupported {
        provider: "vercel".to_owned(),
        capability: "provision a mysql database".to_owned(),
    };

    assert_eq!(
        error.to_string(),
        "vercel cannot provision a mysql database"
    );
}

#[test]
fn is_a_standard_error() {
    fn assert_error<E: std::error::Error>(_: &E) {}

    assert_error(&Error::EmptyBundle);
}
