use std::path::Path;
use std::process::Command;

use super::*;

pub(super) fn cfg() -> LocalAssistantConfig {
    LocalAssistantConfig::default()
}

pub(super) fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

pub(super) fn git_init(root: &Path) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["init", "-q"])
        .status()
        .unwrap()
        .success();
    assert!(ok, "git init");
}

/// A project with one file of each kind the index must treat differently.
pub(super) fn project() -> (tempfile::TempDir, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    git_init(root.path());
    write(root.path(), ".gitignore", "ignored.rs\n");
    write(root.path(), "src/lib.rs", "pub fn foo_bar() {}\n");
    write(root.path(), "src/other.rs", "pub struct Widget;\n");
    write(root.path(), "ignored.rs", "fn hidden_one() {}\n");
    write(root.path(), "target/x.rs", "fn in_target() {}\n");
    write(root.path(), "gen.rs", "// @generated\nfn machine() {}\n");
    std::fs::write(root.path().join("blob.dat"), b"ab\0cd").unwrap();
    std::fs::write(root.path().join("big.txt"), vec![b'a'; 600 * 1024]).unwrap();
    (root, ws)
}

#[test]
fn ignored_generated_binary_and_large_files_are_not_indexed() {
    let (root, ws) = project();
    let index = open_index(ws.path(), root.path()).unwrap();
    let stats = index.refresh(&cfg()).unwrap();
    let rows = index.db.lock().file_rows().unwrap();
    let mut paths: Vec<_> = rows.keys().cloned().collect();
    paths.sort();
    assert_eq!(paths, vec![".gitignore", "src/lib.rs", "src/other.rs"]);
    assert_eq!(stats.changed, 3);
    assert_eq!(stats.skipped_large, 1, "{stats:?}");
    assert!(stats.skipped_generated >= 2, "{stats:?}");
    assert_eq!(stats.skipped_binary, 1, "{stats:?}");
}

#[test]
fn a_second_refresh_reads_nothing_and_a_modified_file_reads_exactly_one() {
    let (root, ws) = project();
    let index = open_index(ws.path(), root.path()).unwrap();
    index.refresh(&cfg()).unwrap();

    let idle = index.refresh(&cfg()).unwrap();
    assert_eq!(
        idle.reread, 0,
        "unchanged project must not be re-read: {idle:?}"
    );
    assert_eq!(idle.changed, 0);

    write(
        root.path(),
        "src/lib.rs",
        "pub fn foo_bar() {}\npub fn added() {}\n",
    );
    let after = index.refresh(&cfg()).unwrap();
    assert_eq!(after.reread, 1);
    assert_eq!(after.changed, 1);
    let symbols = index.db.lock().counts().unwrap().symbols;
    assert_eq!(symbols, 3, "foo_bar, added, Widget");
}

#[test]
fn touching_without_changing_content_rereads_but_does_not_reindex() {
    let (root, ws) = project();
    let index = open_index(ws.path(), root.path()).unwrap();
    index.refresh(&cfg()).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    write(root.path(), "src/lib.rs", "pub fn foo_bar() {}\n");
    let stats = index.refresh(&cfg()).unwrap();
    assert_eq!(stats.reread, 1);
    assert_eq!(stats.changed, 0, "same hash, no re-index");
}

#[test]
fn deleting_a_file_removes_its_rows_everywhere() {
    let (root, ws) = project();
    let index = open_index(ws.path(), root.path()).unwrap();
    index.refresh(&cfg()).unwrap();
    std::fs::remove_file(root.path().join("src/other.rs")).unwrap();
    let stats = index.refresh(&cfg()).unwrap();
    assert_eq!(stats.removed, 1);
    let counts = index.db.lock().counts().unwrap();
    assert_eq!(counts.files, 2);
    let leftover: i64 = index
        .db
        .lock()
        .conn()
        .query_row(
            "SELECT count(*) FROM symbols WHERE name='Widget'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leftover, 0);
    let fts_rows: i64 = index
        .db
        .lock()
        .conn()
        .query_row("SELECT count(*) FROM chunk_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_rows as u64, counts.chunks, "no orphaned fts rows");
}

#[test]
fn batches_bound_the_work_per_transaction() {
    let (root, ws) = project();
    let index = open_index(ws.path(), root.path()).unwrap();
    let mut config = cfg();
    config.index_batch_files = 2;
    let stats = index.refresh(&config).unwrap();
    assert!(stats.batches >= 2, "{stats:?}");
    assert_eq!(index.db.lock().counts().unwrap().files, 3);
}

#[tokio::test]
async fn the_async_refresh_matches_the_sync_one() {
    let (root, ws) = project();
    let index = Arc::new(open_index(ws.path(), root.path()).unwrap());
    let mut config = cfg();
    config.index_batch_files = 1;
    let stats = index.refresh_async(&config).await.unwrap();
    assert_eq!(stats.changed, 3);
    assert!(stats.batches >= 3);
    let again = index.refresh_async(&config).await.unwrap();
    assert_eq!(again.reread, 0);
}

#[test]
fn a_non_git_directory_is_indexed_through_the_walk_fallback() {
    let root = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    write(root.path(), "a/one.rs", "fn one() {}\n");
    write(root.path(), "node_modules/dep/x.js", "function dep() {}\n");
    let index = open_index(ws.path(), root.path()).unwrap();
    let stats = index.refresh(&cfg()).unwrap();
    assert_eq!(stats.changed, 1);
    assert_eq!(index.status().unwrap().files, 1);
}

#[test]
fn caps_and_exclude_globs_keep_files_out() {
    let (root, ws) = project();
    let index = open_index(ws.path(), root.path()).unwrap();
    let mut config = cfg();
    config.index_max_files = 1;
    let stats = index.refresh(&config).unwrap();
    assert_eq!(stats.changed, 1);
    assert!(stats.skipped_cap >= 2, "{stats:?}");

    write(root.path(), "vendor/a/vendor/deep.rs", "fn deep() {}\n");
    write(root.path(), "vendor/a/top.rs", "fn top() {}\n");
    let mut config = cfg();
    config.index_max_files = 100;
    let stats = index.refresh(&config).unwrap();
    assert_eq!(
        stats.skipped_ignored, 1,
        "nested vendor tree excluded: {stats:?}"
    );
    let rows = index.db.lock().file_rows().unwrap();
    assert!(rows.contains_key("vendor/a/top.rs"));
    assert!(!rows.contains_key("vendor/a/vendor/deep.rs"));
}

#[test]
fn symlinks_are_never_indexed() {
    let (root, ws) = project();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "secret.rs", "fn secret_fn() {}\n");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            outside.path().join("secret.rs"),
            root.path().join("src/link.rs"),
        )
        .unwrap();
        let index = open_index(ws.path(), root.path()).unwrap();
        index.refresh(&cfg()).unwrap();
        let rows = index.db.lock().file_rows().unwrap();
        assert!(!rows.contains_key("src/link.rs"));
    }
}

#[test]
fn chunks_respect_the_line_and_byte_bounds() {
    let many_lines = "x\n".repeat(200);
    let chunks = chunk_text(&many_lines);
    assert_eq!(chunks.len(), 3);
    assert_eq!((chunks[0].start, chunks[0].end), (1, 80));
    assert_eq!((chunks[2].start, chunks[2].end), (161, 200));

    let wide = format!("{}\n", "y".repeat(1500)).repeat(6);
    let chunks = chunk_text(&wide);
    assert!(chunks.iter().all(|c| c.text.len() <= CHUNK_BYTES));
    assert!(chunks.len() >= 2);
    assert!(chunk_text("").is_empty());
}
