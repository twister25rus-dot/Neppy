//! Bounded worker metrics: in-memory rings plus daily JSONL files.
//!
//! Samples (memory, gate, worker state) and lifecycle events (spawn, load,
//! unload, stop, crash, restart, pressure transitions, preemption) go to two
//! fixed-capacity rings and are appended to
//! `{workspace}/local_assistant/metrics/samples-YYYYMMDD.jsonl`.
//!
//! Everything is bounded: the rings drop their oldest entry, one day's file
//! stops at `metrics_file_cap_mib` after writing a single `truncated` marker,
//! and only the newest `metrics_retention_days` files are kept. Writes are
//! best-effort — a metrics failure is logged and never fails inference.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
use parking_lot::Mutex;
use serde::Serialize;

/// Samples kept in memory (one hour at the 2 s active cadence).
pub(crate) const SAMPLE_RING_CAP: usize = 1800;
/// Events kept in memory.
pub(crate) const EVENT_RING_CAP: usize = 500;

/// Event kinds, as they appear in the `event` field.
pub(crate) mod event {
    pub(crate) const WORKER_SPAWN: &str = "worker_spawn";
    pub(crate) const MODEL_LOAD_START: &str = "model_load_start";
    pub(crate) const MODEL_READY: &str = "model_ready";
    pub(crate) const MODEL_UNLOAD: &str = "model_unload";
    pub(crate) const WORKER_STOP: &str = "worker_stop";
    pub(crate) const WORKER_CRASH: &str = "worker_crash";
    pub(crate) const WORKER_RESTART: &str = "worker_restart";
    pub(crate) const PRESSURE_TRANSITION: &str = "pressure_transition";
    pub(crate) const REQUEST_PREEMPTED: &str = "request_preempted";
    pub(crate) const ADMISSION_REFUSED: &str = "admission_refused";
    pub(crate) const OLLAMA_UNLOAD: &str = "ollama_unload";
    pub(crate) const UNLOAD_INEFFECTIVE: &str = "unload_ineffective";
}

/// A model Ollama reports as resident.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct OllamaLoaded {
    pub(crate) name: String,
    pub(crate) size_mib: u64,
}

/// One periodic reading.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub(crate) struct MetricsSample {
    pub(crate) ts_ms: i64,
    pub(crate) avail_pct: f64,
    pub(crate) pressure_level: u8,
    pub(crate) pressure_state: String,
    pub(crate) swap_used_mib: u64,
    pub(crate) compressed_mib: u64,
    pub(crate) worker_pid: Option<u32>,
    pub(crate) worker_footprint_mib: Option<u64>,
    pub(crate) worker_rss_mib: Option<u64>,
    pub(crate) worker_state: String,
    pub(crate) loaded_model: Option<String>,
    pub(crate) ollama_loaded: Vec<OllamaLoaded>,
    pub(crate) gate_active: usize,
    pub(crate) gate_waiting: usize,
    pub(crate) server_queue_depth: Option<u64>,
    pub(crate) context_limit: Option<u32>,
    pub(crate) last_prompt_tokens: Option<u64>,
    pub(crate) last_completion_tokens: Option<u64>,
    pub(crate) core_rss_mib: Option<u64>,
    pub(crate) core_footprint_mib: Option<u64>,
    pub(crate) gpu_in_use_mib: Option<u64>,
}

/// One lifecycle event.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct MetricsEvent {
    pub(crate) ts_ms: i64,
    pub(crate) event: String,
    pub(crate) server_id: Option<String>,
    /// Free text; never carries prompts, completions or credentials.
    pub(crate) detail: String,
}

impl MetricsEvent {
    pub(crate) fn new(event: &str, server_id: Option<&str>, detail: impl Into<String>) -> Self {
        Self {
            ts_ms: Utc::now().timestamp_millis(),
            event: event.to_string(),
            server_id: server_id.map(str::to_string),
            detail: detail.into(),
        }
    }
}

/// What `recent` returns.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct MetricsWindow {
    pub(crate) samples: Vec<MetricsSample>,
    pub(crate) events: Vec<MetricsEvent>,
}

#[derive(Default)]
struct FileState {
    dir: Option<PathBuf>,
    day: Option<NaiveDate>,
    bytes_written: u64,
    truncated: bool,
}

/// Token usage of the most recent gated request.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LastUsage {
    pub(crate) prompt_tokens: Option<u64>,
    pub(crate) completion_tokens: Option<u64>,
}

pub(crate) struct MetricsSink {
    samples: Mutex<VecDeque<MetricsSample>>,
    events: Mutex<VecDeque<MetricsEvent>>,
    file: Mutex<FileState>,
    last_usage: Mutex<LastUsage>,
    cap_bytes: Mutex<u64>,
    retention_days: Mutex<u32>,
}

impl Default for MetricsSink {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsSink {
    /// A sink with no directory: rings only, until [`MetricsSink::configure`].
    pub(crate) fn new() -> Self {
        Self {
            samples: Mutex::new(VecDeque::with_capacity(64)),
            events: Mutex::new(VecDeque::with_capacity(64)),
            file: Mutex::new(FileState::default()),
            last_usage: Mutex::new(LastUsage::default()),
            cap_bytes: Mutex::new(20 * 1024 * 1024),
            retention_days: Mutex::new(7),
        }
    }

    /// Point the sink at a directory and set its bounds. A changed directory
    /// (the workspace moves on login) starts a fresh day there.
    pub(crate) fn configure(&self, dir: PathBuf, cap_mib: u64, retention_days: u32) {
        *self.cap_bytes.lock() = cap_mib.max(1) * 1024 * 1024;
        *self.retention_days.lock() = retention_days.max(1);
        let mut file = self.file.lock();
        if file.dir.as_deref() != Some(dir.as_path()) {
            log::debug!("[mlx:worker] metrics directory {}", dir.display());
            *file = FileState {
                dir: Some(dir),
                ..FileState::default()
            };
        }
    }

    /// The directory metrics are written to, under a workspace.
    pub(crate) fn dir_for_workspace(workspace: &Path) -> PathBuf {
        workspace.join("local_assistant").join("metrics")
    }

    pub(crate) fn record_sample(&self, sample: MetricsSample) {
        self.write_line("sample", &sample, Utc::now().date_naive());
        push_bounded(&mut self.samples.lock(), sample, SAMPLE_RING_CAP);
    }

    pub(crate) fn record_event(&self, event: MetricsEvent) {
        log::debug!(
            "[mlx:worker] event {} server={:?} {}",
            event.event,
            event.server_id,
            event.detail
        );
        self.write_line("event", &event, Utc::now().date_naive());
        push_bounded(&mut self.events.lock(), event, EVENT_RING_CAP);
    }

    /// Convenience for `record_event(MetricsEvent::new(..))`.
    pub(crate) fn event(&self, kind: &str, server_id: Option<&str>, detail: impl Into<String>) {
        self.record_event(MetricsEvent::new(kind, server_id, detail));
    }

    pub(crate) fn set_last_usage(&self, prompt_tokens: u64, completion_tokens: u64) {
        *self.last_usage.lock() = LastUsage {
            prompt_tokens: Some(prompt_tokens),
            completion_tokens: Some(completion_tokens),
        };
    }

    pub(crate) fn last_usage(&self) -> LastUsage {
        *self.last_usage.lock()
    }

    /// Entries with `ts_ms >= since_ms`, newest `limit` of each, oldest first.
    pub(crate) fn recent(&self, since_ms: i64, limit: usize, events_only: bool) -> MetricsWindow {
        fn tail<T>(items: Vec<T>, limit: usize) -> Vec<T> {
            let skip = items.len().saturating_sub(limit);
            items.into_iter().skip(skip).collect()
        }
        let samples = if events_only {
            Vec::new()
        } else {
            tail(
                self.samples
                    .lock()
                    .iter()
                    .filter(|s| s.ts_ms >= since_ms)
                    .cloned()
                    .collect(),
                limit,
            )
        };
        let events = tail(
            self.events
                .lock()
                .iter()
                .filter(|e| e.ts_ms >= since_ms)
                .cloned()
                .collect(),
            limit,
        );
        MetricsWindow { samples, events }
    }

    pub(crate) fn latest_sample(&self) -> Option<MetricsSample> {
        self.samples.lock().back().cloned()
    }

    /// Delete all but the newest `retention_days` daily files.
    pub(crate) fn prune(&self, retention_days: u32) -> usize {
        let Some(dir) = self.file.lock().dir.clone() else {
            return 0;
        };
        prune_dir(&dir, retention_days.max(1) as usize)
    }

    /// Append one JSON line for `day`, rotating and capping as needed.
    pub(crate) fn write_line<T: Serialize>(&self, kind: &str, value: &T, day: NaiveDate) {
        let cap = *self.cap_bytes.lock();
        let retention = *self.retention_days.lock() as usize;
        let mut file = self.file.lock();
        let Some(dir) = file.dir.clone() else {
            return;
        };
        let path = dir.join(file_name(day));

        if file.day != Some(day) {
            // New day, or first write since start: continue from what is on
            // disk so a restart does not reset the cap.
            file.day = Some(day);
            file.bytes_written = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            file.truncated = file.bytes_written >= cap || ends_with_marker(&path);
            if let Err(err) = std::fs::create_dir_all(&dir) {
                log::debug!("[mlx:worker] metrics dir {}: {err}", dir.display());
                return;
            }
            let removed = prune_dir(&dir, retention);
            if removed > 0 {
                log::debug!("[mlx:worker] pruned {removed} old metrics files");
            }
        }
        if file.truncated {
            return;
        }

        let mut object = match serde_json::to_value(value) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => return,
        };
        object.insert("kind".into(), serde_json::Value::String(kind.into()));
        let mut line = serde_json::Value::Object(object).to_string();
        line.push('\n');

        let line = if file.bytes_written + line.len() as u64 > cap {
            file.truncated = true;
            format!(
                "{{\"kind\":\"truncated\",\"ts_ms\":{},\"cap_bytes\":{cap}}}\n",
                Utc::now().timestamp_millis()
            )
        } else {
            line
        };

        let result = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut handle| handle.write_all(line.as_bytes()));
        match result {
            Ok(()) => file.bytes_written += line.len() as u64,
            Err(err) => log::debug!("[mlx:worker] metrics write {}: {err}", path.display()),
        }
    }
}

/// Whether a day's file already ends in the `truncated` marker, so a restart
/// does not write a second one.
fn ends_with_marker(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut handle) = std::fs::File::open(path) else {
        return false;
    };
    let len = handle.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = handle.seek(SeekFrom::Start(len.saturating_sub(128)));
    let mut tail = Vec::new();
    let _ = handle.read_to_end(&mut tail);
    String::from_utf8_lossy(&tail)
        .lines()
        .last()
        .is_some_and(|line| line.contains("\"kind\":\"truncated\""))
}

fn push_bounded<T>(ring: &mut VecDeque<T>, item: T, cap: usize) {
    while ring.len() >= cap {
        ring.pop_front();
    }
    ring.push_back(item);
}

pub(crate) fn file_name(day: NaiveDate) -> String {
    format!("samples-{}.jsonl", day.format("%Y%m%d"))
}

fn prune_dir(dir: &Path, keep: usize) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("samples-") && n.ends_with(".jsonl"))
        })
        .collect();
    // YYYYMMDD sorts chronologically.
    files.sort();
    let excess = files.len().saturating_sub(keep);
    files
        .into_iter()
        .take(excess)
        .filter(|path| std::fs::remove_file(path).is_ok())
        .count()
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
