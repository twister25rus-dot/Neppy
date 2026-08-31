//! Unit tests for credential handling and redaction.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn trims_the_api_key() {
    let credentials = Credentials::new("  token  ").unwrap();

    assert_eq!(credentials.api_key(), "token");
    assert_eq!(credentials.team(), None);
}

#[test]
fn rejects_an_empty_api_key() {
    assert_eq!(Credentials::new("").unwrap_err(), Error::EmptyApiKey);
    assert_eq!(Credentials::new(" \t\n ").unwrap_err(), Error::EmptyApiKey);
}

#[test]
fn a_blank_team_is_no_team() {
    let credentials = Credentials::new("token").unwrap().with_team("   ");

    assert_eq!(credentials.team(), None);
}

#[test]
fn trims_the_team() {
    let credentials = Credentials::new("token").unwrap().with_team(" team_abc ");

    assert_eq!(credentials.team(), Some("team_abc"));
}

#[test]
fn debug_redacts_the_api_key() {
    let rendered = format!("{:?}", Credentials::new("super-secret").unwrap());

    assert!(!rendered.contains("super-secret"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}

#[test]
fn deserializes_from_a_request_envelope() {
    let credentials: Credentials =
        serde_json::from_str(r#"{"api_key":"  token  ","team":"team_abc"}"#).unwrap();

    assert_eq!(credentials.api_key(), "token");
    assert_eq!(credentials.team(), Some("team_abc"));
}

#[test]
fn deserializes_without_a_team() {
    let credentials: Credentials = serde_json::from_str(r#"{"api_key":"token"}"#).unwrap();

    assert_eq!(credentials.team(), None);
}

#[test]
fn deserializing_an_empty_api_key_fails() {
    let error = serde_json::from_str::<Credentials>(r#"{"api_key":"  "}"#).unwrap_err();

    assert!(error.to_string().contains("api key must not be empty"));
}

#[test]
fn credentials_compare_by_value() {
    assert_eq!(
        Credentials::new("token").unwrap(),
        Credentials::new("token").unwrap()
    );
    assert_ne!(
        Credentials::new("token").unwrap(),
        Credentials::new("other").unwrap()
    );
}
