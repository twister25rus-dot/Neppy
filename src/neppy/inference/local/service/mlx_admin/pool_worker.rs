//! Worker-control half of the MLX pool: PID lookup and crash reaping.
//!
//! Split from `pool.rs` to keep that file within the size guideline. It is a
//! child module, so it reaches the pool's private process map directly.

use std::sync::Arc;

use crate::neppy::config::Config;

use super::super::metrics::{event, MetricsSink};
use super::{MlxPool, WorkerProcess};

impl MlxPool {
    /// A pool that records lifecycle events into `metrics`.
    pub(crate) fn with_metrics(metrics: Arc<MetricsSink>) -> Self {
        Self {
            metrics: Some(metrics),
            ..Self::default()
        }
    }

    pub(super) fn record(&self, kind: &str, id: &str, detail: String) {
        if let Some(metrics) = &self.metrics {
            metrics.event(kind, Some(id), detail);
        }
    }

    /// PID of a block this pool holds, whether or not it is still alive.
    pub(crate) async fn pid_of(&self, id: &str) -> Option<u32> {
        self.running
            .lock()
            .await
            .get(id)
            .map(|entry| entry.process.pid)
    }

    /// Whether this pool holds `id` and its process has not exited.
    pub(crate) async fn is_running(&self, id: &str) -> bool {
        self.process_of(id).await.is_some_and(|p| p.alive)
    }

    /// The held process for `id`, reaping its exit status without blocking.
    pub(crate) async fn process_of(&self, id: &str) -> Option<WorkerProcess> {
        let mut running = self.running.lock().await;
        let entry = running.get_mut(id)?;
        let alive = entry.process.exit_status().is_none();
        Some(WorkerProcess {
            pid: entry.process.pid,
            alive,
        })
    }

    /// Stop `id` only if this pool holds its process. Unlike [`Self::stop`]
    /// there is no spawn-marker fallback: a server this process did not start
    /// (the user's own, another core's, a CLI start) is never touched.
    pub(crate) async fn stop_if_held(&self, config: &Config, id: &str) -> bool {
        let entry = self.running.lock().await.remove(id);
        match entry {
            Some(mut entry) => {
                entry.process.stop(config).await;
                self.record(event::WORKER_STOP, id, format!("pid={}", entry.process.pid));
                true
            }
            None => false,
        }
    }

    /// Remove every entry whose process has exited, clearing its spawn marker.
    ///
    /// Without this a crashed server stays in the map as `Crashed` forever,
    /// and `start` refuses it as "already running". Returns the reaped ids.
    pub(crate) async fn reap_crashed(&self, config: &Config) -> Vec<String> {
        let mut reaped = Vec::new();
        {
            let mut running = self.running.lock().await;
            let dead: Vec<(String, String)> = running
                .iter_mut()
                .filter_map(|(id, entry)| {
                    entry
                        .process
                        .exit_status()
                        .map(|status| (id.clone(), status))
                })
                .collect();
            for (id, status) in dead {
                if let Some(entry) = running.remove(&id) {
                    let tail = entry.process.logs_tail(3).join(" | ");
                    log::warn!(
                        "[mlx:worker] reaped crashed server `{id}` pid={} ({status})",
                        entry.process.pid
                    );
                    // The child is already gone; this only clears its marker.
                    super::super::process::clear_marker_only(config, &id);
                    self.record(
                        event::WORKER_CRASH,
                        &id,
                        format!("pid={} {status}; {tail}", entry.process.pid),
                    );
                    reaped.push(id);
                }
            }
        }
        reaped
    }

    /// Insert a process as if `start` had spawned it.
    #[cfg(test)]
    pub(crate) async fn insert_for_test(&self, process: super::super::process::MlxProcess) {
        use super::super::health::MlxServerState;
        let id = process.id.clone();
        self.running.lock().await.insert(
            id,
            super::RunningServer {
                process,
                state: MlxServerState::Starting,
                has_been_ready: false,
                detail: None,
                estimated_gib: None,
            },
        );
    }
}
