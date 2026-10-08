//! The real steps behind "Build & install locally": a Tauri `.app` build into
//! a private target dir, and the detached installer helper.
//!
//! The build streams its output into a log file (not a pipe) so the status RPC
//! can show a live tail while it runs. It is argv-only, leads its own process
//! group (killed on timeout or when the pipeline future is dropped) and never
//! sees a signing key: the updater artifacts are switched off for this build,
//! so `tauri build` does not ask for one.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;

use super::candidate_steps::tail;

pub(super) const BUILD_TIMEOUT: Duration = Duration::from_secs(60 * 60);
/// Build output, inside `{workspace}/debug_mode/`.
pub(super) const LOG_FILE: &str = "app-build.log";
/// Private cargo target dir (separate from the release and dev caches).
pub(super) const TARGET_DIR: &str = "app-target";
pub(super) const BUNDLE_NAME: &str = "Neppy.app";
pub(super) const HELPER_SCRIPT: &str = "scripts/neppy-install-local.sh";
/// Overrides `bundle.createUpdaterArtifacts` (true in `tauri.conf.json`, which
/// makes tauri demand `TAURI_SIGNING_PRIVATE_KEY`). A local install has no
/// updater tarball, so it needs no key.
const NO_UPDATER_CONFIG: &str = r#"{"bundle":{"createUpdaterArtifacts":false}}"#;

/// Build the bundle; injectable so the state machine is testable without it.
#[async_trait]
pub(super) trait InstallSteps: Send + Sync {
    /// Build the `.app`, streaming output into `log`. Returns the bundle path;
    /// `Err` carries an output tail.
    async fn build(&self, log: &Path) -> Result<PathBuf, String>;
}

/// Starts the installer helper detached from this process.
pub(super) trait HelperSpawner: Send + Sync {
    fn spawn(&self, script: &Path, new_app: &Path, app_pid: u32) -> Result<(), String>;
}

/// Last `max` bytes of a file (lossy UTF-8); empty when unreadable.
pub(super) fn log_tail(path: &Path, max: u64) -> String {
    let Ok(mut f) = std::fs::File::open(path) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    if f.seek(SeekFrom::Start(len.saturating_sub(max))).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.take(max).read_to_end(&mut buf);
    tail(&String::from_utf8_lossy(&buf), max as usize)
}

pub(super) fn tauri_args() -> Vec<String> {
    [
        "build",
        "--bundles",
        "app",
        "--config",
        NO_UPDATER_CONFIG,
        "--",
        "--bin",
        "Neppy",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

pub(super) fn bundle_path(target_dir: &Path) -> PathBuf {
    target_dir
        .join("release")
        .join("bundle")
        .join("macos")
        .join(BUNDLE_NAME)
}

/// A GUI-launched app has a bare PATH; the build needs cargo, node and pnpm.
pub(super) fn build_path() -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        parts.push(Path::new(&home).join(".cargo/bin").display().to_string());
    }
    parts.extend(["/opt/homebrew/bin".into(), "/usr/local/bin".into()]);
    if let Ok(p) = std::env::var("PATH") {
        parts.push(p);
    }
    parts.join(":")
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

/// Kills the build's process group if the future is dropped (cancel / abort).
struct GroupGuard(Option<u32>);

impl Drop for GroupGuard {
    fn drop(&mut self) {
        if self.0.is_some() {
            log::debug!("[debug_mode] local install build dropped; killing process group");
            kill_group(self.0);
        }
    }
}

pub(super) struct TauriSteps {
    pub root: PathBuf,
    pub target_dir: PathBuf,
    pub timeout: Duration,
}

#[async_trait]
impl InstallSteps for TauriSteps {
    async fn build(&self, log_path: &Path) -> Result<PathBuf, String> {
        let app_dir = self.root.join("app");
        let tauri = app_dir.join("node_modules/.bin/tauri");
        if !tauri.is_file() {
            return Err(format!(
                "{} is missing; run `pnpm install` in the project first",
                tauri.display()
            ));
        }
        if let Some(dir) = log_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let log = std::fs::File::create(log_path).map_err(|e| format!("log file: {e}"))?;
        let log2 = log.try_clone().map_err(|e| format!("log file: {e}"))?;
        log::info!(
            "[debug_mode] local install build start target_dir={}",
            self.target_dir.display()
        );
        let mut cmd = tokio::process::Command::new(&tauri);
        cmd.args(tauri_args())
            .current_dir(&app_dir)
            .env("CARGO_TARGET_DIR", &self.target_dir)
            .env("GGML_NATIVE", "OFF")
            .env("PATH", build_path())
            .env_remove("TAURI_SIGNING_PRIVATE_KEY")
            .env_remove("TAURI_SIGNING_PRIVATE_KEY_PASSWORD")
            .env_remove("TAURI_SIGNING_PRIVATE_KEY_PATH")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log2))
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = cmd.spawn().map_err(|e| format!("spawn tauri build: {e}"))?;
        let mut guard = GroupGuard(child.id());
        let status = match tokio::time::timeout(self.timeout, child.wait()).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(format!("waiting on tauri build: {e}")),
            Err(_) => {
                kill_group(guard.0);
                let _ = child.kill().await;
                return Err(format!(
                    "build timed out after {} min\n{}",
                    self.timeout.as_secs() / 60,
                    log_tail(log_path, 4096)
                ));
            }
        };
        // Reap grandchildren the build left behind (pgid still reserved here).
        kill_group(guard.0);
        guard.0 = None;
        if !status.success() {
            return Err(format!(
                "tauri build exited with {:?}\n{}",
                status.code(),
                log_tail(log_path, 6 * 1024)
            ));
        }
        let bundle = bundle_path(&self.target_dir);
        if !bundle.join("Contents/Info.plist").is_file() {
            return Err(format!(
                "build finished but {} is missing",
                bundle.display()
            ));
        }
        log::info!(
            "[debug_mode] local install build ok bundle={}",
            bundle.display()
        );
        Ok(bundle)
    }
}

/// Runs the helper under its own session so it outlives the app.
pub(super) struct ShellSpawner;

impl HelperSpawner for ShellSpawner {
    #[cfg(unix)]
    fn spawn(&self, script: &Path, new_app: &Path, app_pid: u32) -> Result<(), String> {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new("bash");
        cmd.arg(script)
            .arg(new_app)
            .arg(app_pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: `setsid` is async-signal-safe and only detaches the child
        // from this process's session and controlling terminal.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("cannot start the installer helper: {e}"))?;
        log::info!(
            "[debug_mode] local install helper started pid={}",
            child.id()
        );
        Ok(())
    }

    #[cfg(not(unix))]
    fn spawn(&self, _script: &Path, _new_app: &Path, _app_pid: u32) -> Result<(), String> {
        Err("local install is only supported on macOS".into())
    }
}
