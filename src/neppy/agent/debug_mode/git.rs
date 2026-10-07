//! Git plumbing for Debug Mode: argv-only calls with `-C <root>`, every one
//! under a timeout. Nothing here uses `reset --hard`, `clean` or a branch
//! checkout, and read paths set `GIT_OPTIONAL_LOCKS=0` so a status never
//! refreshes (writes) the index.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::exec::{self, CmdOutput, Keep, RunSpec};
use super::types::{DirtyFiles, PROJECT_ROOT_ENV};

const GIT_TIMEOUT: Duration = Duration::from_secs(60);
const GIT_OUT_CAP: usize = 16 * 1024 * 1024;

/// Tree ids for a checkpoint: what is staged, and what is on disk.
#[derive(Debug, Clone)]
pub struct Trees {
    pub index_tree: String,
    pub worktree_tree: String,
}

#[derive(Debug, Clone)]
pub struct Git {
    root: PathBuf,
}

impl Git {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Run git; a non-zero exit is returned as-is in the output.
    pub async fn run(&self, args: &[&str], timeout: Option<Duration>) -> Result<CmdOutput, String> {
        self.run_cap(args, timeout, GIT_OUT_CAP).await
    }

    /// [`Git::run`] with an explicit per-stream output cap (head is kept).
    pub async fn run_cap(
        &self,
        args: &[&str],
        timeout: Option<Duration>,
        cap: usize,
    ) -> Result<CmdOutput, String> {
        self.run_env(args, timeout, cap, &[]).await
    }

    /// [`Git::run_cap`] with extra environment variables (e.g. `GIT_INDEX_FILE`).
    pub async fn run_env(
        &self,
        args: &[&str],
        timeout: Option<Duration>,
        cap: usize,
        extra_env: &[(&str, &str)],
    ) -> Result<CmdOutput, String> {
        let mut argv: Vec<String> = vec!["-c".into(), "core.quotepath=off".into()];
        // Internal checkpoint snapshots (`commit-tree`) get a fixed identity and
        // no signing so they never prompt; a user-facing `git commit` keeps the
        // user's own identity and signing configuration.
        if args.first() == Some(&"commit-tree") {
            for c in [
                "user.name=neppy-debug",
                "user.email=debug@neppy.local",
                "commit.gpgsign=false",
            ] {
                argv.push("-c".into());
                argv.push(c.into());
            }
        }
        argv.extend(args.iter().map(|s| s.to_string()));
        log::debug!("[debug_mode] git {}", args.first().copied().unwrap_or(""));
        let mut env: Vec<(&str, &str)> =
            vec![("GIT_OPTIONAL_LOCKS", "0"), ("GIT_TERMINAL_PROMPT", "0")];
        env.extend_from_slice(extra_env);
        exec::run(RunSpec {
            program: "git",
            args: &argv,
            cwd: &self.root,
            timeout: timeout.unwrap_or(GIT_TIMEOUT),
            cap,
            keep: Keep::Head,
            env: &env,
        })
        .await
    }

    /// Run git and require exit 0; returns stdout.
    pub async fn ok(&self, args: &[&str]) -> Result<Vec<u8>, String> {
        self.ok_env(args, &[]).await
    }

    /// [`Git::ok`] with extra environment variables.
    pub async fn ok_env(&self, args: &[&str], env: &[(&str, &str)]) -> Result<Vec<u8>, String> {
        let out = self.run_env(args, None, GIT_OUT_CAP, env).await?;
        if out.timed_out {
            return Err(format!("git {} timed out", args.first().unwrap_or(&"")));
        }
        if out.exit_code != Some(0) {
            let msg = String::from_utf8_lossy(&out.stderr);
            let first = msg.lines().next().unwrap_or("").trim();
            return Err(format!(
                "git {} failed (exit {:?}): {first}",
                args.first().unwrap_or(&""),
                out.exit_code
            ));
        }
        Ok(out.stdout)
    }

    pub async fn ok_str(&self, args: &[&str]) -> Result<String, String> {
        Ok(String::from_utf8_lossy(&self.ok(args).await?)
            .trim()
            .to_string())
    }

    /// HEAD commit sha, `None` on an unborn branch.
    pub async fn head(&self) -> Result<Option<String>, String> {
        let out = self
            .run(&["rev-parse", "--verify", "-q", "HEAD"], None)
            .await?;
        if out.success() {
            Ok(Some(
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            ))
        } else {
            Ok(None)
        }
    }

    /// Current branch, `None` when detached.
    pub async fn branch(&self) -> Result<Option<String>, String> {
        let out = self
            .run(&["symbolic-ref", "--short", "-q", "HEAD"], None)
            .await?;
        if out.success() {
            Ok(Some(
                String::from_utf8_lossy(&out.stdout).trim().to_string(),
            ))
        } else {
            Ok(None)
        }
    }

    pub async fn dirty(&self) -> Result<DirtyFiles, String> {
        let out = self
            .ok(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])
            .await?;
        Ok(parse_porcelain_z(&out))
    }

    /// Write trees for the staged state and for the whole working tree
    /// (tracked + untracked, `.gitignore` respected) without ever touching the
    /// real index: a copy of it is used as `GIT_INDEX_FILE`, then discarded.
    pub async fn snapshot_trees(&self) -> Result<Trees, String> {
        let idx = self.ok_str(&["rev-parse", "--git-path", "index"]).await?;
        let real = if Path::new(&idx).is_absolute() {
            PathBuf::from(&idx)
        } else {
            self.root.join(&idx)
        };
        let tmp = tempfile::tempdir().map_err(|e| format!("temp index dir: {e}"))?;
        let tmp_index = tmp.path().join("index");
        if real.is_file() {
            std::fs::copy(&real, &tmp_index).map_err(|e| format!("copy index: {e}"))?;
        }
        let tmp_str = tmp_index.to_string_lossy().into_owned();
        let env = [("GIT_INDEX_FILE", tmp_str.as_str())];
        let tree = |out: Vec<u8>| String::from_utf8_lossy(&out).trim().to_string();
        let index_tree = tree(self.ok_env(&["write-tree"], &env).await?);
        self.ok_env(&["add", "-A"], &env).await?;
        let worktree_tree = tree(self.ok_env(&["write-tree"], &env).await?);
        Ok(Trees {
            index_tree,
            worktree_tree,
        })
    }

    /// Untracked, non-ignored files (files only; nested repos are skipped).
    pub async fn untracked(&self) -> Result<Vec<String>, String> {
        let out = self
            .ok(&["ls-files", "--others", "--exclude-standard", "-z"])
            .await?;
        let mut v: Vec<String> = split_nul(&out)
            .into_iter()
            .filter(|p| !p.ends_with('/'))
            .collect();
        v.sort();
        Ok(v)
    }
}

pub fn split_nul(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Parse `git status --porcelain=v1 -z` output into [`DirtyFiles`].
pub fn parse_porcelain_z(bytes: &[u8]) -> DirtyFiles {
    let mut dirty = DirtyFiles::default();
    let tokens: Vec<&[u8]> = bytes.split(|b| *b == 0).collect();
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        i += 1;
        if t.len() < 4 {
            continue;
        }
        let (x, y) = (t[0] as char, t[1] as char);
        let path = String::from_utf8_lossy(&t[3..]).into_owned();
        // Renames/copies are followed by the original path token.
        if matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C') {
            i += 1;
        }
        if x == '?' && y == '?' {
            dirty.untracked.push(path);
        } else if x == '!' {
            continue;
        } else if x == 'A' {
            dirty.added.push(path);
        } else if x == 'D' || y == 'D' {
            dirty.deleted.push(path);
        } else {
            dirty.modified.push(path);
        }
    }
    for v in [
        &mut dirty.modified,
        &mut dirty.added,
        &mut dirty.deleted,
        &mut dirty.untracked,
    ] {
        v.sort();
    }
    dirty
}

/// Resolve PROJECT_ROOT: explicit param, then `NEPPY_DEBUG_PROJECT_ROOT`, then
/// `config.debug_mode.project_root`, then the crate's compile-time manifest dir.
/// The result is canonical, exists, and must be the root of a git work tree.
pub async fn resolve_project_root(explicit: Option<&str>) -> Result<PathBuf, String> {
    resolve_project_root_with(explicit, None).await
}

/// [`resolve_project_root`] with the configured root (`config_root`) slotted
/// between the environment variable and the build-time default.
pub async fn resolve_project_root_with(
    explicit: Option<&str>,
    config_root: Option<&str>,
) -> Result<PathBuf, String> {
    let non_empty = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let (raw, source) = match explicit.and_then(non_empty) {
        Some(p) => (p, "param"),
        None => match crate::neppy::util::env::var(PROJECT_ROOT_ENV)
            .ok()
            .and_then(|s| non_empty(&s))
        {
            Some(p) => (p, "env"),
            None => match config_root.and_then(non_empty) {
                Some(p) => (p, "config"),
                None => (env!("CARGO_MANIFEST_DIR").to_string(), "default"),
            },
        },
    };
    log::debug!("[debug_mode] resolve project root source={source}");
    let root = std::fs::canonicalize(&raw)
        .map_err(|e| format!("project root '{raw}' is not accessible: {e}"))?;
    if !root.is_dir() {
        return Err(format!(
            "project root '{}' is not a directory",
            root.display()
        ));
    }
    let top = Git::new(&root)
        .ok_str(&["rev-parse", "--show-toplevel"])
        .await
        .map_err(|e| {
            format!(
                "project root '{}' is not a git work tree: {e}",
                root.display()
            )
        })?;
    let top =
        std::fs::canonicalize(&top).map_err(|e| format!("cannot resolve git toplevel: {e}"))?;
    if top != root {
        return Err(format!(
            "project root '{}' is not the root of its git work tree (toplevel is '{}')",
            root.display(),
            top.display()
        ));
    }
    Ok(root)
}
