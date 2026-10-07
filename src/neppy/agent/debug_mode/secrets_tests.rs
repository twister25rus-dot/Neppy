use super::*;

#[test]
fn secret_paths_follow_the_spec_list() {
    for p in [
        ".env",
        "app/.env.local",
        ".env.production",
        "credentials.json",
        "certs/server.pem",
        "tls.key",
        "home/.ssh/id_rsa",
        "id_ed25519",
        "id_ed25519.pub",
        ".npmrc",
        ".pypirc",
        "store.p12",
        "secrets.yaml",
        "SECRETS.json",
    ] {
        assert!(is_secret_path(p), "{p} must be secret");
    }
    for p in [
        ".env.example",
        "sub/.env.sample",
        "src/main.rs",
        "environment.ts",
        "credentials.rs",
        "README.md",
        "keyboard.rs",
    ] {
        assert!(!is_secret_path(p), "{p} must not be secret");
    }
}

#[test]
fn env_text_keeps_keys_comments_and_blank_lines() {
    let src = "# comment\n\nOPENAI_API_KEY=sk-abc123\nexport DB_URL=postgres://u:p@h/db\nEMPTY=\nPEM=\"-----BEGIN\nMIDDLE\n";
    let out = mask_env_text(src);
    assert_eq!(
        out,
        "# comment\n\nOPENAI_API_KEY=********\nexport DB_URL=********\nEMPTY=\nPEM=********\n********\n"
    );
    assert!(!out.contains("sk-abc123") && !out.contains("postgres"));
}

#[test]
fn env_text_preserves_crlf() {
    assert_eq!(mask_env_text("A=1\r\nB=2"), "A=********\r\nB=********");
}

#[test]
fn secret_like_lines_are_masked_in_diffs_and_code() {
    let src = "+OPENAI_API_KEY=sk-live-1234\n-SENTRY_DSN=https://x@sentry.io/1\n \"password\": \"hunter2\",\n+const GITHUB_TOKEN = \"ghp_abcdef\";\n+db_secret: s3cr3tvalue99\n";
    let out = mask_secret_like_lines(src);
    for leak in ["sk-live", "sentry.io", "hunter2", "ghp_abcdef", "s3cr3t"] {
        assert!(!out.contains(leak), "leaked {leak}: {out}");
    }
    assert!(out.contains("+OPENAI_API_KEY=********"));
    assert!(out.contains("-SENTRY_DSN=********"));
    assert_eq!(out.lines().count(), 5);
}

#[test]
fn harmless_lines_are_left_alone() {
    let src = "+const API_KEY = process.env.API_KEY;\n+    pub api_key: Option<String>,\n+    pub token: String,\n+let key = std::env::var(\"X\");\n+max_tokens: 100\n+name = \"neppy\"\n+API_KEY=\n";
    assert_eq!(mask_secret_like_lines(src), src);
}

#[test]
fn diff_hides_secret_files_and_masks_the_rest() {
    let diff = "diff --git a/.env b/.env\nindex 1..2 100644\n--- a/.env\n+++ b/.env\n@@ -1 +1 @@\n-A=1\n+OPENAI_API_KEY=sk-live\ndiff --git a/src/x.rs b/src/x.rs\nindex 1..2 100644\n--- a/src/x.rs\n+++ b/src/x.rs\n@@ -1 +1,2 @@\n+let a = 1;\n+SECRET_TOKEN = \"abc\"\ndiff --git a/sub/credentials.json b/sub/credentials.json\n+++ b/sub/credentials.json\n+{\"k\":\"v\"}\n";
    let out = mask_diff(diff);
    assert!(out.contains("diff --git a/.env b/.env\n[secret file changed, content hidden]\n"));
    assert!(out.contains("diff --git a/sub/credentials.json b/sub/credentials.json\n[secret file changed, content hidden]\n"));
    assert!(!out.contains("sk-live") && !out.contains("\"abc\"") && !out.contains("\"k\""));
    assert!(out.contains("+let a = 1;"));
    assert!(out.contains("+SECRET_TOKEN = ********"));
    assert!(
        !out.contains("OPENAI_API_KEY"),
        "hunks of a secret file are dropped"
    );
}

#[test]
fn diff_without_secrets_is_unchanged() {
    let diff = "diff --git a/a.txt b/a.txt\n--- a/a.txt\n+++ b/a.txt\n@@ -1 +1 @@\n-one\n+two\n";
    assert_eq!(mask_diff(diff), diff);
    assert_eq!(mask_diff(""), "");
}

#[test]
fn file_masking_only_touches_secret_paths() {
    assert_eq!(mask_file_for_agent(".env", "A=1\n"), "A=********\n");
    assert_eq!(mask_file_for_agent("a.txt", "A=1\n"), "A=1\n");
    assert_eq!(mask_file_for_agent(".env.example", "A=1\n"), "A=1\n");
    assert_eq!(
        mask_file_for_agent("id_rsa", "-----BEGIN\nabc==\n-----END\n"),
        KEY_FILE_MARKER
    );
    assert_eq!(
        mask_file_for_agent("credentials.json", "{\n  \"k\": \"v\"\n}\n"),
        "********\n********\n********\n"
    );
}
