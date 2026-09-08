//! Tests for partial `[[mlx.server]]` updates.

use super::*;

fn base() -> MlxServerConfig {
    MlxServerConfig::default()
}

fn patch_from(json: serde_json::Value) -> ServerPatch {
    serde_json::from_value(json).expect("patch parses")
}

#[test]
fn an_empty_patch_changes_nothing() {
    // The panel saves on edit, so a form submitted unchanged must not trigger
    // a config write and a server restart.
    let mut server = base();
    let outcome = apply(&mut server, ServerPatch::default());

    assert!(outcome.is_empty());
    assert!(!outcome.needs_restart);
}

#[test]
fn resubmitting_identical_values_changes_nothing() {
    let mut server = base();
    server.max_tokens = 512;

    let outcome = apply(
        &mut server,
        patch_from(serde_json::json!({"max_tokens": 512})),
    );

    assert!(outcome.is_empty(), "same value must not count as a change");
}

#[test]
fn only_the_named_fields_move() {
    // Absent means "leave alone" — that is what lets the panel save one field
    // without resending forty and clobbering a concurrent edit.
    let mut server = base();
    server.model = "keep/me".to_string();
    server.max_tokens = 256;

    let outcome = apply(
        &mut server,
        patch_from(serde_json::json!({"max_tokens": 1024})),
    );

    assert_eq!(server.model, "keep/me");
    assert_eq!(server.max_tokens, 1024);
    assert_eq!(outcome.changed, vec!["max_tokens"]);
}

#[test]
fn several_model_slots_can_be_set_at_once() {
    // mlx_vlm.server serves all six slots from one process, which is why the
    // UI ticks models rather than picking one.
    let mut server = base();

    let outcome = apply(
        &mut server,
        patch_from(serde_json::json!({
            "model": "mlx-community/Qwen3.8-27B-nvfp4",
            "stt_model": "mlx-community/whisper-large-v3",
            "tts_model": "mlx-community/Kokoro-82M",
        })),
    );

    assert_eq!(server.model, "mlx-community/Qwen3.8-27B-nvfp4");
    assert_eq!(server.stt_model, "mlx-community/whisper-large-v3");
    assert_eq!(server.tts_model, "mlx-community/Kokoro-82M");
    assert_eq!(outcome.changed.len(), 3);
    assert!(outcome.needs_restart);
}

#[test]
fn clearing_a_slot_is_a_change() {
    // Unticking a model has to reach the server, or the slot would silently
    // stay loaded after the user cleared it.
    let mut server = base();
    server.stt_model = "mlx-community/whisper-large-v3".to_string();

    let outcome = apply(
        &mut server,
        patch_from(serde_json::json!({"stt_model": ""})),
    );

    assert_eq!(server.stt_model, "");
    assert_eq!(outcome.changed, vec!["stt_model"]);
}

#[test]
fn autostart_alone_needs_no_restart() {
    // It is not part of the command line, so a running server does not care.
    let mut server = base();

    let outcome = apply(
        &mut server,
        patch_from(serde_json::json!({"autostart": false})),
    );

    assert_eq!(outcome.changed, vec!["autostart"]);
    assert!(!outcome.needs_restart);
}

#[test]
fn a_tuning_flag_alongside_autostart_still_needs_a_restart() {
    let mut server = base();

    let outcome = apply(
        &mut server,
        patch_from(serde_json::json!({"autostart": false, "kv_bits": 4.0})),
    );

    assert!(outcome.needs_restart);
}

#[test]
fn sampling_sentinels_survive_a_round_trip() {
    // -1 means "leave the server's own default alone", and 0.0 is a real
    // temperature. Both have to be settable.
    let mut server = base();

    apply(&mut server, patch_from(serde_json::json!({"temp": 0.0})));
    assert_eq!(server.temp, 0.0);

    let outcome = apply(&mut server, patch_from(serde_json::json!({"temp": -1.0})));
    assert_eq!(server.temp, -1.0);
    assert_eq!(outcome.changed, vec!["temp"]);
}

#[test]
fn an_unknown_field_is_rejected_rather_than_silently_dropped() {
    // A typo in the panel would otherwise look like a successful save that
    // did nothing.
    let result: Result<ServerPatch, _> =
        serde_json::from_value(serde_json::json!({"max_tokns": 100}));

    assert!(result.is_err());
}
