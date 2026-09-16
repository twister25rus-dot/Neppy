use super::*;
use crate::neppy::memory::sources::types::{MemorySourceEntry, SourceKind};

/// Built through serde rather than a struct literal: `MemorySourceEntry`
/// flattens every kind's fields onto one struct and implements no `Default`,
/// so this is both shorter and immune to a new optional field being added.
fn folder(id: &str, path: Option<&str>) -> MemorySourceEntry {
    let mut value = serde_json::json!({
        "id": id,
        "kind": "folder",
        "label": "Notes",
        "enabled": true,
    });
    if let Some(path) = path {
        value["path"] = serde_json::Value::String(path.to_string());
    }
    serde_json::from_value(value).expect("fixture deserializes")
}

fn config_with(sources: Vec<MemorySourceEntry>) -> Config {
    Config {
        memory_sources: sources,
        ..Default::default()
    }
}

#[test]
fn an_absolute_path_is_left_exactly_as_it_was() {
    let mut config = config_with(vec![folder("src_a", Some("/Users/someone/Notes"))]);

    let stats = run(&mut config).expect("migration runs");

    assert_eq!(
        stats,
        Stats::default(),
        "nothing to repair, nothing unresolved"
    );
    assert_eq!(
        config.memory_sources[0].path.as_deref(),
        Some("/Users/someone/Notes")
    );
}

#[test]
fn an_unresolvable_name_keeps_its_value_and_is_counted() {
    // The no-match branch is the one that matters most: replacing this with a
    // path that also does not exist would trade a legible error for a puzzling
    // one, so the original must survive.
    let name = "a folder that exists nowhere 4b81f2";
    let mut config = config_with(vec![folder("src_b", Some(name))]);

    let stats = run(&mut config).expect("migration runs");

    assert_eq!(stats.repaired, 0);
    assert_eq!(stats.unresolved, 1);
    assert_eq!(config.memory_sources[0].path.as_deref(), Some(name));
}

#[test]
fn non_folder_sources_are_untouched() {
    // `path` on a feed or repo source means something else; rewriting it as a
    // filesystem path would corrupt a working source.
    let mut source = folder("src_c", Some("some-name"));
    source.kind = SourceKind::RssFeed;
    let mut config = config_with(vec![source]);

    let stats = run(&mut config).expect("migration runs");

    assert_eq!(stats, Stats::default());
    assert_eq!(config.memory_sources[0].path.as_deref(), Some("some-name"));
}

#[test]
fn a_source_without_a_path_is_skipped() {
    let mut config = config_with(vec![folder("src_d", None)]);

    let stats = run(&mut config).expect("migration runs");

    assert_eq!(stats, Stats::default());
    assert!(config.memory_sources[0].path.is_none());
}

#[test]
fn an_empty_path_is_not_counted_as_unresolvable() {
    // Empty is "not configured yet", not "broken", and the Add dialog already
    // refuses to save it.
    let mut config = config_with(vec![folder("src_e", Some("   "))]);

    let stats = run(&mut config).expect("migration runs");

    assert_eq!(stats.unresolved, 0);
}

#[test]
fn the_real_broken_shape_resolves_when_the_folder_is_present() {
    // Reproduces the exact value the broken picker wrote. Skipped rather than
    // failed on a machine without that vault, so this is a real assertion where
    // it can be one and never a false failure in CI.
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let vault = home.join("Library/Mobile Documents/iCloud~md~obsidian/Documents/AI Memory Hub");
    if !vault.is_dir() {
        return;
    }

    let mut config = config_with(vec![folder("src_f", Some("AI Memory Hub"))]);
    let stats = run(&mut config).expect("migration runs");

    assert_eq!(stats.repaired, 1);
    assert_eq!(
        config.memory_sources[0].path.as_deref(),
        Some(vault.display().to_string().as_str()),
        "the name must resolve to Obsidian's own copy, not a duplicate elsewhere"
    );
}
