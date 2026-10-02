//! Pure activity rules: which observation, if any, is worth a suggestion.
//!
//! Inputs are scrubbed events; outputs are a [`TriggerKind`] plus a
//! fingerprint used for novelty. Rules (kind -> condition):
//!
//! * `BuildError`: terminal or IDE app and text (clipboard / selection / OCR /
//!   ask) matching a compiler or runtime error shape.
//! * `EmailDraft`: a mail app, or a browser whose title looks like a compose
//!   view, plus a selection of at least 20 chars.
//! * `Term`: a reader or browser with a scholarly title and a 1-6 word,
//!   at most 60 char selection.
//! * `Ask`: the user asked. `Capture`: the user triggered a manual capture.
//!   Autonomous captures (`user_initiated == false`) are ordinary OCR text and
//!   are run through the `BuildError` rule only.

use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

use super::types::{ObservationEvent, ObservationKind, TriggerKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppClass {
    Terminal,
    Ide,
    Mail,
    Browser,
    Reader,
    Other,
}

const TERMINALS: &[&str] = &[
    "com.apple.terminal",
    "com.googlecode.iterm2",
    "dev.warp.warp-stable",
    "com.mitchellh.ghostty",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "io.alacritty",
];
const IDES: &[&str] = &[
    "com.microsoft.vscode",
    "com.todesktop.230313mzl4w4u92",
    "dev.zed.zed",
    "com.apple.dt.xcode",
];
const MAIL: &[&str] = &["com.apple.mail", "com.microsoft.outlook"];
const BROWSERS: &[&str] = &[
    "com.apple.safari",
    "com.google.chrome",
    "org.mozilla.firefox",
    "company.thebrowser.browser",
    "com.brave.browser",
    "com.microsoft.edgemac",
];
const READERS: &[&str] = &["com.apple.preview", "net.sourceforge.skim-app.skim"];

pub fn classify_app(bundle_id: Option<&str>) -> AppClass {
    let Some(b) = bundle_id else {
        return AppClass::Other;
    };
    let b = b.to_lowercase();
    if TERMINALS.contains(&b.as_str()) {
        AppClass::Terminal
    } else if IDES.contains(&b.as_str()) || b.starts_with("com.jetbrains.") {
        AppClass::Ide
    } else if MAIL.contains(&b.as_str()) {
        AppClass::Mail
    } else if BROWSERS.contains(&b.as_str()) {
        AppClass::Browser
    } else if READERS.contains(&b.as_str()) {
        AppClass::Reader
    } else {
        AppClass::Other
    }
}

static BUILD_ERROR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)(error(\[E\d{4}\])?:|Traceback \(most recent call last\)|npm ERR!|^FAILED|BUILD FAILED|fatal error:|\bTS\d{4}:|Exception in thread|panicked at|cannot find (module|symbol))",
    )
    .expect("build error")
});
static COMPOSE_TITLE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(gmail|outlook|compose|^re:|^fwd?:)").expect("compose"));
static TERM_TITLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(arxiv|doi\.org|\.pdf|sciencedirect|springer|ieee|acm\.org|pubmed|jstor|semantic scholar)",
    )
    .expect("term title")
});

pub const EMAIL_MIN_SELECTION_CHARS: usize = 20;
pub const TERM_MAX_WORDS: usize = 6;
pub const TERM_MAX_CHARS: usize = 60;
const FINGERPRINT_CHARS: usize = 200;

/// A detected trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detected {
    pub kind: TriggerKind,
    /// `sha256(kind | bundle_id | normalized first 200 chars)`, hex.
    pub fingerprint: String,
}

/// Plain inputs for [`detect`] (no raw/serialisable event required).
#[derive(Debug, Clone, Copy)]
pub struct ActivityInput<'a> {
    pub kind: ObservationKind,
    pub bundle_id: Option<&'a str>,
    pub title: Option<&'a str>,
    pub text: Option<&'a str>,
    pub user_initiated: bool,
}

/// Evaluate one scrubbed event.
pub fn detect_event(ev: &ObservationEvent) -> Option<Detected> {
    detect(&ActivityInput {
        kind: ev.kind,
        bundle_id: ev.bundle_id.as_deref(),
        title: ev.title.as_ref().map(|t| t.as_str()),
        text: ev.text.as_ref().map(|t| t.as_str()),
        user_initiated: ev.flags.user_initiated,
    })
}

pub fn detect(input: &ActivityInput<'_>) -> Option<Detected> {
    let class = classify_app(input.bundle_id);
    let text = input.text.unwrap_or("");
    let title = input.title.unwrap_or("");

    match input.kind {
        ObservationKind::Ask => {
            return Some(detected(TriggerKind::Ask, input.bundle_id, text));
        }
        ObservationKind::Capture if input.user_initiated => {
            return Some(detected(TriggerKind::Capture, input.bundle_id, text));
        }
        _ => {}
    }

    // Build errors can come from any text-bearing observation in a dev app.
    if matches!(class, AppClass::Terminal | AppClass::Ide)
        && matches!(
            input.kind,
            ObservationKind::Clipboard | ObservationKind::Selection | ObservationKind::Capture
        )
    {
        if let Some(m) = BUILD_ERROR_RE.find(text) {
            return Some(detected(
                TriggerKind::BuildError,
                input.bundle_id,
                &text[m.start()..],
            ));
        }
    }

    // The remaining rules are selection based.
    if input.kind != ObservationKind::Selection {
        return None;
    }
    let sel_chars = text.trim().chars().count();
    let email_ctx =
        class == AppClass::Mail || (class == AppClass::Browser && COMPOSE_TITLE_RE.is_match(title));
    if email_ctx && sel_chars >= EMAIL_MIN_SELECTION_CHARS {
        return Some(detected(TriggerKind::EmailDraft, input.bundle_id, text));
    }
    let term_ctx =
        matches!(class, AppClass::Reader | AppClass::Browser) && TERM_TITLE_RE.is_match(title);
    let words = text.split_whitespace().count();
    if term_ctx && (1..=TERM_MAX_WORDS).contains(&words) && sel_chars <= TERM_MAX_CHARS {
        return Some(detected(TriggerKind::Term, input.bundle_id, text));
    }
    None
}

fn detected(kind: TriggerKind, bundle_id: Option<&str>, text: &str) -> Detected {
    Detected {
        kind,
        fingerprint: fingerprint(kind, bundle_id, text),
    }
}

/// Stable fingerprint: lowercased, whitespace collapsed, digit runs folded
/// (so a changing line number does not defeat novelty), first 200 chars.
pub fn fingerprint(kind: TriggerKind, bundle_id: Option<&str>, text: &str) -> String {
    let mut norm = String::new();
    let mut prev_space = true;
    let mut prev_digit = false;
    let mut count = 0usize;
    for c in text.chars() {
        if count >= FINGERPRINT_CHARS {
            break;
        }
        count += 1;
        if c.is_whitespace() {
            if !prev_space {
                norm.push(' ');
            }
            prev_space = true;
            prev_digit = false;
        } else if c.is_ascii_digit() {
            if !prev_digit {
                norm.push('#');
            }
            prev_digit = true;
            prev_space = false;
        } else {
            norm.extend(c.to_lowercase());
            prev_space = false;
            prev_digit = false;
        }
    }
    let mut h = Sha256::new();
    h.update(kind.as_str().as_bytes());
    h.update(b"|");
    h.update(bundle_id.unwrap_or("").to_lowercase().as_bytes());
    h.update(b"|");
    h.update(norm.trim().as_bytes());
    hex::encode(h.finalize())
}

#[cfg(test)]
#[path = "activity_tests.rs"]
mod activity_tests;
