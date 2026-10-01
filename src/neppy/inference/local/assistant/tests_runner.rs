//! Running the user's test command.
//!
//! Only a command the user supplied in `start_task` is ever run, and never one
//! the model wrote: the model can ask for "run the tests", not choose what
//! that means. It runs through `sh -c` in the project root, in its own process
//! group, with a timeout that kills the whole group and an output cap that
//! keeps only the last bytes.
//!
//! Not sandboxed. The policy gate in `command_policy` is the control. The
//! sandbox family (`neppy::sandbox::cwd_jail`) has a one-shot `spawn`, but it
//! builds the wrapped command without stdio redirection, so output could not be
//! captured through it, and the Landlock backend cannot deny network at all.
//! Wiring it up needs an upstream change (stdio passthrough on `JailBackend`),
//! tracked as a follow-up rather than done by approximation here.

use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::runner::StopSignal;
use super::types::*;

/// How long to wait for output readers after the process ends. A backgrounded
/// grandchild can hold a pipe open indefinitely.
const READER_GRACE: Duration = Duration::from_secs(2);

/// A result recording that the command was not run, and why.
pub(crate) fn refused_result(why: &str) -> TestResult {
    TestResult {
        exit_code: TEST_REFUSED,
        duration_ms: 0,
        tail: format!("test command not run: {why}"),
        timed_out: false,
    }
}

#[derive(Default)]
struct Tail(Vec<u8>);

impl Tail {
    fn push(&mut self, chunk: &[u8]) {
        self.0.extend_from_slice(chunk);
        if self.0.len() > TEST_TAIL_RUN * 2 {
            let cut = self.0.len() - TEST_TAIL_RUN;
            self.0.drain(..cut);
        }
    }
}

async fn drain<R: tokio::io::AsyncRead + Unpin>(mut stream: R, tail: Arc<Mutex<Tail>>) {
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => tail.lock().push(&buf[..n]),
        }
    }
}

#[cfg(unix)]
fn kill_group(pid: u32) {
    // SAFETY: plain signal delivery to the process group this function's
    // caller created with `process_group(0)`.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

/// Resolves when `pid` has exited, *without reaping it*. The caller kills the
/// process group while the leader is still a zombie: a reaped leader frees its
/// pid, and a signal sent to that number afterwards could reach an unrelated
/// group that was handed the same id.
#[cfg(unix)]
fn watch_exit(pid: u32) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_blocking(move || loop {
        // SAFETY: `waitid` only writes `info`; `WNOWAIT` leaves the child
        // waitable, so tokio's own `wait` still collects the status.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let rc = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if rc == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return;
        }
    })
}

/// How a stoppable test run ended.
pub(crate) enum TestOutcome {
    /// The command ran to an end, or was killed by its timeout.
    Finished(TestResult),
    /// The run was stopped from outside; the command was killed and there is no
    /// result to record.
    Stopped(StopReason),
}

/// Who stopped a test run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    Cancelled,
    Disabled,
}

/// How often a running test looks at the master switch.
const SWITCH_POLL: Duration = Duration::from_millis(200);

/// Run `command` in `root`. Never fails: a spawn error is a result with a
/// non-zero exit code, so the step can record it and carry on. Cannot be
/// stopped from outside; see [`run_test_command_until`].
pub(crate) async fn run_test_command(root: &Path, command: &str, timeout: Duration) -> TestResult {
    match run_test_command_until(root, command, timeout, &StopSignal::new()).await {
        TestOutcome::Finished(result) => result,
        TestOutcome::Stopped(_) => unreachable!("a fresh StopSignal is never raised"),
    }
}

/// [`run_test_command`], killed promptly if `stop` is cancelled or the
/// assistant is disabled while it runs.
///
/// The whole process group is killed however the command ends (exit, timeout,
/// cancel): a `sleep 300 &` left behind by a test script would otherwise outlive
/// the task and keep the output pipes open.
pub(crate) async fn run_test_command_until(
    root: &Path,
    command: &str,
    timeout: Duration,
    stop: &StopSignal,
) -> TestOutcome {
    let started = Instant::now();
    log::debug!(
        "[local_assistant:test] running test command ({} chars) timeout={}s",
        command.len(),
        timeout.as_secs()
    );
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            return TestOutcome::Finished(TestResult {
                exit_code: -1,
                duration_ms: 0,
                tail: format!("could not start the test command: {err}"),
                timed_out: false,
            })
        }
    };
    let pid = child.id();
    let tail = Arc::new(Mutex::new(Tail::default()));
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(tokio::spawn(drain(out, Arc::clone(&tail))));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(tokio::spawn(drain(err, Arc::clone(&tail))));
    }
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut poll = tokio::time::interval(SWITCH_POLL);
    let mut stopped: Option<StopReason> = None;
    // Whether the command ended by itself, as opposed to being killed here.
    let mut exited_by_itself = false;
    #[cfg(unix)]
    let mut exited = pid.map(watch_exit);
    let (mut exit_code, timed_out) = loop {
        tokio::select! {
            biased;
            _ = stop.cancel.cancelled() => {
                stopped = Some(StopReason::Cancelled);
                break (-1, false);
            }
            // Unix: the exit is noticed without reaping, so the group can be
            // killed before the leader's pid is released.
            _ = async {
                #[cfg(unix)]
                if let Some(watch) = exited.as_mut() {
                    let _ = watch.await;
                    return;
                }
                std::future::pending::<()>().await
            } => {
                exited_by_itself = true;
                break (-1, false);
            }
            status = child.wait(), if cfg!(not(unix)) => {
                exited_by_itself = true;
                break (status.ok().and_then(|s| s.code()).unwrap_or(-1), false);
            }
            _ = &mut deadline => {
                log::warn!(
                    "[local_assistant:test] timed out after {}s; killing",
                    timeout.as_secs()
                );
                break (-1, true);
            }
            _ = poll.tick() => {
                if !stop.enabled.load(Ordering::SeqCst) {
                    stopped = Some(StopReason::Disabled);
                    break (-1, false);
                }
            }
        }
    };
    // Whatever ended it, nothing the command started may outlive it. A command
    // that already exited may have left background children in its group.
    #[cfg(unix)]
    if let Some(pid) = pid {
        kill_group(pid);
    }
    if stopped.is_some() || timed_out {
        let _ = child.kill().await;
        let _ = child.wait().await;
    } else if exited_by_itself && cfg!(unix) {
        // Now reap the leader and take its status.
        exit_code = match child.wait().await {
            Ok(status) => status.code().unwrap_or(-1),
            Err(err) => {
                log::warn!("[local_assistant:test] wait failed: {err}");
                -1
            }
        };
    }
    for reader in readers {
        if tokio::time::timeout(READER_GRACE, reader).await.is_err() {
            log::debug!("[local_assistant:test] output reader still open; detaching");
        }
    }
    if let Some(reason) = stopped {
        log::info!("[local_assistant:test] stopped ({reason:?}); command killed");
        return TestOutcome::Stopped(reason);
    }
    let raw = tail.lock().0.clone();
    let mut text = String::from_utf8_lossy(&raw).into_owned();
    text = clip_tail(&text, TEST_TAIL_RUN);
    if timed_out {
        text.push_str("\n[test command timed out and was killed]");
    }
    let result = TestResult {
        exit_code,
        duration_ms: started.elapsed().as_millis() as u64,
        tail: text,
        timed_out,
    };
    log::debug!(
        "[local_assistant:test] finished exit={} timed_out={} in {}ms ({} output bytes kept)",
        result.exit_code,
        result.timed_out,
        result.duration_ms,
        result.tail.len()
    );
    TestOutcome::Finished(result)
}

#[cfg(test)]
#[path = "tests_runner_tests.rs"]
mod tests;
