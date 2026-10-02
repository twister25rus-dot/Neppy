//! RPC operations: settings get/update validation, lease, pause persistence,
//! status shape, permissions, and the no-content logging rule.

use std::sync::Arc;

use serde_json::json;

use super::ops;
use super::state::{Runtime, SystemClock};
use super::test_support::*;

#[test]
fn get_returns_defaults_and_unavailable_sources() {
    let h = harness();
    let s = ops::get(&h.rt, &h.config).unwrap();
    assert_eq!(s["enabled"], false, "OFF until the user consents");
    for k in ["app_window", "selection", "clipboard", "screen_capture"] {
        assert_eq!(s["sources"][k], true, "D1: every source on once enabled");
    }
    assert_eq!(s["allow_cloud_model"], true, "D3");
    assert_eq!(s["ocr_languages"], json!(["en-US"]));
    assert!(s["unavailable_sources"].as_array().unwrap().len() >= 1);
    assert_eq!(s["hotkeys"]["pause"], "CmdOrCtrl+Alt+Shift+P");
}

#[test]
fn update_errors_name_the_field() {
    let h = harness();
    let bad = |v: serde_json::Value| {
        ops::update(&h.rt, &h.config, v.as_object().unwrap().clone()).unwrap_err()
    };
    assert!(bad(json!({ "nope": 1 })).contains("nope"));
    assert!(bad(json!({ "category_levels": { "send_message": 2 } })).contains("high-risk"));
    assert!(bad(json!({ "excluded_title_patterns": ["re:(unclosed"] }))
        .contains("excluded_title_patterns"));
    assert!(bad(json!({ "hotkeys": { "pause": "P" } })).contains("hotkeys.pause"));
    assert!(bad(json!({ "ocr_languages": ["english please"] })).contains("ocr_languages"));
    assert!(bad(json!({ "level": 9 })).contains("'level'"));
    let ok = ops::update(
        &h.rt,
        &h.config,
        json!({ "ocr_languages": ["en-US", "de-DE"] })
            .as_object()
            .unwrap()
            .clone(),
    )
    .unwrap();
    assert_eq!(ok["ocr_languages"], json!(["en-US", "de-DE"]));
}

#[test]
fn lease_reports_state_and_hotkeys() {
    let h = harness();
    let r = ops::lease(&h.rt, true, &[]);
    assert_eq!(r["enabled"], false);
    assert_eq!(r["state"], "off");
    assert_eq!(r["hotkeys"]["ask"], "CmdOrCtrl+Alt+Shift+Space");
    h.enable();
    let r = ops::lease(&h.rt, true, &["pause".into()]);
    assert_eq!(r["state"], "observing");
    assert_eq!(r["platform_supported"], true);
    let r = ops::lease(&h.rt, false, &[]);
    assert_eq!(r["state"], "suspended");
}

#[test]
fn pause_persists_across_a_restart() {
    let h = harness();
    h.enable();
    let st = ops::pause(&h.rt, &h.config, Some(30), "tray").unwrap();
    assert_eq!(st["state"], "paused");
    assert_eq!(st["paused"], true);
    assert!(st["paused_until"].is_string());
    assert!(ops::pause(&h.rt, &h.config, Some(0), "ui")
        .unwrap_err()
        .contains("'minutes'"));
    // A fresh runtime on the same workspace comes back paused.
    let fresh = Runtime::new(
        h.sensor.clone(),
        h.generator.clone(),
        h.handoff.clone(),
        Arc::new(SystemClock),
        fast_timing(),
    );
    fresh.bind(&h.config).unwrap();
    fresh.stop_sampler();
    assert_eq!(fresh.state().0, "paused");
    let st = ops::resume(&fresh, &h.config, "ui").unwrap();
    assert_eq!(st["paused"], false);
}

#[test]
fn status_has_the_contract_fields() {
    let h = harness();
    h.enable();
    let st = ops::status(&h.rt, &h.config).unwrap();
    assert_eq!(st["state"], "suspended");
    assert_eq!(st["suspended_reason"], "no_indicator");
    for k in [
        "screen_capture_active",
        "platform_supported",
        "lease_active",
        "effective_level",
        "tier_cap",
        "permissions",
        "recent",
        "metrics",
    ] {
        assert!(st.get(k).is_some(), "status misses {k}");
    }
    assert!(st["metrics"]["drops_by_reason"].is_object());
    assert_eq!(st["metrics"]["samples_total"], 0);
}

#[test]
fn ask_and_capture_respect_the_gate() {
    let h = harness();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let out = rt.block_on(ops::ask(&h.rt, &h.config, "hotkey")).unwrap();
    assert_eq!(out["status"], "disabled");
    h.enable();
    let out = rt
        .block_on(ops::capture(&h.rt, &h.config, "hotkey"))
        .unwrap();
    assert_eq!(out["status"], "no_indicator");
    h.lease();
    *h.sensor.screen_perm.lock().unwrap() =
        crate::neppy::desktop::accessibility::PermissionState::Denied;
    let out = rt
        .block_on(ops::capture(&h.rt, &h.config, "hotkey"))
        .unwrap();
    assert_eq!(out["status"], "permission_required");
    *h.sensor.screen_perm.lock().unwrap() =
        crate::neppy::desktop::accessibility::PermissionState::Granted;
    let out = rt
        .block_on(ops::capture(&h.rt, &h.config, "hotkey"))
        .unwrap();
    assert_eq!(out["status"], "cancelled");
    h.sensor.set_app("Bitwarden", "com.bitwarden.desktop", 9);
    let out = rt.block_on(ops::ask(&h.rt, &h.config, "hotkey")).unwrap();
    assert_eq!(out["status"], "excluded");
    assert_eq!(
        h.sensor.count("context"),
        0,
        "no title read for an excluded app"
    );
}

#[test]
fn request_permission_validates_kind() {
    let h = harness();
    assert!(ops::request_permission(&h.rt, "camera")
        .unwrap_err()
        .contains("'kind'"));
    let r = ops::request_permission(&h.rt, "screen_recording").unwrap();
    assert_eq!(r["state"], "granted");
}

/// PC16: no log line in the runtime formats observed text, prompts or model
/// output (titles, selections, clipboard, OCR, bodies, excerpts).
#[test]
fn runtime_logs_never_format_content() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/neppy/pet/companion/runtime");
    const FORBIDDEN: &[&str] = &[
        "text",
        "title",
        "selection",
        "prompt",
        "body",
        "excerpt",
        "raw",
        "sel",
        "clip",
    ];
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".rs") || name.ends_with("_tests.rs") || name == "test_support.rs" {
            continue;
        }
        let src = std::fs::read_to_string(entry.path()).unwrap();
        let mut rest = src.as_str();
        while let Some(i) = rest.find("log::") {
            let stmt_end = rest[i..].find(");").map(|e| i + e).unwrap_or(rest.len());
            let stmt = &rest[i..stmt_end];
            // Named inline args `{x}` and trailing positional args.
            let args: String = stmt
                .split('"')
                .enumerate()
                .map(|(n, part)| {
                    if n % 2 == 1 {
                        inline_args(part)
                    } else {
                        part.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            for f in FORBIDDEN {
                let hit = args
                    .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                    .any(|tok| {
                        tok == *f
                            || tok == format!("raw_{f}")
                            || tok.ends_with(&format!("_{f}")) && *f != "text"
                    });
                assert!(
                    !hit,
                    "{name}: log statement may format content ({f}): {stmt}"
                );
            }
            rest = &rest[stmt_end.min(rest.len())..];
            if rest.is_empty() {
                break;
            }
            rest = &rest[1..];
        }
    }
}

fn inline_args(fmt: &str) -> String {
    let mut out = String::new();
    let mut s = fmt;
    while let Some(o) = s.find('{') {
        let Some(c) = s[o..].find('}') else { break };
        let inner = &s[o + 1..o + c];
        out.push_str(inner.split(':').next().unwrap_or(""));
        out.push(' ');
        s = &s[o + c + 1..];
    }
    out
}
