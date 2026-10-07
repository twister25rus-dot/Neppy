//! The real build / launch / health-check steps of a staged-update candidate
//! (spec sections 35-36). Everything here runs *outside* the running app: the
//! candidate is built into an isolated target dir and launched as a separate
//! process on a random loopback port with a throwaway workspace, HOME, token
//! and file-backed keyring, so it can never touch the user's real data.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::exec::{self, Keep, RunSpec};

pub(super) const BUILD_TIMEOUT: Duration = Duration::from_secs(40 * 60);
pub(super) const HEALTH_DEADLINE: Duration = Duration::from_secs(90);
const OUTPUT_CAP: usize = 64 * 1024;
/// Cap on `CandidateRecord::error_tail`.
pub(super) const ERROR_TAIL_CAP: usize = 8 * 1024;

/// A launched candidate process. Dropping it kills the whole process group.
pub(super) struct Launched {
    pub port: u16,
    pub token: String,
    pub log_path: Option<PathBuf>,
    pid: Option<u32>,
    child: Option<tokio::process::Child>,
    _tmp: Option<tempfile::TempDir>,
}

impl Launched {
    /// A launch that owns no process (test runners).
    pub(super) fn inert() -> Self {
        Self {
            port: 0,
            token: String::new(),
            log_path: None,
            pid: None,
            child: None,
            _tmp: None,
        }
    }
}

impl Drop for Launched {
    fn drop(&mut self) {
        kill_group(self.pid);
    }
}

#[cfg(unix)]
fn kill_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: plain syscall on a pgid created via `process_group(0)`.
        unsafe {
            libc::killpg(pid as i32, libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: Option<u32>) {}

/// The three fallible stages of a candidate, injectable so the state machine
/// is testable without running cargo.
#[async_trait]
pub(super) trait Steps: Send + Sync {
    /// Build the candidate binary; `Err` carries an output tail.
    async fn build(&self) -> Result<(), String>;
    /// Start the candidate as a separate process.
    async fn launch(&self) -> Result<Launched, String>;
    /// Wait for `/health`, then make one authenticated RPC.
    async fn health(&self, launched: &mut Launched) -> Result<(), String>;
    /// Stop the process (idempotent; `Drop` also kills it).
    async fn stop(&self, launched: Launched);
}

/// Last `max` bytes of `s`, on a char boundary.
pub(super) fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.trim().to_string();
    }
    let mut cut = s.len() - max;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    s[cut..].trim().to_string()
}

pub(super) fn binary_path(target_dir: &Path) -> PathBuf {
    target_dir
        .join("debug")
        .join(format!("neppy-core{}", std::env::consts::EXE_SUFFIX))
}

pub(super) struct CargoSteps {
    pub root: PathBuf,
    pub target_dir: PathBuf,
}

fn read_tail(path: &Path) -> String {
    std::fs::read(path)
        .map(|b| tail(&String::from_utf8_lossy(&b), 2048))
        .unwrap_or_default()
}

fn free_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| format!("free port: {e}"))?;
    l.local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("free port: {e}"))
}

#[async_trait]
impl Steps for CargoSteps {
    async fn build(&self) -> Result<(), String> {
        log::info!(
            "[debug_mode] candidate build start target_dir={}",
            self.target_dir.display()
        );
        let target = self.target_dir.to_string_lossy().into_owned();
        let args: Vec<String> = ["build", "--bin", "neppy-core"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out = exec::run(RunSpec {
            program: "cargo",
            args: &args,
            cwd: &self.root,
            timeout: BUILD_TIMEOUT,
            cap: OUTPUT_CAP,
            keep: Keep::Tail,
            env: &[
                ("CARGO_TARGET_DIR", target.as_str()),
                ("GGML_NATIVE", "OFF"),
            ],
        })
        .await?;
        if out.success() {
            return Ok(());
        }
        let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
        if text.trim().is_empty() {
            text = String::from_utf8_lossy(&out.stdout).into_owned();
        }
        let head = if out.timed_out {
            format!(
                "build timed out after {} min\n",
                BUILD_TIMEOUT.as_secs() / 60
            )
        } else {
            format!("cargo build exited with {:?}\n", out.exit_code)
        };
        Err(format!("{head}{}", tail(&text, ERROR_TAIL_CAP - 100)))
    }

    async fn launch(&self) -> Result<Launched, String> {
        let bin = binary_path(&self.target_dir);
        if !bin.is_file() {
            return Err(format!("candidate binary missing: {}", bin.display()));
        }
        let tmp = tempfile::tempdir().map_err(|e| format!("temp dir: {e}"))?;
        let (ws, home) = (tmp.path().join("workspace"), tmp.path().join("home"));
        for d in [&ws, &home] {
            std::fs::create_dir_all(d).map_err(|e| format!("temp dir: {e}"))?;
        }
        let port = free_port()?;
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let log_path = tmp.path().join("candidate.log");
        let log = std::fs::File::create(&log_path).map_err(|e| format!("log file: {e}"))?;
        let log2 = log.try_clone().map_err(|e| format!("log file: {e}"))?;
        let mut cmd = tokio::process::Command::new(&bin);
        cmd.args(["serve", "--host", "127.0.0.1", "--port"])
            .arg(port.to_string())
            .arg("--headless-api")
            .current_dir(tmp.path())
            .env("NEPPY_WORKSPACE", &ws)
            .env("NEPPY_CORE_TOKEN", &token)
            .env("NEPPY_KEYRING_BACKEND", "file")
            .env("HOME", &home)
            .env_remove("NEPPY_CORE_PORT")
            .env_remove("NEPPY_CORE_HOST")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log2))
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd.spawn().map_err(|e| format!("spawn candidate: {e}"))?;
        let pid = child.id();
        log::info!("[debug_mode] candidate launched pid={pid:?} port={port}");
        Ok(Launched {
            port,
            token,
            log_path: Some(log_path),
            pid,
            child: Some(child),
            _tmp: Some(tmp),
        })
    }

    async fn health(&self, l: &mut Launched) -> Result<(), String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .no_proxy()
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        let base = format!("http://127.0.0.1:{}", l.port);
        let deadline = Instant::now() + HEALTH_DEADLINE;
        let log_tail = |l: &Launched| l.log_path.as_deref().map(read_tail).unwrap_or_default();
        let mut last;
        loop {
            if let Some(child) = l.child.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    return Err(format!(
                        "candidate exited before becoming healthy ({status})\n{}",
                        log_tail(l)
                    ));
                }
            }
            match client.get(format!("{base}/health")).send().await {
                Ok(r) if r.status().is_success() => break,
                Ok(r) => last = format!("/health returned {}", r.status()),
                Err(e) => last = format!("/health: {e}"),
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "health check timed out after {}s ({last})\n{}",
                    HEALTH_DEADLINE.as_secs(),
                    log_tail(l)
                ));
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        log::debug!("[debug_mode] candidate /health ok; calling core.ping");
        let body =
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "core.ping", "params": {}});
        let resp = client
            .post(format!("{base}/rpc"))
            .bearer_auth(&l.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("core.ping: {e}"))?;
        let status = resp.status();
        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        if !status.is_success() || json.get("error").is_some() || json.get("result").is_none() {
            return Err(format!(
                "core.ping failed (http {status}): {}",
                tail(&json.to_string(), 500)
            ));
        }
        Ok(())
    }

    async fn stop(&self, mut l: Launched) {
        // `take()` so `Drop` never signals a pgid that may be recycled by now.
        kill_group(l.pid.take());
        if let Some(child) = l.child.as_mut() {
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        }
        log::debug!("[debug_mode] candidate stopped");
    }
}
