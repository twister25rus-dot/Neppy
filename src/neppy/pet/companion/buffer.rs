//! In-memory ring of recent observations. Bounded three ways (50 events,
//! 32 KiB of text, 10 min TTL) and cleared on pause, disable, delete-all and
//! lease loss. Nothing here is persisted: PC3 says raw observations never are.
//!
//! Alongside the events (which carry scrubbed text) the buffer keeps a small
//! ring of text-free [`ObservationSummary`] rows, including drops, so the Now
//! tab can show "what the pet sees" and why something was ignored.

use std::collections::VecDeque;

use chrono::{DateTime, Duration, Utc};

use super::types::{DropReason, ObservationEvent, ObservationKind, ObservationSummary};

pub const MAX_EVENTS: usize = 50;
pub const MAX_TEXT_BYTES: usize = 32 * 1024;
pub const TTL_MINUTES: i64 = 10;
pub const MAX_SUMMARIES: usize = 50;

#[derive(Default)]
pub struct ObservationBuffer {
    events: VecDeque<ObservationEvent>,
    text_bytes: usize,
    summaries: VecDeque<ObservationSummary>,
}

impl ObservationBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an accepted event (and its summary), evicting expired and oldest.
    pub fn push(&mut self, event: ObservationEvent) {
        self.evict_expired(event.at);
        self.push_summary(event.summary());
        self.text_bytes += event.text_bytes();
        self.events.push_back(event);
        while self.events.len() > MAX_EVENTS || self.text_bytes > MAX_TEXT_BYTES {
            match self.events.pop_front() {
                Some(old) => self.text_bytes = self.text_bytes.saturating_sub(old.text_bytes()),
                None => break,
            }
        }
    }

    /// Record a discarded observation (no text).
    pub fn record_drop(
        &mut self,
        at: DateTime<Utc>,
        kind: ObservationKind,
        app_name: &str,
        reason: DropReason,
    ) {
        self.evict_expired(at);
        self.push_summary(ObservationSummary {
            at,
            kind,
            app_name: app_name.to_string(),
            title_excerpt: None,
            dropped: Some(reason),
        });
    }

    fn push_summary(&mut self, s: ObservationSummary) {
        self.summaries.push_back(s);
        while self.summaries.len() > MAX_SUMMARIES {
            self.summaries.pop_front();
        }
    }

    /// Drop everything older than the TTL relative to `now`.
    pub fn evict_expired(&mut self, now: DateTime<Utc>) {
        let cutoff = now - Duration::minutes(TTL_MINUTES);
        while self.events.front().is_some_and(|e| e.at < cutoff) {
            if let Some(old) = self.events.pop_front() {
                self.text_bytes = self.text_bytes.saturating_sub(old.text_bytes());
            }
        }
        while self.summaries.front().is_some_and(|s| s.at < cutoff) {
            self.summaries.pop_front();
        }
    }

    /// Forget everything (pause, disable, delete-all, lease loss).
    pub fn clear(&mut self) {
        self.events.clear();
        self.summaries.clear();
        self.text_bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.summaries.is_empty()
    }

    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }

    /// The newest event, if any.
    pub fn last(&self) -> Option<&ObservationEvent> {
        self.events.back()
    }

    /// Events from oldest to newest.
    pub fn events(&self) -> impl Iterator<Item = &ObservationEvent> {
        self.events.iter()
    }

    /// The `n` newest summaries, newest first.
    pub fn summaries(&self, n: usize) -> Vec<ObservationSummary> {
        self.summaries.iter().rev().take(n).cloned().collect()
    }
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod buffer_tests;
