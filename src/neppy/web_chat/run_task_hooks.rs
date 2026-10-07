//! Test-only hooks run at the top of `run_chat_task` (debug and test builds):
//! a forced failure and a parked turn used by the concurrency / cancellation
//! tests. Split out of `run_task.rs` to keep that file small.

use super::ops::TEST_FORCED_RUN_CHAT_TASK_ERROR;

/// `Some(error)` when a test forced this turn to fail (or parked it until it
/// timed out); `None` for every normal turn.
pub(super) async fn forced_outcome(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
) -> Option<String> {
    {
        let mut slot = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await;
        if let Some(forced) = slot.take() {
            log::debug!(
                "[web-channel][test] forced run_chat_task failure client_id={} thread_id={} request_id={}",
                client_id,
                thread_id,
                request_id
            );
            return Some(forced);
        }
    }

    // Test hook: park the turn in-flight so concurrency / cooperative
    // cancellation can be observed. A `Drop` guard flips the supplied flag if
    // this future is dropped (i.e. cancelled) before the sleep elapses, proving
    // the turn was torn down cooperatively rather than left running.
    {
        let block = {
            let slot = super::ops::TEST_RUN_CHAT_TASK_BLOCK.lock().await;
            slot.clone()
        };
        if let Some(block) = block {
            struct DropGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
            impl Drop for DropGuard {
                fn drop(&mut self) {
                    self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _guard = DropGuard(block.dropped.clone());
            log::debug!(
                "[web-channel][test] parking run_chat_task thread_id={} request_id={}",
                thread_id,
                request_id
            );
            // Signal that the turn future is live and parked, so a test can
            // cancel only after the guard exists (otherwise a `biased` cancel
            // could short-circuit before this future is ever polled).
            block
                .started
                .store(true, std::sync::atomic::Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            return Some("test block elapsed".to_string());
        }
    }
    None
}
