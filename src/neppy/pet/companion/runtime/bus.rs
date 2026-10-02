//! UI event stream for the companion, bridged to Socket.IO as `pet:companion`
//! by `core::socketio`. Payloads carry scrubbed excerpts only (suggestions are
//! re-scrubbed by the store before they exist) and never raw observations.

use std::sync::LazyLock;

use serde::Serialize;
use tokio::sync::broadcast;

use crate::neppy::pet::companion::types::CompanionSuggestion;

/// Socket event name (and its underscore alias) the bridge emits.
pub const SOCKET_EVENT: &str = "pet:companion";
pub const SOCKET_EVENT_ALIAS: &str = "pet_companion";

/// One `pet:companion` frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompanionUiEvent {
    State {
        state: &'static str,
        paused: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        suspended_reason: Option<&'static str>,
        screen_capture_active: bool,
    },
    Suggestion {
        suggestion: CompanionSuggestion,
    },
    SuggestionUpdate {
        suggestion: CompanionSuggestion,
    },
}

static UI_BUS: LazyLock<broadcast::Sender<CompanionUiEvent>> =
    LazyLock::new(|| broadcast::channel(64).0);

/// Subscribe to companion UI events (the socket bridge, tests).
pub fn subscribe_companion_events() -> broadcast::Receiver<CompanionUiEvent> {
    UI_BUS.subscribe()
}

/// Publish an event; returns the number of live receivers.
pub fn publish(event: CompanionUiEvent) -> usize {
    let kind = match &event {
        CompanionUiEvent::State { state, .. } => *state,
        CompanionUiEvent::Suggestion { .. } => "suggestion",
        CompanionUiEvent::SuggestionUpdate { .. } => "suggestion_update",
    };
    log::debug!("[pet::companion] ui event {kind}");
    UI_BUS.send(event).unwrap_or(0)
}
