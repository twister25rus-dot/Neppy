//! The project index: filenames, symbols and content tokens for a project that
//! stays on disk.
//!
//! Refresh is incremental and bounded. A stat walk finds the files whose
//! `(size, mtime)` changed; only those are re-read, and a file whose hash did
//! not change is not re-indexed. Work is applied in batches of
//! `index_batch_files`, one transaction each, yielding between them, so a
//! first index of a large project does not hold a thread or the database.
//!
//! Nothing here puts a whole project, or a whole file, in a prompt: retrieval
//! returns at most one chunk or window per file, read back from disk.

mod scan;
mod search;
mod store;
mod symbols;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::neppy::config::schema::LocalAssistantConfig;

use super::types::{clip, now_ms, AssistantError, Result};
use scan::{is_generated_path, looks_binary, ExcludeSet};
use store::{ChunkRec, IndexDb};

pub(crate) use scan::is_generated_path as is_generated_rel;
pub(crate) use scan::{git_head, is_generated, list_candidates};
pub(crate) use search::query_terms;
pub(crate) use symbols::{extract_symbols, Lang};

/// Lines per chunk, and the byte ceiling of one chunk's indexed text.
const CHUNK_LINES: usize = 80;
const CHUNK_BYTES: usize = 4096;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RefreshStats {
    /// Paths the listing returned.
    pub listed: usize,
    /// Files re-read because their stat changed.
    pub reread: usize,
    /// Files whose content was (re)indexed.
    pub changed: usize,
    /// Rows deleted because the file is gone or no longer eligible.
    pub removed: usize,
    pub skipped_generated: usize,
    pub skipped_ignored: usize,
    pub skipped_binary: usize,
    pub skipped_large: usize,
    /// Files left out by `index_max_files` / `index_max_bytes`.
    pub skipped_cap: usize,
    /// Bytes of eligible files.
    pub bytes: u64,
    /// Transactions used.
    pub batches: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexStatus {
    pub root: String,
    pub files: u64,
    pub chunks: u64,
    pub symbols: u64,
    pub db_bytes: u64,
    pub last_refresh_ms: Option<i64>,
    pub git_head: Option<String>,
}

enum Work {
    Item(PlanItem),
    RemoveFile(i64),
    RemoveSkip(String),
}

struct PlanItem {
    rel: String,
    size: u64,
    mtime_ns: i64,
    existing: Option<i64>,
    /// Re-index even if the file hash is unchanged (a chunk hash disagreed
    /// with the disk).
    force: bool,
}

pub struct ProjectIndex {
    pub(super) root: PathBuf,
    db_path: PathBuf,
    pub(super) db: Mutex<IndexDb>,
    pub(super) cache: Mutex<search::QueryCache>,
}

/// Open (creating if needed) the index for `root` under `workspace`.
pub fn open_index(workspace: &Path, root: &Path) -> Result<ProjectIndex> {
    let root = root
        .canonicalize()
        .map_err(|e| AssistantError::Invalid(format!("project root: {e}")))?;
    let key = sha_hex(root.to_string_lossy().as_bytes());
    let db_path = workspace
        .join("local_assistant")
        .join("index")
        .join(format!("{}.db", &key[..16]));
    let db = IndexDb::open(&db_path)?;
    db.set_meta("root", &root.to_string_lossy())?;
    log::debug!(
        "[local_assistant:index] opened index for {} at {}",
        root.display(),
        db_path.display()
    );
    Ok(ProjectIndex {
        root,
        db_path,
        db: Mutex::new(db),
        cache: Mutex::new(search::QueryCache::default()),
    })
}

/// Whether any of `globs` excludes the project-relative path `rel`.
pub(crate) fn exclude_matches(globs: &[String], rel: &str) -> bool {
    ExcludeSet::new(globs).matches(rel)
}

pub(crate) fn sha_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn mtime_ns_of(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Lines of `text` with their terminators, the unit chunk hashes are taken
/// over. Shared by indexing and verification so the two cannot disagree.
pub(crate) fn line_slices(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn chunk_text(text: &str) -> Vec<ChunkRec> {
    let lines = line_slices(text);
    let mut out = Vec::new();
    let mut start = 0usize;
    while start < lines.len() {
        let mut end = start;
        let mut bytes = 0usize;
        while end < lines.len() && end - start < CHUNK_LINES {
            if end > start && bytes + lines[end].len() > CHUNK_BYTES {
                break;
            }
            bytes += lines[end].len();
            end += 1;
        }
        let body: String = lines[start..end].concat();
        out.push(ChunkRec {
            start: (start + 1) as u32,
            end: end as u32,
            sha: sha_hex(body.as_bytes()),
            text: clip(&body, CHUNK_BYTES),
        });
        start = end;
    }
    out
}

impl ProjectIndex {
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Bring the index up to date. Synchronous; see [`Self::refresh_async`].
    pub fn refresh(&self, cfg: &LocalAssistantConfig) -> Result<RefreshStats> {
        let (batches, mut stats, head) = self.plan_work(cfg)?;
        for batch in batches {
            let part = self.apply_batch(batch, cfg)?;
            merge(&mut stats, &part);
        }
        self.finish_refresh(&stats, head)?;
        Ok(stats)
    }

    /// [`Self::refresh`] with each batch on the blocking pool and a yield
    /// between batches.
    pub async fn refresh_async(
        self: &Arc<Self>,
        cfg: &LocalAssistantConfig,
    ) -> Result<RefreshStats> {
        let (batches, mut stats, head) = {
            let me = Arc::clone(self);
            let cfg = cfg.clone();
            tokio::task::spawn_blocking(move || me.plan_work(&cfg))
                .await
                .map_err(|e| AssistantError::Io(e.to_string()))??
        };
        for batch in batches {
            let me = Arc::clone(self);
            let cfg = cfg.clone();
            let part = tokio::task::spawn_blocking(move || me.apply_batch(batch, &cfg))
                .await
                .map_err(|e| AssistantError::Io(e.to_string()))??;
            merge(&mut stats, &part);
            tokio::task::yield_now().await;
        }
        self.finish_refresh(&stats, head)?;
        Ok(stats)
    }

    /// Stat walk: decide what to re-read and what to delete. Reads no file
    /// content.
    fn plan_work(
        &self,
        cfg: &LocalAssistantConfig,
    ) -> Result<(Vec<Vec<Work>>, RefreshStats, Option<String>)> {
        let started = std::time::Instant::now();
        let (rows, skips) = {
            let db = self.db.lock();
            (db.file_rows()?, db.skip_rows()?)
        };
        let candidates = list_candidates(&self.root)?;
        let head = git_head(&self.root);
        let exclude = ExcludeSet::new(&cfg.exclude_globs);
        let mut stats = RefreshStats {
            listed: candidates.len(),
            ..RefreshStats::default()
        };
        let mut keep = std::collections::HashSet::with_capacity(candidates.len());
        let mut items = Vec::new();
        let mut kept_bytes = 0u64;
        for rel in candidates {
            if is_generated_path(&rel) {
                stats.skipped_generated += 1;
                continue;
            }
            if exclude.matches(&rel) {
                stats.skipped_ignored += 1;
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(self.root.join(&rel)) else {
                continue;
            };
            // Symlinks are never followed: a link can point outside the root.
            if !meta.is_file() {
                continue;
            }
            let size = meta.len();
            if size > cfg.index_max_file_bytes {
                stats.skipped_large += 1;
                continue;
            }
            if keep.len() >= cfg.index_max_files || kept_bytes + size > cfg.index_max_bytes {
                stats.skipped_cap += 1;
                continue;
            }
            kept_bytes += size;
            let mtime_ns = mtime_ns_of(&meta);
            keep.insert(rel.clone());
            let row = rows.get(&rel);
            if row.is_some_and(|r| r.size == size && r.mtime_ns == mtime_ns) {
                continue;
            }
            if skips.get(&rel) == Some(&(size, mtime_ns)) {
                continue;
            }
            items.push(PlanItem {
                rel,
                size,
                mtime_ns,
                existing: row.map(|r| r.id),
                force: false,
            });
        }
        stats.bytes = kept_bytes;
        let mut work: Vec<Work> = Vec::new();
        for (path, row) in &rows {
            if !keep.contains(path) {
                work.push(Work::RemoveFile(row.id));
            }
        }
        for path in skips.keys() {
            if !keep.contains(path) {
                work.push(Work::RemoveSkip(path.clone()));
            }
        }
        work.extend(items.into_iter().map(Work::Item));
        let batch = cfg.index_batch_files.max(1);
        let mut batches = Vec::new();
        let mut iter = work.into_iter().peekable();
        while iter.peek().is_some() {
            batches.push(iter.by_ref().take(batch).collect::<Vec<_>>());
        }
        log::debug!(
            "[local_assistant:index] plan: listed={} work_batches={} in {}ms",
            stats.listed,
            batches.len(),
            started.elapsed().as_millis()
        );
        Ok((batches, stats, head))
    }

    /// One transaction. Returns the counters this batch contributed.
    fn apply_batch(&self, batch: Vec<Work>, cfg: &LocalAssistantConfig) -> Result<RefreshStats> {
        let mut stats = RefreshStats {
            batches: 1,
            ..RefreshStats::default()
        };
        let mut db = self.db.lock();
        let tx = db.transaction()?;
        for work in batch {
            match work {
                Work::RemoveFile(id) => {
                    store::delete_file(&tx, id)?;
                    stats.removed += 1;
                }
                Work::RemoveSkip(path) => store::delete_skip(&tx, &path)?,
                Work::Item(item) => self.apply_item(&tx, &item, cfg, &mut stats)?,
            }
        }
        tx.commit()?;
        Ok(stats)
    }

    fn apply_item(
        &self,
        tx: &rusqlite::Transaction<'_>,
        item: &PlanItem,
        cfg: &LocalAssistantConfig,
        stats: &mut RefreshStats,
    ) -> Result<()> {
        stats.reread += 1;
        let drop_existing =
            |tx: &rusqlite::Transaction<'_>, stats: &mut RefreshStats| -> Result<()> {
                if let Some(id) = item.existing {
                    store::delete_file(tx, id)?;
                    stats.removed += 1;
                }
                Ok(())
            };
        let bytes = match std::fs::read(self.root.join(&item.rel)) {
            Ok(bytes) => bytes,
            Err(_) => return drop_existing(tx, stats),
        };
        if bytes.len() as u64 > cfg.index_max_file_bytes {
            stats.skipped_large += 1;
            return drop_existing(tx, stats);
        }
        if looks_binary(&item.rel, &bytes) {
            stats.skipped_binary += 1;
            store::upsert_skip(tx, &item.rel, item.size, item.mtime_ns)?;
            return drop_existing(tx, stats);
        }
        if is_generated(&item.rel, &bytes) {
            stats.skipped_generated += 1;
            store::upsert_skip(tx, &item.rel, item.size, item.mtime_ns)?;
            return drop_existing(tx, stats);
        }
        let Ok(text) = String::from_utf8(bytes) else {
            stats.skipped_binary += 1;
            store::upsert_skip(tx, &item.rel, item.size, item.mtime_ns)?;
            return drop_existing(tx, stats);
        };
        store::delete_skip(tx, &item.rel)?;
        let sha = sha_hex(text.as_bytes());
        let row = match item.existing {
            Some(id) => self.db_sha(tx, id)?,
            None => None,
        };
        if let (Some(id), Some(old)) = (item.existing, row) {
            if old == sha && !item.force {
                store::touch_file(tx, id, item.size, item.mtime_ns)?;
                return Ok(());
            }
        }
        let lang = Lang::from_path(&item.rel);
        let chunks = chunk_text(&text);
        let symbols = extract_symbols(lang, &text);
        store::upsert_file(
            tx,
            item.existing,
            &item.rel,
            item.size,
            item.mtime_ns,
            &sha,
            lang.as_str(),
            &chunks,
            &symbols,
        )?;
        stats.changed += 1;
        Ok(())
    }

    fn db_sha(&self, tx: &rusqlite::Transaction<'_>, id: i64) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        Ok(tx
            .query_row("SELECT sha FROM files WHERE id=?1", [id], |r| r.get(0))
            .optional()?)
    }

    fn finish_refresh(&self, stats: &RefreshStats, head: Option<String>) -> Result<()> {
        if stats.changed > 0 || stats.removed > 0 {
            self.cache.lock().clear();
        }
        let db = self.db.lock();
        db.set_meta("last_refresh_ms", &now_ms().to_string())?;
        if let Some(head) = head {
            db.set_meta("git_head", &head)?;
        }
        log::debug!(
            "[local_assistant:index] refresh done: listed={} reread={} changed={} removed={} \
             skipped(gen={} ign={} bin={} large={} cap={}) batches={}",
            stats.listed,
            stats.reread,
            stats.changed,
            stats.removed,
            stats.skipped_generated,
            stats.skipped_ignored,
            stats.skipped_binary,
            stats.skipped_large,
            stats.skipped_cap,
            stats.batches
        );
        Ok(())
    }

    /// Re-read one file now, outside a refresh. Used when retrieval finds the
    /// disk no longer matches the index.
    pub(super) fn reindex_file(&self, rel: &str, cfg: &LocalAssistantConfig) -> Result<()> {
        let meta = std::fs::symlink_metadata(self.root.join(rel));
        let existing = self.db.lock().file_row(rel)?.map(|r| r.id);
        let work = match meta {
            Ok(meta) if meta.is_file() => Work::Item(PlanItem {
                rel: rel.to_string(),
                size: meta.len(),
                mtime_ns: mtime_ns_of(&meta),
                existing,
                force: true,
            }),
            _ => match existing {
                Some(id) => Work::RemoveFile(id),
                None => return Ok(()),
            },
        };
        let stats = self.apply_batch(vec![work], cfg)?;
        if stats.changed > 0 || stats.removed > 0 {
            self.cache.lock().clear();
        }
        Ok(())
    }

    pub fn status(&self) -> Result<IndexStatus> {
        let db = self.db.lock();
        let counts = db.counts()?;
        let wal = PathBuf::from(format!("{}-wal", self.db_path.display()));
        let db_bytes = [self.db_path.clone(), wal]
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        Ok(IndexStatus {
            root: self.root.to_string_lossy().into_owned(),
            files: counts.files,
            chunks: counts.chunks,
            symbols: counts.symbols,
            db_bytes,
            last_refresh_ms: db.get_meta("last_refresh_ms")?.and_then(|v| v.parse().ok()),
            git_head: db.get_meta("git_head")?,
        })
    }

    /// Drop the query cache. Called when a task ends.
    pub fn clear_cache(&self) {
        self.cache.lock().clear();
    }
}

fn merge(total: &mut RefreshStats, part: &RefreshStats) {
    total.reread += part.reread;
    total.changed += part.changed;
    total.removed += part.removed;
    total.skipped_generated += part.skipped_generated;
    total.skipped_binary += part.skipped_binary;
    total.skipped_large += part.skipped_large;
    total.batches += part.batches;
}

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod refresh_tests;
#[cfg(test)]
#[path = "search_tests.rs"]
mod search_tests;
