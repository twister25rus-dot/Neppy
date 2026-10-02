//! CPU-cheap counters for the companion (PC15). Atomics only on the hot path;
//! the latency ring is a small mutex touched once per sample. Nothing here
//! carries content.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::Serialize;

use crate::neppy::pet::companion::types::DropReason;

const LATENCY_RING: usize = 256;

/// Live counters. `Default` is all-zero.
#[derive(Default)]
pub struct Metrics {
    pub samples_total: AtomicU64,
    pub events_accepted: AtomicU64,
    pub queue_drops: AtomicU64,
    pub suggestions_created: AtomicU64,
    pub suggestions_acted: AtomicU64,
    pub suggestions_dismissed: AtomicU64,
    pub auto_actions: AtomicU64,
    pub llm_calls: AtomicU64,
    pub local_model_calls: AtomicU64,
    pub ocr_calls: AtomicU64,
    pub ocr_ms_total: AtomicU64,
    pub capture_ms_total: AtomicU64,
    pub screen_unchanged: AtomicU64,
    pub helper_errors: AtomicU64,
    pub sensor_errors: AtomicU64,
    drops: [AtomicU64; DROP_SLOTS],
    latency: Mutex<LatencyRing>,
}

const DROP_SLOTS: usize = 9;

fn drop_slot(r: DropReason) -> usize {
    DropReason::ALL.iter().position(|x| *x == r).unwrap_or(0)
}

#[derive(Default)]
struct LatencyRing {
    buf: Vec<u32>,
    next: usize,
}

/// Wire snapshot (`CompanionStatus.metrics`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MetricsSnapshot {
    pub samples_total: u64,
    pub events_accepted: u64,
    pub queue_drops: u64,
    pub drops_by_reason: BTreeMap<&'static str, u64>,
    pub suggestions_created: u64,
    pub suggestions_acted: u64,
    pub suggestions_dismissed: u64,
    pub auto_actions: u64,
    pub llm_calls: u64,
    pub local_model_calls: u64,
    pub ocr_calls: u64,
    pub ocr_ms_total: u64,
    pub capture_ms_total: u64,
    pub screen_unchanged: u64,
    pub helper_errors: u64,
    pub sensor_errors: u64,
    pub sample_latency_p50_ms: u32,
    pub sample_latency_p95_ms: u32,
}

impl Metrics {
    pub fn inc(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn add(counter: &AtomicU64, n: u64) {
        counter.fetch_add(n, Ordering::Relaxed);
    }

    pub fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }

    pub fn record_drop(&self, reason: DropReason) {
        debug_assert_eq!(DropReason::ALL.len(), DROP_SLOTS);
        self.drops[drop_slot(reason)].fetch_add(1, Ordering::Relaxed);
    }

    pub fn drops(&self, reason: DropReason) -> u64 {
        self.drops[drop_slot(reason)].load(Ordering::Relaxed)
    }

    /// Record one sample's wall time (ms).
    pub fn record_sample(&self, ms: u32) {
        Self::inc(&self.samples_total);
        if let Ok(mut ring) = self.latency.lock() {
            if ring.buf.len() < LATENCY_RING {
                ring.buf.push(ms);
            } else {
                let i = ring.next;
                ring.buf[i] = ms;
            }
            ring.next = (ring.next + 1) % LATENCY_RING;
        }
    }

    fn percentiles(&self) -> (u32, u32) {
        let Ok(ring) = self.latency.lock() else {
            return (0, 0);
        };
        if ring.buf.is_empty() {
            return (0, 0);
        }
        let mut v = ring.buf.clone();
        v.sort_unstable();
        let at = |p: usize| v[((v.len() - 1) * p) / 100];
        (at(50), at(95))
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let (p50, p95) = self.percentiles();
        let drops_by_reason = DropReason::ALL
            .iter()
            .map(|r| (r.as_str(), self.drops(*r)))
            .filter(|(_, n)| *n > 0)
            .collect();
        MetricsSnapshot {
            samples_total: Self::get(&self.samples_total),
            events_accepted: Self::get(&self.events_accepted),
            queue_drops: Self::get(&self.queue_drops),
            drops_by_reason,
            suggestions_created: Self::get(&self.suggestions_created),
            suggestions_acted: Self::get(&self.suggestions_acted),
            suggestions_dismissed: Self::get(&self.suggestions_dismissed),
            auto_actions: Self::get(&self.auto_actions),
            llm_calls: Self::get(&self.llm_calls),
            local_model_calls: Self::get(&self.local_model_calls),
            ocr_calls: Self::get(&self.ocr_calls),
            ocr_ms_total: Self::get(&self.ocr_ms_total),
            capture_ms_total: Self::get(&self.capture_ms_total),
            screen_unchanged: Self::get(&self.screen_unchanged),
            helper_errors: Self::get(&self.helper_errors),
            sensor_errors: Self::get(&self.sensor_errors),
            sample_latency_p50_ms: p50,
            sample_latency_p95_ms: p95,
        }
    }
}
