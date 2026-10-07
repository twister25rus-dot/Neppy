//! Argv-only child-process runner with a timeout and bounded output capture.
//!
//! Every Debug Mode command (git and validation checks alike) goes through
//! [`run`]: no shell is ever involved, output is capped while it is read (a
//! chatty build cannot exhaust memory), the child leads its own process group,
//! and a timeout kills the whole group (npm/cargo spawn grandchildren).

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

/// Which end of an over-long stream to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Head,
    Tail,
}

pub struct RunSpec<'a> {
    pub program: &'a str,
    pub args: &'a [String],
    pub cwd: &'a Path,
    pub timeout: Duration,
    /// Per-stream byte cap.
    pub cap: usize,
    pub keep: Keep,
    pub env: &'a [(&'a str, &'a str)],
}

#[derive(Debug)]
pub struct CmdOutput {
    /// `None` when killed by a signal or on timeout.
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
    pub duration: Duration,
}

impl CmdOutput {
    pub fn success(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }
}

async fn read_capped<R: AsyncRead + Unpin>(mut r: R, cap: usize, keep: Keep) -> (Vec<u8>, bool) {
    let mut buf: Vec<u8> = Vec::new();
    let mut truncated = false;
    let mut chunk = [0u8; 8192];
    loop {
        let n = match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        match keep {
            Keep::Head => {
                let room = cap.saturating_sub(buf.len());
                let take = n.min(room);
                buf.extend_from_slice(&chunk[..take]);
                if take < n {
                    truncated = true;
                }
            }
            Keep::Tail => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > cap.saturating_mul(2).max(cap + 1) {
                    let cut = buf.len() - cap;
                    buf.drain(..cut);
                    truncated = true;
                }
            }
        }
    }
    if keep == Keep::Tail && buf.len() > cap {
        let cut = buf.len() - cap;
        buf.drain(..cut);
        truncated = true;
    }
    (buf, truncated)
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: plain syscall on a pgid we created via `process_group(0)`.
        unsafe {
            libc::killpg(pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: Option<u32>) {}

/// Inherited git environment that could redirect a git call to another repo,
/// index or object store. Removed from every child; callers re-add what they
/// need (e.g. `GIT_INDEX_FILE`) through [`RunSpec::env`].
const SCRUBBED_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CEILING_DIRECTORIES",
];

/// Kills the child's whole process group if the future is dropped before the
/// command finished (`kill_on_drop` alone only reaches the direct child).
struct GroupGuard(Option<u32>);

impl GroupGuard {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if self.0.is_some() {
            log::debug!("[debug_mode] exec future dropped; killing process group");
            kill_group(self.0);
        }
    }
}

/// Run `spec.program` with `spec.args` (argv, no shell) under a timeout.
///
/// `Err` means the process could not be spawned or waited on; a non-zero exit
/// or a timeout is an `Ok` with the details in [`CmdOutput`].
pub async fn run(spec: RunSpec<'_>) -> Result<CmdOutput, String> {
    let mut cmd = Command::new(spec.program);
    cmd.args(spec.args)
        .current_dir(spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    for k in SCRUBBED_ENV {
        cmd.env_remove(k);
    }
    for (k, v) in spec.env {
        cmd.env(k, v);
    }
    let started = Instant::now();
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn '{}': {e}", spec.program))?;
    let pid = child.id();
    let mut guard = GroupGuard(pid);
    let (cap, keep) = (spec.cap, spec.keep);
    let out_task = child
        .stdout
        .take()
        .map(|s| tokio::spawn(read_capped(s, cap, keep)));
    let err_task = child
        .stderr
        .take()
        .map(|s| tokio::spawn(read_capped(s, cap, keep)));

    let (exit_code, timed_out) = match tokio::time::timeout(spec.timeout, child.wait()).await {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(format!("failed waiting on '{}': {e}", spec.program)),
        Err(_) => {
            log::debug!(
                "[debug_mode] timeout after {:?}; killing process group program={}",
                spec.timeout,
                spec.program
            );
            kill_group(pid);
            let _ = child.kill().await;
            (None, true)
        }
    };

    // Reap background grandchildren the command left behind (they would also
    // hold the output pipes open). Done while the leader's pgid is still
    // reserved by `guard`, then disarmed so a recycled pid is never signalled.
    kill_group(pid);
    guard.disarm();

    async fn collect(task: Option<tokio::task::JoinHandle<(Vec<u8>, bool)>>) -> (Vec<u8>, bool) {
        match task {
            None => (Vec::new(), false),
            Some(mut t) => match tokio::time::timeout(Duration::from_secs(5), &mut t).await {
                Ok(Ok(v)) => v,
                _ => {
                    t.abort();
                    (Vec::new(), true)
                }
            },
        }
    }
    let (stdout, stdout_truncated) = collect(out_task).await;
    let (stderr, stderr_truncated) = collect(err_task).await;
    let duration = started.elapsed();
    log::debug!(
        "[debug_mode] exec program={} exit={:?} timed_out={} duration_ms={}",
        spec.program,
        exit_code,
        timed_out,
        duration.as_millis()
    );
    Ok(CmdOutput {
        exit_code,
        stdout,
        stderr,
        stdout_truncated,
        stderr_truncated,
        timed_out,
        duration,
    })
}
