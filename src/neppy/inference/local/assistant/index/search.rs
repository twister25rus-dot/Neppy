//! Retrieval: pick the few pieces of the project relevant to one step.
//!
//! Ranking is symbols first (an exact or prefix name match is the strongest
//! signal a small model's query can give), then filenames, then content by
//! bm25. A file appears at most once. The result is capped by snippet count,
//! per-snippet lines and a character budget, so the prompt never depends on
//! how large the project is.
//!
//! Text is read back from disk and checked against what was indexed. If the
//! disk moved on, the file is marked for re-indexing and dropped from this
//! result rather than shown stale.

use std::collections::{HashMap, VecDeque};

use crate::neppy::config::schema::LocalAssistantConfig;

use super::super::types::{Result, Snippet};
use super::{line_slices, mtime_ns_of, sha_hex, ProjectIndex};

/// Terms a query may carry.
pub(crate) const MAX_TERMS: usize = 12;
const MAX_TERM_CHARS: usize = 64;
/// Longest window returned for a symbol or filename hit, lines. Chunk hits are
/// at most one chunk (80 lines); nothing exceeds [`MAX_SNIPPET_LINES`].
const SYMBOL_WINDOW_LINES: u32 = 40;
const MAX_SNIPPET_LINES: u32 = 120;
const CACHE_CAP: usize = 64;
/// Below this many remaining characters a final snippet is not worth adding.
const MIN_USEFUL_CHARS: usize = 200;

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "that", "this", "with", "from", "into", "each", "every", "all", "any",
    "are", "was", "has", "have", "not", "but", "its", "you", "your", "one", "line", "add", "find",
    "list", "use", "make", "code", "path", "paths", "file", "files", "to", "of", "in", "on", "is",
    "it", "as", "at", "by", "or", "be", "an", "do",
];

/// Identifiers and words worth searching for, in the order given, deduplicated
/// and capped at [`MAX_TERMS`].
pub(crate) fn query_terms(texts: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for text in texts {
        let mut current = String::new();
        let flush = |current: &mut String, out: &mut Vec<String>| {
            if current.len() >= 2 {
                let term = current.to_ascii_lowercase();
                let term: String = term.chars().take(MAX_TERM_CHARS).collect();
                if !STOPWORDS.contains(&term.as_str()) && !out.contains(&term) {
                    out.push(term);
                }
            }
            current.clear();
        };
        for ch in text.chars() {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                current.push(ch);
            } else {
                flush(&mut current, &mut out);
            }
        }
        flush(&mut current, &mut out);
        if out.len() >= MAX_TERMS {
            break;
        }
    }
    out.truncate(MAX_TERMS);
    out
}

#[derive(Default)]
pub(crate) struct QueryCache {
    map: HashMap<String, Vec<Snippet>>,
    order: VecDeque<String>,
}

impl QueryCache {
    fn get(&mut self, key: &str) -> Option<Vec<Snippet>> {
        let hit = self.map.get(key).cloned()?;
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
        Some(hit)
    }

    fn put(&mut self, key: String, value: Vec<Snippet>) {
        if self.map.insert(key.clone(), value).is_some() {
            self.order.retain(|k| *k != key);
        }
        self.order.push_back(key);
        while self.order.len() > CACHE_CAP {
            if let Some(oldest) = self.order.pop_front() {
                self.map.remove(&oldest);
            }
        }
    }

    pub(crate) fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.map.len()
    }
}

struct Cand {
    file_id: i64,
    score: f64,
    hits: u32,
    start: u32,
    end: u32,
    why: String,
    chunk_sha: Option<String>,
}

fn add(cands: &mut HashMap<i64, Cand>, new: Cand) {
    match cands.get_mut(&new.file_id) {
        Some(old) => {
            old.hits += 1;
            if new.score > old.score {
                old.score = new.score;
                old.start = new.start;
                old.end = new.end;
                old.why = new.why;
                old.chunk_sha = new.chunk_sha;
            }
        }
        None => {
            cands.insert(new.file_id, Cand { hits: 1, ..new });
        }
    }
}

fn like_escape(term: &str) -> String {
    term.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn fts_query(terms: &[String]) -> String {
    terms
        .iter()
        .map(|t| format!("\"{t}\"*"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

impl ProjectIndex {
    /// Snippets relevant to `terms`, within the configured count and budget.
    pub fn search(&self, terms: &[String], cfg: &LocalAssistantConfig) -> Result<Vec<Snippet>> {
        let terms = query_terms(&terms.iter().map(String::as_str).collect::<Vec<_>>());
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let key = format!(
            "{}|{}|{}",
            terms.join("\u{1}"),
            cfg.snippet_budget_chars,
            cfg.max_snippets
        );
        if let Some(hit) = self.cache.lock().get(&key) {
            log::debug!(
                "[local_assistant:index] search cache hit terms={}",
                terms.len()
            );
            return Ok(hit);
        }
        let started = std::time::Instant::now();
        let mut ranked = self.candidates(&terms)?;
        ranked.sort_by(|a, b| {
            (b.score + 5.0 * f64::from(b.hits.saturating_sub(1)))
                .partial_cmp(&(a.score + 5.0 * f64::from(a.hits.saturating_sub(1))))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // A few spares: a stale candidate is dropped, not shown.
        ranked.truncate(cfg.max_snippets.saturating_mul(2).max(1));

        let mut out = Vec::new();
        let mut remaining = cfg.snippet_budget_chars;
        let mut stale = Vec::new();
        for cand in ranked {
            if out.len() >= cfg.max_snippets || remaining == 0 {
                break;
            }
            match self.read_candidate(&cand, remaining)? {
                Read::Stale(path) => stale.push(path),
                Read::Gone => {}
                Read::Snippet(snippet) => {
                    remaining = remaining.saturating_sub(snippet.text.len());
                    out.push(snippet);
                    if remaining < MIN_USEFUL_CHARS {
                        break;
                    }
                }
            }
        }
        let had_stale = !stale.is_empty();
        for path in stale {
            log::debug!("[local_assistant:index] stale hit; re-indexing {path}");
            self.reindex_file(&path, cfg)?;
        }
        log::debug!(
            "[local_assistant:index] search terms={} snippets={} chars={} in {}ms",
            terms.len(),
            out.len(),
            out.iter().map(|s| s.text.len()).sum::<usize>(),
            started.elapsed().as_millis()
        );
        // A result that dropped a stale hit is not the answer once the file is
        // re-indexed, so it is not remembered.
        if !had_stale {
            self.cache.lock().put(key, out.clone());
        }
        Ok(out)
    }

    fn candidates(&self, terms: &[String]) -> Result<Vec<Cand>> {
        let db = self.db.lock();
        let conn = db.conn();
        let mut cands: HashMap<i64, Cand> = HashMap::new();

        let mut exact = conn.prepare(
            "SELECT file_id, line, kind, name FROM symbols
             WHERE name = ?1 COLLATE NOCASE LIMIT 30",
        )?;
        let mut prefix = conn.prepare(
            "SELECT file_id, line, kind, name FROM symbols
             WHERE name LIKE ?1 ESCAPE '\\' LIMIT 30",
        )?;
        for term in terms {
            for (stmt, pattern, score, label) in [
                (&mut exact, term.clone(), 1000.0, "symbol"),
                (
                    &mut prefix,
                    format!("{}%", like_escape(term)),
                    600.0,
                    "symbol~",
                ),
            ] {
                let rows = stmt.query_map([pattern], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?;
                for row in rows {
                    let (file_id, line, kind, name) = row?;
                    let start = line.saturating_sub(2).max(1);
                    add(
                        &mut cands,
                        Cand {
                            file_id,
                            score,
                            hits: 0,
                            start,
                            end: start + SYMBOL_WINDOW_LINES - 1,
                            why: format!("{label}:{kind} {name}"),
                            chunk_sha: None,
                        },
                    );
                }
            }
        }

        let mut by_segment = conn
            .prepare("SELECT rowid FROM path_fts WHERE path_fts MATCH ?1 ORDER BY rank LIMIT 20")?;
        let mut by_substring =
            conn.prepare("SELECT id FROM files WHERE path LIKE ?1 ESCAPE '\\' LIMIT 20")?;
        for term in terms {
            let head = |file_id: i64, score: f64| Cand {
                file_id,
                score,
                hits: 0,
                start: 1,
                end: SYMBOL_WINDOW_LINES,
                why: "filename".to_string(),
                chunk_sha: None,
            };
            let rows = by_segment.query_map([format!("\"{term}\"*")], |r| r.get::<_, i64>(0))?;
            for id in rows {
                add(&mut cands, head(id?, 400.0));
            }
            let rows = by_substring
                .query_map([format!("%{}%", like_escape(term))], |r| r.get::<_, i64>(0))?;
            for id in rows {
                add(&mut cands, head(id?, 350.0));
            }
        }

        let mut content = conn.prepare(
            "SELECT c.file_id, c.start_line, c.end_line, c.sha, bm25(chunk_fts)
             FROM chunk_fts JOIN chunks c ON c.id = chunk_fts.rowid
             WHERE chunk_fts MATCH ?1 ORDER BY bm25(chunk_fts) LIMIT 40",
        )?;
        let rows = content.query_map([fts_query(terms)], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, u32>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, f64>(4)?,
            ))
        })?;
        for row in rows {
            let (file_id, start, end, sha, rank) = row?;
            add(
                &mut cands,
                Cand {
                    file_id,
                    score: 200.0 + (-rank).clamp(0.0, 50.0),
                    hits: 0,
                    start,
                    end,
                    why: "content".to_string(),
                    chunk_sha: Some(sha),
                },
            );
        }
        Ok(cands.into_values().collect())
    }

    fn read_candidate(&self, cand: &Cand, remaining: usize) -> Result<Read> {
        let (path, row) = {
            let db = self.db.lock();
            let Some(path) = db.path_of(cand.file_id)? else {
                return Ok(Read::Gone);
            };
            let row = db.file_row(&path)?;
            (path, row)
        };
        let Some(row) = row else {
            return Ok(Read::Gone);
        };
        let abs = self.root.join(&path);
        let Ok(meta) = std::fs::symlink_metadata(&abs) else {
            return Ok(Read::Stale(path));
        };
        if !meta.is_file() || meta.len() != row.size || mtime_ns_of(&meta) != row.mtime_ns {
            return Ok(Read::Stale(path));
        }
        let Ok(text) = std::fs::read_to_string(&abs) else {
            return Ok(Read::Stale(path));
        };
        let lines = line_slices(&text);
        if lines.is_empty() {
            return Ok(Read::Gone);
        }
        let start = cand.start.max(1);
        let end = cand
            .end
            .min(lines.len() as u32)
            .min(start + MAX_SNIPPET_LINES - 1);
        if start > end {
            return Ok(Read::Stale(path));
        }
        let mut body = lines[(start - 1) as usize..end as usize].concat();
        if let Some(expected) = &cand.chunk_sha {
            if cand.end == end && sha_hex(body.as_bytes()) != *expected {
                return Ok(Read::Stale(path));
            }
        }
        let mut end = end;
        if body.len() > remaining {
            let mut cut = remaining;
            while cut > 0 && !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
            // Whole lines only.
            if let Some(nl) = body.rfind('\n') {
                body.truncate(nl + 1);
            }
            end = start + body.matches('\n').count().max(1) as u32 - 1;
        }
        if body.is_empty() {
            return Ok(Read::Gone);
        }
        Ok(Read::Snippet(Snippet {
            path,
            start,
            end,
            text: body,
            why: cand.why.clone(),
        }))
    }

    /// Break one chunk's recorded hash so the next retrieval sees a mismatch.
    #[cfg(test)]
    pub(super) fn corrupt_chunk_sha_for_test(&self, path: &str) {
        use rusqlite::params;
        let db = self.db.lock();
        db.conn()
            .execute(
                "UPDATE chunks SET sha='bad' WHERE file_id=(SELECT id FROM files WHERE path=?1)",
                params![path],
            )
            .expect("corrupt chunk sha");
    }
}

enum Read {
    Snippet(Snippet),
    /// The disk no longer matches the index for this file.
    Stale(String),
    Gone,
}
