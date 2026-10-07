//! Secret protection for Debug Mode (spec section 22).
//!
//! The agent may need to know an environment variable *exists* but never needs
//! its value. These helpers mask values in three places: `debug_mode_diff`
//! output, `file_read` results inside a Debug turn, and task summaries /
//! validation output before they are stored.
//!
//! No existing helper fits: `util::redact` hashes identifiers for log lines and
//! `security::approval::redact` walks JSON trees by key name. What is needed
//! here is line-oriented text masking that keeps keys and comments readable.

use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

/// What a masked value is replaced with.
pub const MASK: &str = "********";
/// Shown instead of a changed secret file's hunks.
pub const SECRET_FILE_MARKER: &str = "[secret file changed, content hidden]";

/// Whether `path`'s file name marks it as a secret store.
pub fn is_secret_path(path: impl AsRef<Path>) -> bool {
    let Some(name) = path.as_ref().file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let n = name.to_ascii_lowercase();
    if n == ".env" || n.starts_with(".env.") {
        return !matches!(n.as_str(), ".env.example" | ".env.sample");
    }
    n == "credentials.json"
        || n.ends_with(".pem")
        || n.ends_with(".key")
        || n.ends_with(".p12")
        || n.starts_with("id_rsa")
        || n.starts_with("id_ed25519")
        || n == ".npmrc"
        || n == ".pypirc"
        || n.starts_with("secrets.")
}

/// Masks every value of a dotenv-style text: `KEY=value` becomes `KEY=********`.
/// Comments, blank lines and keys survive; an empty value stays empty (it is
/// information, not a secret); a non-comment line without `=` (a continuation
/// of a multi-line value) is masked whole.
pub fn mask_env_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, eol) = split_eol(line);
        let trimmed = body.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push_str(body);
        } else if let Some(eq) = body.find('=') {
            let (key, value) = body.split_at(eq + 1);
            out.push_str(key);
            if !value.trim().is_empty() {
                out.push_str(MASK);
            }
        } else {
            out.push_str(MASK);
        }
        out.push_str(eol);
    }
    out
}

fn split_eol(line: &str) -> (&str, &str) {
    if let Some(b) = line.strip_suffix("\r\n") {
        (b, "\r\n")
    } else if let Some(b) = line.strip_suffix('\n') {
        (b, "\n")
    } else {
        (line, "")
    }
}

fn assignment_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r#"^(?P<pre>[+\- ]?\s*(?:export\s+|pub\s+|const\s+|let\s+|var\s+|static\s+)*["']?(?P<key>[A-Za-z0-9_.\-]+)["']?\s*(?P<sep>[=:])\s*)(?P<val>\S.*)$"#,
        )
        .expect("static regex")
    })
}

fn key_is_secret_like(key: &str) -> bool {
    let k = key.to_ascii_uppercase().replace(['.', '-'], "_");
    const TAILS: &[&str] = &[
        "_KEY",
        "APIKEY",
        "PRIVATEKEY",
        "ACCESSKEY",
        "SECRETKEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "DSN",
    ];
    k == "KEY" || TAILS.iter().any(|t| k.ends_with(t))
}

/// A value that only reads a secret from somewhere else (or is a type name)
/// carries nothing to hide, and masking it would make diffs unreadable.
fn value_is_harmless(val: &str, sep: &str) -> bool {
    let v = val.trim().trim_end_matches([',', ';']).trim();
    if v.is_empty() || v == MASK {
        return true;
    }
    let lower = v.to_ascii_lowercase();
    if v.starts_with('$')
        || lower.contains("process.env")
        || lower.contains("env::var")
        || lower.contains("getenv")
        || lower.contains("os.environ")
        || lower.contains("std::env")
        || lower.contains("import.meta.env")
    {
        return true;
    }
    if sep == ":" {
        let quoted = v.starts_with('"') || v.starts_with('\'');
        if !quoted {
            let type_like = v.contains(['<', '>', '(', ')', '[', ']', '{', '}', '&'])
                || !(v.len() >= 12 || v.chars().any(|c| c.is_ascii_digit()));
            return type_like;
        }
    }
    false
}

/// Masks the value of lines whose key looks like `*_KEY` / `*_TOKEN` /
/// `*_SECRET` / `PASSWORD` / `DSN`. Works on plain text and on unified-diff
/// lines (a leading `+`, `-` or space is kept).
pub fn mask_secret_like_lines(text: &str) -> String {
    let re = assignment_re();
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let (body, eol) = split_eol(line);
        match re.captures(body) {
            Some(c)
                if key_is_secret_like(&c["key"]) && !value_is_harmless(&c["val"], &c["sep"]) =>
            {
                out.push_str(&c["pre"]);
                out.push_str(MASK);
            }
            _ => out.push_str(body),
        }
        out.push_str(eol);
    }
    out
}

/// Path of a `diff --git a/X b/X` header, if `line` is one.
fn diff_header_path(line: &str) -> Option<String> {
    let rest = line.strip_prefix("diff --git ")?;
    let b = rest.rfind(" b/").map(|i| &rest[i + 3..]).or_else(|| {
        rest.rfind(" \"b/")
            .map(|i| rest[i + 4..].trim_end_matches('"'))
    })?;
    Some(b.trim_end_matches('"').to_string())
}

/// Masks a unified diff: hunks of secret files are replaced by
/// [`SECRET_FILE_MARKER`]; every other file passes through
/// [`mask_secret_like_lines`].
pub fn mask_diff(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut section = String::new();
    let mut hidden = false;
    let flush = |section: &mut String, hidden: bool, out: &mut String| {
        if section.is_empty() {
            return;
        }
        if hidden {
            let header = section.lines().next().unwrap_or("");
            out.push_str(header);
            out.push('\n');
            out.push_str(SECRET_FILE_MARKER);
            out.push('\n');
        } else {
            out.push_str(&mask_secret_like_lines(section));
        }
        section.clear();
    };
    for line in text.split_inclusive('\n') {
        if let Some(p) = diff_header_path(line.trim_end_matches(['\n', '\r'])) {
            flush(&mut section, hidden, &mut out);
            hidden = is_secret_path(&p);
        }
        section.push_str(line);
    }
    flush(&mut section, hidden, &mut out);
    out
}

/// Shown instead of the content of a key / certificate file.
pub const KEY_FILE_MARKER: &str = "[key material hidden]";

fn is_key_material(path: &Path) -> bool {
    let n = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    n.ends_with(".pem")
        || n.ends_with(".key")
        || n.ends_with(".p12")
        || n.starts_with("id_rsa")
        || n.starts_with("id_ed25519")
}

/// Masking applied to `file_read` output inside a Debug turn. Key and
/// certificate files are hidden whole (base64 padding makes line masking leak
/// prefixes); other secret files are dotenv-masked line by line; everything
/// else passes through unchanged.
pub fn mask_file_for_agent(path: impl AsRef<Path>, contents: &str) -> String {
    let path = path.as_ref();
    if !is_secret_path(path) {
        contents.to_string()
    } else if is_key_material(path) {
        KEY_FILE_MARKER.to_string()
    } else {
        mask_env_text(contents)
    }
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
