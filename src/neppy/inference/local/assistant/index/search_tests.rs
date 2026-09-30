use super::refresh_tests::{cfg, project, write};
use super::*;

fn terms(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

fn indexed() -> (tempfile::TempDir, tempfile::TempDir, ProjectIndex) {
    let (root, ws) = project();
    let index = open_index(ws.path(), root.path()).unwrap();
    index.refresh(&cfg()).unwrap();
    (root, ws, index)
}

#[test]
fn a_symbol_is_found_by_exact_name_and_by_prefix() {
    let (_r, _w, index) = indexed();
    let exact = index.search(&terms(&["foo_bar"]), &cfg()).unwrap();
    assert_eq!(exact[0].path, "src/lib.rs");
    assert!(exact[0].why.starts_with("symbol:"), "{}", exact[0].why);
    assert!(exact[0].text.contains("fn foo_bar"));

    let prefix = index.search(&terms(&["foo_b"]), &cfg()).unwrap();
    assert_eq!(prefix[0].path, "src/lib.rs");
    assert!(prefix[0].why.starts_with("symbol~"), "{}", prefix[0].why);
}

#[test]
fn filenames_and_content_are_searched() {
    let (root, _w, index) = indexed();
    let by_name = index.search(&terms(&["other"]), &cfg()).unwrap();
    assert!(by_name
        .iter()
        .any(|s| s.path == "src/other.rs" && s.why == "filename"));

    write(
        root.path(),
        "notes.md",
        "the quarantined_token lives only in prose here\n",
    );
    index.refresh(&cfg()).unwrap();
    let by_content = index
        .search(&terms(&["quarantined_token"]), &cfg())
        .unwrap();
    assert_eq!(by_content[0].path, "notes.md");
    assert_eq!(by_content[0].why, "content");
}

#[test]
fn a_file_appears_once_and_ignored_files_never_appear() {
    let (_r, _w, index) = indexed();
    let hits = index
        .search(&terms(&["foo_bar", "lib", "foo_b", "pub"]), &cfg())
        .unwrap();
    let lib: Vec<_> = hits.iter().filter(|s| s.path == "src/lib.rs").collect();
    assert_eq!(lib.len(), 1);
    assert!(hits
        .iter()
        .all(|s| s.path != "ignored.rs" && s.path != "target/x.rs"));
    assert!(index
        .search(&terms(&["hidden_one"]), &cfg())
        .unwrap()
        .is_empty());
}

#[test]
fn snippets_are_bounded_by_count_lines_and_characters() {
    let root = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    for n in 0..30 {
        let body = format!(
            "fn shared_token_{n}() {{}}\n{}",
            "let shared_token = 1;\n".repeat(400)
        );
        write(root.path(), &format!("f{n}.rs"), &body);
    }
    let index = open_index(ws.path(), root.path()).unwrap();
    index.refresh(&cfg()).unwrap();
    let mut config = cfg();
    config.max_snippets = 5;
    config.snippet_budget_chars = 3_000;
    let hits = index.search(&terms(&["shared_token"]), &config).unwrap();
    assert!(hits.len() <= 5);
    let total: usize = hits.iter().map(|s| s.text.len()).sum();
    assert!(total <= 3_000, "total {total}");
    assert!(hits.iter().all(|s| s.end - s.start < 120));
}

#[test]
fn a_chunk_hash_mismatch_marks_the_file_dirty_and_reindexes_it() {
    let root = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    write(root.path(), "a.md", "the zebra_marker paragraph\n");
    let index = open_index(ws.path(), root.path()).unwrap();
    index.refresh(&cfg()).unwrap();
    assert_eq!(
        index
            .search(&terms(&["zebra_marker"]), &cfg())
            .unwrap()
            .len(),
        1
    );

    index.cache.lock().clear();
    index.corrupt_chunk_sha_for_test("a.md");
    let stale = index.search(&terms(&["zebra_marker"]), &cfg()).unwrap();
    assert!(stale.is_empty(), "a stale hit is dropped, not shown");

    let again = index.search(&terms(&["zebra_marker"]), &cfg()).unwrap();
    assert_eq!(again.len(), 1, "the file was re-indexed");
}

#[test]
fn an_edit_made_behind_the_index_is_never_shown_stale() {
    let (root, _w, index) = indexed();
    write(root.path(), "src/lib.rs", "pub fn renamed_thing() {}\n");
    let stale = index.search(&terms(&["foo_bar"]), &cfg()).unwrap();
    assert!(stale.iter().all(|s| !s.text.contains("foo_bar")));
    let fresh = index.search(&terms(&["renamed_thing"]), &cfg()).unwrap();
    assert_eq!(fresh[0].path, "src/lib.rs");
}

#[test]
fn the_query_cache_is_bounded_and_cleared_by_changes() {
    let (root, _w, index) = indexed();
    for n in 0..100 {
        index
            .search(&terms(&[&format!("term{n}x")]), &cfg())
            .unwrap();
    }
    assert!(index.cache.lock().len() <= 64);
    index.search(&terms(&["foo_bar"]), &cfg()).unwrap();
    assert!(index.cache.lock().len() > 0);
    write(root.path(), "src/new.rs", "fn fresh_fn() {}\n");
    index.refresh(&cfg()).unwrap();
    assert_eq!(index.cache.lock().len(), 0);
    index.search(&terms(&["foo_bar"]), &cfg()).unwrap();
    index.clear_cache();
    assert_eq!(index.cache.lock().len(), 0);
}

#[test]
fn query_terms_keep_identifiers_and_drop_noise() {
    let got = search::query_terms(&["Find the spawn_worker fn and add a NOTE to each"]);
    assert_eq!(got, vec!["spawn_worker", "fn", "note"]);
    let many: String = (0..40).map(|n| format!("word{n} ")).collect();
    assert_eq!(search::query_terms(&[&many]).len(), search::MAX_TERMS);
    assert!(search::query_terms(&["a ! ?"]).is_empty());
}
