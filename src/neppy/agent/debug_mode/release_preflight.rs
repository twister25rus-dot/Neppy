//! Preflight for "Publish release": everything the UI needs to decide whether
//! the button may be offered, using the same rules `scripts/release-neppy.sh`
//! enforces (branch, clean tree, origin not ahead of us, signing key, `gh`).
//!
//! The script stays the single source of truth; this only predicts its own
//! early refusals so the user is told before a confirm dialog, not after a
//! failed run. It never reads the signing key, only checks that the file exists.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::git::Git;
use super::release;
use super::release_steps::{GhAuthProbe, GhProbe};

/// `git fetch` budget; on failure the preflight continues from local refs.
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const SHORT_GIT_TIMEOUT: Duration = Duration::from_secs(15);
const FETCH_ERROR_CAP: usize = 300;
/// Signing key location relative to `$HOME` (the script's `KEY`).
const KEY_REL: &str = ".neppy-updater/neppy.key";
const DEFAULT_BRANCH: &str = "main";

pub(super) const NOT_RELEASE_BRANCH: &str = "not_release_branch";
pub(super) const DIRTY: &str = "dirty";
pub(super) const BEHIND: &str = "behind";
pub(super) const NO_SIGNING_KEY: &str = "no_signing_key";
pub(super) const GH_NOT_READY: &str = "gh_not_ready";
pub(super) const RELEASE_RUNNING: &str = "release_running";
pub(super) const NOTHING_TO_RELEASE: &str = "nothing_to_release";

/// What `release_preflight` returns. Field names are the wire contract mirrored
/// by `ReleasePreflight` in `app/src/services/api/debugModeApi.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReleasePreflight {
    pub project_root: String,
    pub branch: String,
    pub release_branch: String,
    pub clean: bool,
    pub behind: bool,
    pub ahead_commits: u64,
    pub current_version: String,
    pub suggested_version: String,
    pub signing_key_present: bool,
    pub gh_ready: bool,
    pub fetch_error: Option<String>,
    pub blockers: Vec<String>,
    pub last_tag: Option<String>,
}

/// The machine-dependent inputs, injectable so tests need no real `$HOME`,
/// environment variable or `gh`.
pub(super) struct ReleaseEnv {
    pub home: Option<PathBuf>,
    pub release_branch: String,
    pub gh: Arc<dyn GhProbe>,
}

impl ReleaseEnv {
    pub(super) fn from_process() -> Self {
        let raw = std::env::var("NEPPY_RELEASE_BRANCH").unwrap_or_default();
        Self {
            home: std::env::var_os("HOME").map(PathBuf::from),
            release_branch: sanitize_branch(&raw),
            gh: Arc::new(GhAuthProbe),
        }
    }
}

/// The script treats an empty `NEPPY_RELEASE_BRANCH` as unset. A value that
/// could be read as a git option, or that is not a plausible branch name, falls
/// back to `main` rather than reaching a git argv.
pub(super) fn sanitize_branch(raw: &str) -> String {
    let v = raw.trim();
    if v.is_empty() {
        return DEFAULT_BRANCH.into();
    }
    let ok = !v.starts_with('-')
        && !v.contains("..")
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '.'));
    if ok {
        v.into()
    } else {
        log::warn!("[debug_mode][release] ignoring an invalid NEPPY_RELEASE_BRANCH; using main");
        DEFAULT_BRANCH.into()
    }
}

/// `X.Y.Z` as three numbers; the same shape the script accepts.
pub(super) fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.split('.');
    let mut next = || {
        let p = parts.next()?;
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse::<u64>().ok()
    };
    let out = (next()?, next()?, next()?);
    parts.next().is_none().then_some(out)
}

pub(super) fn bump_patch(v: &str) -> Option<String> {
    let (a, b, c) = parse_semver(v)?;
    Some(format!("{a}.{b}.{}", c.checked_add(1)?))
}

/// `version` from `app/package.json`.
pub(super) fn read_version(root: &Path) -> Option<String> {
    let bytes = std::fs::read(root.join("app/package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v.get("version")?.as_str().map(str::to_string)
}

/// Strips `user:password@` from URLs so a remote's credentials never reach a
/// payload or a log.
pub(super) fn redact_urls(s: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"://[^/@\s]+@").expect("static regex"))
        .replace_all(s, "://***@")
        .into_owned()
}

fn first_line(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let line = text.lines().map(str::trim).find(|l| !l.is_empty());
    let line = redact_urls(line.unwrap_or(""));
    line.chars().take(FETCH_ERROR_CAP).collect()
}

/// Best-effort `git fetch origin <branch> --tags`; `Some(reason)` on failure.
async fn fetch(git: &Git, branch: &str) -> Option<String> {
    log::debug!("[debug_mode][release] preflight fetch branch={branch}");
    match git
        .run(&["fetch", "origin", branch, "--tags"], Some(FETCH_TIMEOUT))
        .await
    {
        Err(e) => Some(first_line(e.as_bytes())),
        Ok(out) if out.timed_out => Some(format!(
            "git fetch timed out after {} s",
            FETCH_TIMEOUT.as_secs()
        )),
        Ok(out) if out.exit_code == Some(0) => None,
        Ok(out) => {
            let reason = first_line(&out.stderr);
            Some(if reason.is_empty() {
                format!("git fetch exited with {:?}", out.exit_code)
            } else {
                reason
            })
        }
    }
}

/// Runs the preflight against `root`. `dir` is the Debug Mode state dir, which
/// holds the release record (so `release_running` can be reported).
pub(super) async fn preflight_with(
    dir: &Path,
    root: &Path,
    env: &ReleaseEnv,
) -> Result<ReleasePreflight, String> {
    let branch_name = env.release_branch.as_str();
    log::debug!(
        "[debug_mode][release] preflight start root={} release_branch={branch_name}",
        root.display()
    );
    let current_version = read_version(root)
        .ok_or_else(|| "cannot read `version` from app/package.json".to_string())?;
    let suggested_version = bump_patch(&current_version).ok_or_else(|| {
        format!("app/package.json version '{current_version}' is not in X.Y.Z form")
    })?;

    let git = Git::new(root);
    let branch = git
        .branch()
        .await?
        .unwrap_or_else(|| "(detached HEAD)".to_string());
    let clean = git.dirty().await?.is_clean();

    let fetch_error = fetch(&git, branch_name).await;
    if let Some(e) = &fetch_error {
        log::warn!("[debug_mode][release] preflight fetch failed; using local refs: {e}");
    }

    let origin_ref = format!("origin/{branch_name}");
    let has_ref = git
        .run(
            &["rev-parse", "--verify", "-q", &origin_ref],
            Some(SHORT_GIT_TIMEOUT),
        )
        .await?
        .success();
    // Like the script: no origin ref, or one that HEAD does not contain, is behind.
    let behind = if has_ref {
        !git.run(
            &["merge-base", "--is-ancestor", &origin_ref, "HEAD"],
            Some(SHORT_GIT_TIMEOUT),
        )
        .await?
        .success()
    } else {
        true
    };
    let ahead_commits = if has_ref {
        let range = format!("{origin_ref}..HEAD");
        let out = git
            .run(&["rev-list", "--count", &range], Some(SHORT_GIT_TIMEOUT))
            .await?;
        if out.success() {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse()
                .unwrap_or(0)
        } else {
            0
        }
    } else {
        0
    };

    let last_tag = {
        let out = git
            .run(
                &["describe", "--tags", "--abbrev=0", "--match", "v*"],
                Some(SHORT_GIT_TIMEOUT),
            )
            .await?;
        out.success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
            .filter(|t| !t.is_empty())
    };

    let signing_key_present = env.home.as_ref().is_some_and(|h| h.join(KEY_REL).is_file());
    let gh_ready = env.gh.ready().await;
    let release_running = release::is_running(dir)?;

    let mut blockers: Vec<String> = Vec::new();
    let mut push = |cond: bool, code: &str| {
        if cond {
            blockers.push(code.to_string());
        }
    };
    push(branch != branch_name, NOT_RELEASE_BRANCH);
    push(!clean, DIRTY);
    push(behind, BEHIND);
    push(!signing_key_present, NO_SIGNING_KEY);
    push(!gh_ready, GH_NOT_READY);
    push(release_running, RELEASE_RUNNING);
    push(ahead_commits == 0, NOTHING_TO_RELEASE);

    log::info!(
        "[debug_mode][release] preflight done branch={branch} clean={clean} behind={behind} \
         ahead={ahead_commits} key={signing_key_present} gh={gh_ready} running={release_running} \
         blockers={blockers:?}"
    );
    Ok(ReleasePreflight {
        project_root: root.display().to_string(),
        branch,
        release_branch: env.release_branch.clone(),
        clean,
        behind,
        ahead_commits,
        current_version,
        suggested_version,
        signing_key_present,
        gh_ready,
        fetch_error,
        blockers,
        last_tag,
    })
}
