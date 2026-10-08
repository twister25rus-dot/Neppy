//! "Publish release": runs `scripts/release-neppy.sh <version>` from the project
//! root after an explicit confirm in the UI.
//!
//! UI/RPC only. This is deliberately **not** an agent tool: publishing pushes
//! commits, a tag and a GitHub release signed with the user's key, and a model
//! must never be able to do that. The script stays the single source of truth
//! for every release step; this module starts it, survives the app quitting
//! (the launcher is a detached session), and reconciles the outcome on read.
//!
//! State: `{workspace}/debug_mode/release.json` (record + launcher pid),
//! `release.log` (script output) and `release.exit` (the script's exit code,
//! written by the launcher). A running record is resolved by status reads: the
//! exit file wins, then a dead pid, then a 2 h deadline (surfaced as a failure,
//! never by killing a release mid-push).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::local_install_steps::{build_path, log_tail};
use super::ops::{self, done, DebugCtx, RpcResult};
use super::release_preflight::{
    parse_semver, preflight_with, read_version, redact_urls, ReleaseEnv, ReleasePreflight,
};
use super::release_steps::{
    pid_alive, script_path, ReleaseSpawner, ShellReleaseSpawner, SpawnRequest, LAUNCH_SCRIPT,
};
use super::secrets::mask_secret_like_lines;

const FILE: &str = "release.json";
pub(super) const LOG_FILE: &str = "release.log";
pub(super) const EXIT_FILE: &str = "release.exit";
const LOG_TAIL_BYTES: u64 = 6 * 1024;
const ERROR_CAP: usize = 300;
/// A release that has run this long is reported as failed (and left running).
const MAX_RUN: chrono::Duration = chrono::Duration::hours(2);

static FILE_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleasePhase {
    #[default]
    Idle,
    Running,
    Succeeded,
    Failed,
}

/// What `release_status` / `release_start` return. Field names are the wire
/// contract mirrored by `ReleaseRecord` in `app/src/services/api/debugModeApi.ts`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReleaseRecord {
    #[serde(default)]
    pub phase: ReleasePhase,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// Filled per status call, never persisted.
    #[serde(default)]
    pub log_tail: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub release_url: Option<String>,
    #[serde(default)]
    pub error: String,
}

/// The persisted form: the public record plus what only this module needs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Stored {
    #[serde(flatten)]
    record: ReleaseRecord,
    /// The launcher's pid (it leads its own session).
    #[serde(default)]
    pid: Option<u32>,
    /// Project root the release was started in (for the release URL).
    #[serde(default)]
    root: Option<String>,
}

struct ReleaseStore {
    dir: PathBuf,
}

impl ReleaseStore {
    fn read(&self) -> Result<Stored, String> {
        match fs::read(self.dir.join(FILE)) {
            Ok(b) => Ok(serde_json::from_slice(&b).unwrap_or_else(|e| {
                log::warn!("[debug_mode][release] {FILE} unreadable ({e}); treating as idle");
                Stored::default()
            })),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stored::default()),
            Err(e) => Err(format!("[debug_mode][release] cannot read {FILE}: {e}")),
        }
    }

    fn write(&self, s: &Stored) -> Result<(), String> {
        fs::create_dir_all(&self.dir).map_err(|e| format!("[debug_mode][release] mkdir: {e}"))?;
        let tmp = self
            .dir
            .join(format!(".{FILE}.{}.tmp", uuid::Uuid::new_v4().simple()));
        let mut persisted = s.clone();
        persisted.record.log_tail.clear();
        let data = serde_json::to_vec_pretty(&persisted).map_err(|e| e.to_string())?;
        let go = || -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
            fs::rename(&tmp, self.dir.join(FILE))
        };
        go().map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("[debug_mode][release] write {FILE}: {e}")
        })
    }

    /// The script's exit code, once the launcher has written it.
    fn exit_code(&self) -> Option<i32> {
        fs::read_to_string(self.dir.join(EXIT_FILE))
            .ok()
            .and_then(|s| s.trim().parse().ok())
    }

    /// Resolves a `running` record against reality. Returns true when it changed.
    /// The caller holds [`FILE_LOCK`] and persists the result.
    fn reconcile(&self, s: &mut Stored) -> bool {
        if s.record.phase != ReleasePhase::Running {
            return false;
        }
        if let Some(code) = self.exit_code() {
            log::info!("[debug_mode][release] reconcile: exit file says code={code}");
            self.finish(s, code);
            return true;
        }
        if !s.pid.is_some_and(pid_alive) {
            log::warn!("[debug_mode][release] reconcile: process gone, no exit code recorded");
            fail(s, "the release process exited without reporting a result");
            return true;
        }
        let overdue = s
            .record
            .started_at
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .is_some_and(|t| chrono::Utc::now().signed_duration_since(t) > MAX_RUN);
        if overdue {
            log::warn!("[debug_mode][release] reconcile: running over 2 h; marking failed");
            fail(
                s,
                "the release did not finish within 2 hours; check it manually (it was not stopped)",
            );
            return true;
        }
        false
    }

    fn finish(&self, s: &mut Stored, code: i32) {
        s.record.finished_at = Some(chrono::Utc::now().to_rfc3339());
        s.record.exit_code = Some(code);
        if code == 0 {
            s.record.phase = ReleasePhase::Succeeded;
            s.record.error.clear();
            if let Some(v) = s.record.version.clone() {
                let tag = format!("v{v}");
                s.record.release_url = s
                    .root
                    .as_deref()
                    .and_then(|r| repo_slug(Path::new(r)))
                    .map(|slug| format!("https://github.com/{slug}/releases/tag/{tag}"));
                s.record.tag = Some(tag);
            }
        } else {
            s.record.phase = ReleasePhase::Failed;
            s.record.error = last_log_line(&self.dir.join(LOG_FILE))
                .unwrap_or_else(|| format!("the release script exited with code {code}"));
        }
        log::info!(
            "[debug_mode][release] finished phase={:?} exit_code={code} url_known={}",
            s.record.phase,
            s.record.release_url.is_some()
        );
    }
}

fn fail(s: &mut Stored, reason: &str) {
    s.record.phase = ReleasePhase::Failed;
    s.record.finished_at = Some(chrono::Utc::now().to_rfc3339());
    s.record.error = reason.to_string();
}

/// Last ~6 KB of the log with secret-like lines masked and URL credentials stripped.
fn masked_log_tail(log: &Path) -> String {
    redact_urls(&mask_secret_like_lines(&log_tail(log, LOG_TAIL_BYTES)))
}

/// Last non-empty log line (masked), trimmed to [`ERROR_CAP`] characters.
fn last_log_line(log: &Path) -> Option<String> {
    let tail = masked_log_tail(log);
    let line = tail.lines().map(str::trim).rev().find(|l| !l.is_empty())?;
    let skip = line.chars().count().saturating_sub(ERROR_CAP);
    Some(line.chars().skip(skip).collect())
}

/// `owner/repo` for the release URL: the script's own `REPO="..."` line, then
/// the `origin` remote.
fn repo_slug(root: &Path) -> Option<String> {
    let from_script = fs::read_to_string(script_path(root)).ok().and_then(|text| {
        text.lines().find_map(|l| {
            let rest = l.trim().strip_prefix("REPO=")?;
            let slug = rest.trim().trim_matches(|c| c == '"' || c == '\'');
            valid_slug(slug).then(|| slug.to_string())
        })
    });
    from_script.or_else(|| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["remote", "get-url", "origin"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| slug_from_remote(String::from_utf8_lossy(&out.stdout).trim()))?
    })
}

fn valid_slug(s: &str) -> bool {
    let mut parts = s.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

fn slug_from_remote(url: &str) -> Option<String> {
    let rest = &url[url.find("github.com")? + "github.com".len()..];
    let rest = rest.strip_prefix([':', '/'])?;
    let rest = rest.trim_end_matches('/');
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    valid_slug(rest).then(|| rest.to_string())
}

fn status_in(dir: &Path) -> Result<ReleaseRecord, String> {
    let store = ReleaseStore { dir: dir.into() };
    let mut s = {
        let _g = lock();
        let mut s = store.read()?;
        if store.reconcile(&mut s) {
            store.write(&s)?;
        }
        s
    };
    if s.record.phase != ReleasePhase::Idle {
        s.record.log_tail = masked_log_tail(&dir.join(LOG_FILE));
    }
    Ok(s.record)
}

/// True while a release is genuinely running (after reconciling).
pub(super) fn is_running(dir: &Path) -> Result<bool, String> {
    let store = ReleaseStore { dir: dir.into() };
    let _g = lock();
    let mut s = store.read()?;
    if store.reconcile(&mut s) {
        store.write(&s)?;
    }
    Ok(s.record.phase == ReleasePhase::Running)
}

/// Validates, re-runs the preflight, records `running` and spawns the launcher.
pub(super) async fn start_with(
    dir: &Path,
    root: &Path,
    version: &str,
    env: &ReleaseEnv,
    spawner: &dyn ReleaseSpawner,
) -> Result<ReleaseRecord, String> {
    let shown: String = version.chars().take(40).collect();
    log::info!("[debug_mode][release] start requested version={shown}");
    let wanted = parse_semver(version)
        .ok_or_else(|| format!("version must be X.Y.Z (digits only), got '{shown}'"))?;
    let current = read_version(root)
        .ok_or_else(|| "cannot read `version` from app/package.json".to_string())?;
    let current_parts = parse_semver(&current)
        .ok_or_else(|| format!("app/package.json version '{current}' is not in X.Y.Z form"))?;
    if wanted <= current_parts {
        log::info!("[debug_mode][release] start refused: {shown} is not above {current}");
        return Err(format!(
            "version {version} must be greater than the current version {current}"
        ));
    }
    let pre: ReleasePreflight = preflight_with(dir, root, env).await?;
    if !pre.blockers.is_empty() {
        log::info!(
            "[debug_mode][release] start refused: blockers={:?}",
            pre.blockers
        );
        return Err(format!(
            "release refused, resolve first: {}",
            pre.blockers.join(", ")
        ));
    }
    let script = script_path(root);
    if !script.is_file() {
        return Err(format!("release script missing: {}", script.display()));
    }

    let store = ReleaseStore { dir: dir.into() };
    let log_path = dir.join(LOG_FILE);
    let exit_path = dir.join(EXIT_FILE);
    let _g = lock();
    let mut s = store.read()?;
    if store.reconcile(&mut s) {
        store.write(&s)?;
    }
    if s.record.phase == ReleasePhase::Running {
        log::info!("[debug_mode][release] start refused: already running");
        return Err("a release is already running; wait for it to finish".into());
    }
    fs::create_dir_all(dir).map_err(|e| format!("[debug_mode][release] mkdir: {e}"))?;
    fs::File::create(&log_path).map_err(|e| format!("cannot reset the release log: {e}"))?;
    match fs::remove_file(&exit_path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("cannot clear the previous exit code: {e}")),
    }
    s = Stored {
        record: ReleaseRecord {
            phase: ReleasePhase::Running,
            version: Some(version.to_string()),
            started_at: Some(chrono::Utc::now().to_rfc3339()),
            ..Default::default()
        },
        pid: None,
        root: Some(root.display().to_string()),
    };
    store.write(&s)?;

    let req = SpawnRequest {
        argv: vec![
            "bash".into(),
            "-c".into(),
            LAUNCH_SCRIPT.into(),
            "_".into(),
            version.to_string(),
            exit_path.display().to_string(),
        ],
        cwd: root.to_path_buf(),
        env: vec![
            ("PATH".into(), build_path()),
            ("GGML_NATIVE".into(), "OFF".into()),
        ],
        log_path,
    };
    match spawner.spawn(&req) {
        Ok(pid) => {
            log::info!("[debug_mode][release] launcher started pid={pid} version={version}");
            s.pid = Some(pid);
            store.write(&s)?;
            Ok(s.record)
        }
        Err(e) => {
            log::warn!("[debug_mode][release] launcher failed to start: {e}");
            fail(&mut s, &e);
            let _ = store.write(&s);
            Err(e)
        }
    }
}

/// `debug_mode_release_preflight`.
pub async fn preflight(ctx: &DebugCtx) -> RpcResult<ReleasePreflight> {
    let root = ctx.resolve_root(None).await?;
    done(preflight_with(ctx.store.dir(), &root, &ReleaseEnv::from_process()).await?)
}

/// `debug_mode_release_start`. The UI must have shown the user this exact
/// version and received a confirm before calling.
pub async fn start(ctx: &DebugCtx, version: &str) -> RpcResult<ReleaseRecord> {
    let r = async {
        let root = ctx.resolve_root(None).await?;
        start_with(
            ctx.store.dir(),
            &root,
            version,
            &ReleaseEnv::from_process(),
            &ShellReleaseSpawner,
        )
        .await
    }
    .await;
    ops::audit(ctx, "release_start", version, &r);
    r.and_then(done)
}

/// `debug_mode_release_status`.
pub async fn status(ctx: &DebugCtx) -> RpcResult<ReleaseRecord> {
    done(status_in(ctx.store.dir())?)
}

#[cfg(test)]
#[path = "release_tests.rs"]
mod tests;
