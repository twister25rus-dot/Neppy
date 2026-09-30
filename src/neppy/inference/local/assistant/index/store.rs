//! SQLite layer of the project index.
//!
//! The database holds tokens and coordinates, never file text: `chunk_fts` is
//! contentless, and a hit is turned back into text by reading the file. The
//! project stays on disk, and the index stays small enough to rebuild.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::super::types::Result;
use super::symbols::Symbol;

/// Bumped when the schema changes. The index is rebuildable, so a mismatch
/// drops it rather than migrating.
const SCHEMA_VERSION: &str = "1";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL,
    sha TEXT NOT NULL,
    lang TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY,
    file_id INTEGER NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    sha TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS chunks_file ON chunks(file_id);
CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    file_id INTEGER NOT NULL,
    line INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name COLLATE NOCASE);
CREATE INDEX IF NOT EXISTS symbols_file ON symbols(file_id);
CREATE TABLE IF NOT EXISTS skips (
    path TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    mtime_ns INTEGER NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS chunk_fts USING fts5(
    body, content='', contentless_delete=1, tokenize=\"unicode61 tokenchars '_'\"
);
CREATE VIRTUAL TABLE IF NOT EXISTS path_fts USING fts5(
    segments, content='', contentless_delete=1, tokenize=\"unicode61 tokenchars '_'\"
);
";

#[derive(Debug, Clone)]
pub(crate) struct FileRow {
    pub(crate) id: i64,
    pub(crate) size: u64,
    pub(crate) mtime_ns: i64,
    pub(crate) sha: String,
}

pub(crate) struct ChunkRec {
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) sha: String,
    /// Indexed text. Not stored.
    pub(crate) text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) files: u64,
    pub(crate) chunks: u64,
    pub(crate) symbols: u64,
}

pub(crate) struct IndexDb {
    conn: Connection,
}

/// `src/neppy/mod.rs` -> `src neppy mod rs`, so a path is searchable by any of
/// its segments.
pub(crate) fn path_segments(path: &str) -> String {
    path.split(['/', '.', '-', '\\'])
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

impl IndexDb {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        // WAL keeps a reader (the prompt builder) from blocking a batch write.
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        let db = Self { conn };
        db.ensure_schema()?;
        Ok(db)
    }

    fn ensure_schema(&self) -> Result<()> {
        let has_meta: bool = self
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='meta'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)?;
        if has_meta {
            let version = self.get_meta("schema_version")?;
            if version.as_deref() != Some(SCHEMA_VERSION) {
                log::info!("[local_assistant:index] schema changed; rebuilding the index");
                self.conn.execute_batch(
                    "DROP TABLE IF EXISTS chunk_fts; DROP TABLE IF EXISTS path_fts;
                     DROP TABLE IF EXISTS symbols; DROP TABLE IF EXISTS chunks;
                     DROP TABLE IF EXISTS files; DROP TABLE IF EXISTS skips;
                     DROP TABLE IF EXISTS meta;",
                )?;
            }
        }
        self.conn.execute_batch(SCHEMA)?;
        self.set_meta("schema_version", SCHEMA_VERSION)?;
        Ok(())
    }

    pub(crate) fn get_meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub(crate) fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key,value) VALUES(?1,?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub(crate) fn file_rows(&self) -> Result<HashMap<String, FileRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, size, mtime_ns, sha FROM files")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(1)?,
                FileRow {
                    id: r.get(0)?,
                    size: r.get::<_, i64>(2)? as u64,
                    mtime_ns: r.get(3)?,
                    sha: r.get(4)?,
                },
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (path, row) = row?;
            out.insert(path, row);
        }
        Ok(out)
    }

    /// Files whose content was inspected and rejected (binary, generated),
    /// with the stat they had, so an unchanged one is not re-read every time.
    pub(crate) fn skip_rows(&self) -> Result<HashMap<String, (u64, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, size, mtime_ns FROM skips")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)?),
            ))
        })?;
        let mut out = HashMap::new();
        for row in rows {
            let (path, stat) = row?;
            out.insert(path, stat);
        }
        Ok(out)
    }

    pub(crate) fn file_row(&self, path: &str) -> Result<Option<FileRow>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, size, mtime_ns, sha FROM files WHERE path=?1",
                [path],
                |r| {
                    Ok(FileRow {
                        id: r.get(0)?,
                        size: r.get::<_, i64>(1)? as u64,
                        mtime_ns: r.get(2)?,
                        sha: r.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub(crate) fn transaction(&mut self) -> Result<Transaction<'_>> {
        Ok(self.conn.transaction()?)
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub(crate) fn counts(&self) -> Result<Counts> {
        let one = |sql: &str| -> Result<u64> {
            Ok(self.conn.query_row(sql, [], |r| r.get::<_, i64>(0))? as u64)
        };
        Ok(Counts {
            files: one("SELECT count(*) FROM files")?,
            chunks: one("SELECT count(*) FROM chunks")?,
            symbols: one("SELECT count(*) FROM symbols")?,
        })
    }

    /// Path of a file row by id.
    pub(crate) fn path_of(&self, id: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT path FROM files WHERE id=?1", [id], |r| r.get(0))
            .optional()?)
    }
}

/// Remove every derived row of a file, keeping (or not) the file row itself.
fn clear_file(tx: &Transaction<'_>, id: i64, drop_row: bool) -> Result<()> {
    tx.execute(
        "DELETE FROM chunk_fts WHERE rowid IN (SELECT id FROM chunks WHERE file_id=?1)",
        [id],
    )?;
    tx.execute("DELETE FROM chunks WHERE file_id=?1", [id])?;
    tx.execute("DELETE FROM symbols WHERE file_id=?1", [id])?;
    tx.execute("DELETE FROM path_fts WHERE rowid=?1", [id])?;
    if drop_row {
        tx.execute("DELETE FROM files WHERE id=?1", [id])?;
    }
    Ok(())
}

pub(crate) fn delete_file(tx: &Transaction<'_>, id: i64) -> Result<()> {
    clear_file(tx, id, true)
}

/// Stat data changed but the content hash did not.
pub(crate) fn touch_file(tx: &Transaction<'_>, id: i64, size: u64, mtime_ns: i64) -> Result<()> {
    tx.execute(
        "UPDATE files SET size=?2, mtime_ns=?3 WHERE id=?1",
        params![id, size as i64, mtime_ns],
    )?;
    Ok(())
}

/// Insert or replace a file with its chunks and symbols.
pub(crate) fn upsert_file(
    tx: &Transaction<'_>,
    existing: Option<i64>,
    path: &str,
    size: u64,
    mtime_ns: i64,
    sha: &str,
    lang: &str,
    chunks: &[ChunkRec],
    symbols: &[Symbol],
) -> Result<i64> {
    let id = match existing {
        Some(id) => {
            clear_file(tx, id, false)?;
            tx.execute(
                "UPDATE files SET size=?2, mtime_ns=?3, sha=?4, lang=?5 WHERE id=?1",
                params![id, size as i64, mtime_ns, sha, lang],
            )?;
            id
        }
        None => {
            tx.execute(
                "INSERT INTO files(path,size,mtime_ns,sha,lang) VALUES(?1,?2,?3,?4,?5)",
                params![path, size as i64, mtime_ns, sha, lang],
            )?;
            tx.last_insert_rowid()
        }
    };
    tx.execute(
        "INSERT INTO path_fts(rowid, segments) VALUES(?1, ?2)",
        params![id, path_segments(path)],
    )?;
    for chunk in chunks {
        tx.execute(
            "INSERT INTO chunks(file_id,start_line,end_line,sha) VALUES(?1,?2,?3,?4)",
            params![id, chunk.start, chunk.end, chunk.sha],
        )?;
        let chunk_id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO chunk_fts(rowid, body) VALUES(?1, ?2)",
            params![chunk_id, chunk.text],
        )?;
    }
    for symbol in symbols {
        tx.execute(
            "INSERT INTO symbols(name,kind,file_id,line) VALUES(?1,?2,?3,?4)",
            params![symbol.name, symbol.kind, id, symbol.line],
        )?;
    }
    Ok(id)
}

pub(crate) fn upsert_skip(
    tx: &Transaction<'_>,
    path: &str,
    size: u64,
    mtime_ns: i64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO skips(path,size,mtime_ns) VALUES(?1,?2,?3)
         ON CONFLICT(path) DO UPDATE SET size=excluded.size, mtime_ns=excluded.mtime_ns",
        params![path, size as i64, mtime_ns],
    )?;
    Ok(())
}

pub(crate) fn delete_skip(tx: &Transaction<'_>, path: &str) -> Result<()> {
    tx.execute("DELETE FROM skips WHERE path=?1", [path])?;
    Ok(())
}
