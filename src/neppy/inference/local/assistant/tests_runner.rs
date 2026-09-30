//! Running the user's test command.
//!
//! Only a command the user supplied in `start_task` is ever run, and never one
//! the model wrote: the model can ask for "run the tests", not choose what
//! that means. It runs through `sh -c` in the project root, in its own process
//! group, with a timeout that kills the whole group and an output cap that
//! keeps only the last bytes.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::neppy::security::policy::{CommandClass, GateDecision};
use crate::neppy::security::SecurityPolicy;

use super::types::*;

/// How long to wait for output readers after the process ends. A backgrounded
/// grandchild can hold a pipe open indefinitely.
const READER_GRACE: Duration = Duration::from_secs(2);

/// Whether the policy lets this command run at all.
///
/// `Block` is refused outright (a read-only tier allows only read commands).
/// `Prompt` is allowed: the user wrote the command into the task request, which
/// is the approval. `Destructive` is refused regardless, because a test command
/// has no business being catastrophic.
pub(crate) fn check_command(
    policy: &SecurityPolicy,
    command: &str,
) -> std::result::Result<(), String> {
    let class = policy.classify_command(command);
    if policy.gate_decision(class) == GateDecision::Block {
        return Err(format!(
            "blocked by the autonomy tier (command class {class:?})"
        ));
    }
    if class == CommandClass::Destructive {
        return Err("a destructive command cannot be used as a test command".into());
    }
    Ok(())
}

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

/// Run `command` in `root`. Never fails: a spawn error is a result with a
/// non-zero exit code, so the step can record it and carry on.
pub(crate) async fn run_test_command(root: &Path, command: &str, timeout: Duration) -> TestResult {
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
            return TestResult {
                exit_code: -1,
                duration_ms: 0,
                tail: format!("could not start the test command: {err}"),
                timed_out: false,
            }
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
    let (exit_code, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => (status.code().unwrap_or(-1), false),
        Ok(Err(err)) => {
            log::warn!("[local_assistant:test] wait failed: {err}");
            (-1, false)
        }
        Err(_) => {
            log::warn!(
                "[local_assistant:test] timed out after {}s; killing",
                timeout.as_secs()
            );
            #[cfg(unix)]
            if let Some(pid) = pid {
                kill_group(pid);
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
            (-1, true)
        }
    };
    for reader in readers {
        if tokio::time::timeout(READER_GRACE, reader).await.is_err() {
            log::debug!("[local_assistant:test] output reader still open; detaching");
        }
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
    result
}

#[cfg(test)]
#[path = "tests_runner_tests.rs"]
mod tests;
