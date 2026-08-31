//! Unit tests for deployment bundles.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn normalizes_a_relative_path() {
    let file = SiteFile::new("./app/page.tsx", b"x".to_vec()).unwrap();

    assert_eq!(file.path, "app/page.tsx");
    assert_eq!(file.len(), 1);
    assert!(!file.is_empty());
}

#[test]
fn converts_windows_separators() {
    let file = SiteFile::new(r"app\routes\page.tsx", Vec::new()).unwrap();

    assert_eq!(file.path, "app/routes/page.tsx");
    assert!(file.is_empty());
}

#[test]
fn rejects_a_path_outside_the_bundle() {
    for path in ["", "   ", "/etc/passwd", "../secrets.env", "app/../../x"] {
        let error = SiteFile::new(path, Vec::new()).unwrap_err();
        assert!(
            matches!(error, Error::InvalidBundlePath { .. }),
            "{path} produced {error:?}"
        );
    }
}

#[test]
fn debug_prints_the_size_and_never_the_bytes() {
    let file = SiteFile::new("secret.env", b"TOKEN=hunter2".to_vec()).unwrap();
    let rendered = format!("{file:?}");

    assert!(!rendered.contains("hunter2"), "{rendered}");
    assert!(rendered.contains("bytes: 13"), "{rendered}");
}

#[test]
fn insert_replaces_a_file_at_the_same_path() {
    let mut bundle = Bundle::new();
    bundle.insert("package.json", b"{}".to_vec()).unwrap();
    bundle
        .insert("package.json", b"{\"a\":1}".to_vec())
        .unwrap();

    assert_eq!(bundle.len(), 1);
    assert_eq!(bundle.files()[0].contents, b"{\"a\":1}");
    assert_eq!(bundle.total_bytes(), 7);
}

#[test]
fn insert_rejects_an_invalid_path() {
    let mut bundle = Bundle::new();

    assert!(matches!(
        bundle.insert("../x", Vec::new()).unwrap_err(),
        Error::InvalidBundlePath { .. }
    ));
}

#[test]
fn an_empty_bundle_reports_itself_empty() {
    let bundle = Bundle::default();

    assert!(bundle.is_empty());
    assert_eq!(bundle.len(), 0);
    assert_eq!(bundle.total_bytes(), 0);
    assert!(bundle.files().is_empty());
}

#[test]
fn from_files_keeps_what_it_was_given() {
    let bundle = Bundle::from_files(vec![SiteFile::new("a.txt", b"a".to_vec()).unwrap()]);

    assert_eq!(bundle.len(), 1);
}

#[test]
fn reads_a_directory_tree_and_skips_build_output() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();

    std::fs::write(path.join("package.json"), b"{}").unwrap();
    std::fs::create_dir(path.join("app")).unwrap();
    std::fs::write(path.join("app/page.tsx"), b"page").unwrap();
    std::fs::create_dir(path.join("node_modules")).unwrap();
    std::fs::write(path.join("node_modules/huge.js"), b"nope").unwrap();
    std::fs::create_dir(path.join(".next")).unwrap();
    std::fs::write(path.join(".next/build"), b"nope").unwrap();
    std::fs::write(path.join(".env"), b"TOKEN=nope").unwrap();

    let bundle = Bundle::from_dir(path).unwrap();
    let mut paths: Vec<&str> = bundle
        .files()
        .iter()
        .map(|file| file.path.as_str())
        .collect();
    paths.sort_unstable();

    assert_eq!(paths, ["app/page.tsx", "package.json"]);
}

#[test]
fn an_empty_directory_is_not_a_deployment() {
    let root = tempfile::tempdir().unwrap();

    assert_eq!(
        Bundle::from_dir(root.path()).unwrap_err(),
        Error::EmptyBundle
    );
}

#[test]
fn a_missing_directory_reports_what_could_not_be_read() {
    let error = Bundle::from_dir("./does-not-exist-anywhere").unwrap_err();

    assert!(
        matches!(error, Error::ReadBundle { .. }),
        "unexpected error: {error:?}"
    );
}

#[cfg(unix)]
#[test]
fn skips_symbolic_links() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path();

    std::fs::write(path.join("real.txt"), b"real").unwrap();
    std::os::unix::fs::symlink("/etc/hostname", path.join("link.txt")).unwrap();

    let bundle = Bundle::from_dir(path).unwrap();

    assert_eq!(bundle.len(), 1);
    assert_eq!(bundle.files()[0].path, "real.txt");
}

#[cfg(unix)]
#[test]
fn a_directory_that_cannot_be_read_reports_which_one() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("package.json"), b"{}").unwrap();
    let blocked = root.path().join("blocked");
    std::fs::create_dir(&blocked).unwrap();
    std::fs::write(blocked.join("inner.txt"), b"inner").unwrap();
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();

    let result = Bundle::from_dir(root.path());

    // Root ignores the mode bits, so the unreadable case cannot be forced there.
    match result {
        Err(error) => assert!(
            matches!(error, Error::ReadBundle { .. }),
            "unexpected error: {error:?}"
        ),
        Ok(bundle) => assert_eq!(bundle.len(), 2, "only root can read a 0o000 directory"),
    }

    // Leave the tree removable.
    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn contents_travel_as_base64() {
    let mut bundle = Bundle::new();
    bundle.insert("app/page.tsx", b"hello".to_vec()).unwrap();

    let json = serde_json::to_string(&bundle).unwrap();
    assert!(json.contains("aGVsbG8="), "{json}");

    let restored: Bundle = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, bundle);
}

#[test]
fn deserializing_revalidates_every_path() {
    let error =
        serde_json::from_str::<Bundle>(r#"[{"path":"../escape","contents":"aGk="}]"#).unwrap_err();

    assert!(error.to_string().contains("not a relative path"), "{error}");
}

#[test]
fn deserializing_rejects_contents_that_are_not_base64() {
    let error = serde_json::from_str::<Bundle>(r#"[{"path":"a.txt","contents":"not base64!"}]"#)
        .unwrap_err();

    assert!(!error.to_string().is_empty());
}

#[test]
fn a_bundle_converts_back_into_its_files() {
    let mut bundle = Bundle::new();
    bundle.insert("a.txt", b"a".to_vec()).unwrap();

    let files: Vec<SiteFile> = bundle.into();

    assert_eq!(files.len(), 1);
}
