//! Sensitive-data scrubber: the single door through which observed text becomes
//! an [`ObservationEvent`](super::types::ObservationEvent) field.
//!
//! [`Scrubbed`] has a private field, so the only way to obtain one is
//! [`scrub`]. Raw text therefore cannot be typed into an event, a log line, a
//! socket frame or a DB row. The function is pure, synchronous, I/O free and
//! idempotent (`scrub(scrub(x)) == scrub(x)`, test enforced), and handles
//! multi-line input (OCR of a whole screen), not just a single clipboard token.
//!
//! Hard **drop** (whole text discarded): secure field, concealed pasteboard,
//! any PEM block (`-----BEGIN ...`), a seed-phrase-like run of 12+ lowercase
//! words (selection / clipboard / OCR), a password-like single token
//! (selection / clipboard), or more than half of the text redacted.
//!
//! **Redact** spans, in order: API keys and tokens and national IDs
//! (`memory::safety::sanitize_text`, which also runs the Luhn-checked card and
//! IBAN screen with fullwidth-digit normalisation), OTP codes, `password:`
//! style lines.
//!
//! Nothing here logs text. Truncation happens AFTER redaction and never leaves
//! a partial token, so a cap cannot cut a secret in half.

use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

use super::types::DropReason;
use crate::neppy::memory::safety;

pub const MAX_TITLE_CHARS: usize = 200;
pub const MAX_SELECTION_CHARS: usize = 2000;
pub const MAX_CLIPBOARD_CHARS: usize = 2000;
pub const MAX_OCR_CHARS: usize = 4000;
pub const MAX_GENERATED_CHARS: usize = 4000;
/// Input beyond this is cut (at whitespace) before scanning.
const MAX_RAW_CHARS: usize = 100_000;
/// Run length of lowercase words treated as a seed phrase.
const SEED_MIN_WORDS: usize = 12;

/// Text that has been through [`scrub`]. Private field: not constructible
/// elsewhere. No `Serialize`, no `Default`; `Debug` never prints content.
#[derive(Clone, PartialEq, Eq)]
pub struct Scrubbed(String);

impl Scrubbed {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// First `max_chars` characters (already scrubbed text, so safe to show).
    pub fn excerpt(&self, max_chars: usize) -> String {
        cap_chars(&self.0, max_chars)
    }
}

impl fmt::Debug for Scrubbed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Scrubbed(<{} bytes>)", self.0.len())
    }
}

/// Where the text came from. Decides which rules and which cap apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrubSource {
    Title,
    Selection,
    Clipboard,
    /// On-device OCR of a screen capture (multi-line).
    Ocr,
    /// Model output / stored excerpts being re-scrubbed: redaction and PEM only.
    Generated,
}

impl ScrubSource {
    pub fn default_cap(self) -> usize {
        match self {
            ScrubSource::Title => MAX_TITLE_CHARS,
            ScrubSource::Selection => MAX_SELECTION_CHARS,
            ScrubSource::Clipboard => MAX_CLIPBOARD_CHARS,
            ScrubSource::Ocr => MAX_OCR_CHARS,
            ScrubSource::Generated => MAX_GENERATED_CHARS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrubCtx {
    pub source: ScrubSource,
    /// The focused field is a secure (password) field.
    pub secure_field: bool,
    /// The pasteboard is flagged concealed / transient / auto-generated.
    pub concealed: bool,
    /// Override of the default cap for the source.
    pub cap: Option<usize>,
}

impl ScrubCtx {
    pub fn new(source: ScrubSource) -> Self {
        Self {
            source,
            secure_field: false,
            concealed: false,
            cap: None,
        }
    }
    pub fn title() -> Self {
        Self::new(ScrubSource::Title)
    }
    pub fn selection(secure_field: bool) -> Self {
        Self {
            secure_field,
            ..Self::new(ScrubSource::Selection)
        }
    }
    pub fn clipboard(concealed: bool) -> Self {
        Self {
            concealed,
            ..Self::new(ScrubSource::Clipboard)
        }
    }
    pub fn ocr() -> Self {
        Self::new(ScrubSource::Ocr)
    }
    pub fn generated() -> Self {
        Self::new(ScrubSource::Generated)
    }
    pub fn capped(mut self, cap: usize) -> Self {
        self.cap = Some(cap);
        self
    }
}

/// What was found (never the content). For counters and drop reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SensitiveKind {
    SecureField,
    ConcealedClipboard,
    PemBlock,
    SeedPhrase,
    PasswordToken,
    /// API key / token / bearer / JWT.
    Secret,
    /// Card number, IBAN, national ID, email, phone.
    Pii,
    Otp,
    PasswordLine,
    /// More than half of the text was redaction markers.
    TooRedacted,
}

impl SensitiveKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SecureField => "secure_field",
            Self::ConcealedClipboard => "concealed_clipboard",
            Self::PemBlock => "pem_block",
            Self::SeedPhrase => "seed_phrase",
            Self::PasswordToken => "password_token",
            Self::Secret => "secret",
            Self::Pii => "pii",
            Self::Otp => "otp",
            Self::PasswordLine => "password_line",
            Self::TooRedacted => "too_redacted",
        }
    }

    /// The [`DropReason`] a hard drop of this kind is reported as.
    pub fn drop_reason(self) -> DropReason {
        match self {
            Self::SecureField => DropReason::SecureField,
            Self::ConcealedClipboard => DropReason::ConcealedClipboard,
            _ => DropReason::SensitiveContent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrubResult {
    /// Nothing sensitive found.
    Clean(Scrubbed),
    /// Spans were redacted; the text is still usable.
    Redacted(Scrubbed, Vec<SensitiveKind>),
    /// The whole text is discarded.
    Drop(SensitiveKind),
}

impl ScrubResult {
    /// The scrubbed text, if not dropped.
    pub fn text(&self) -> Option<&Scrubbed> {
        match self {
            ScrubResult::Clean(s) | ScrubResult::Redacted(s, _) => Some(s),
            ScrubResult::Drop(_) => None,
        }
    }

    pub fn into_text(self) -> Option<Scrubbed> {
        match self {
            ScrubResult::Clean(s) | ScrubResult::Redacted(s, _) => Some(s),
            ScrubResult::Drop(_) => None,
        }
    }

    pub fn was_redacted(&self) -> bool {
        matches!(self, ScrubResult::Redacted(..))
    }

    pub fn drop_kind(&self) -> Option<SensitiveKind> {
        match self {
            ScrubResult::Drop(k) => Some(*k),
            _ => None,
        }
    }
}

static PEM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"-----BEGIN [A-Z0-9 ]+-----").expect("pem header"));
static OTP_NUM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{4,8}\b").expect("otp num"));
static OTP_KW_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\b(code|otp|passcode|verification|2fa|one[- ]time|pin)\b|код)")
        .expect("otp kw")
});
static PASSWORD_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(password|passwd|pwd|passphrase|pin)[ \t]*[:=][ \t]*\S+")
        .expect("password line")
});
static MARKER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[REDACTED[A-Z_]*\]").expect("marker"));

const OTP_MARKER: &str = "[REDACTED_OTP]";
const PASSWORD_MARKER: &str = "[REDACTED_PASSWORD]";
const OTP_KEYWORD_DISTANCE: usize = 40;

/// Scrub `raw`. See the module docs for the rules.
pub fn scrub(raw: &str, ctx: &ScrubCtx) -> ScrubResult {
    if ctx.secure_field {
        return ScrubResult::Drop(SensitiveKind::SecureField);
    }
    if ctx.concealed {
        return ScrubResult::Drop(SensitiveKind::ConcealedClipboard);
    }

    let prepared = prepare(raw);
    if PEM_RE.is_match(&prepared) {
        return ScrubResult::Drop(SensitiveKind::PemBlock);
    }
    let reads_free_text = matches!(
        ctx.source,
        ScrubSource::Selection | ScrubSource::Clipboard | ScrubSource::Ocr
    );
    if reads_free_text && has_seed_run(&prepared) {
        return ScrubResult::Drop(SensitiveKind::SeedPhrase);
    }
    if matches!(ctx.source, ScrubSource::Selection | ScrubSource::Clipboard)
        && is_password_like_token(prepared.trim())
    {
        return ScrubResult::Drop(SensitiveKind::PasswordToken);
    }

    let mut kinds: Vec<SensitiveKind> = Vec::new();
    let push = |k: SensitiveKind, kinds: &mut Vec<SensitiveKind>| {
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    };

    // 1. API keys / tokens / PII (sanitize_text runs redact_pii internally).
    let sanitized = safety::sanitize_text(&prepared);
    if sanitized.report.blocked_secret_hits > 0 {
        return ScrubResult::Drop(SensitiveKind::PemBlock);
    }
    if sanitized.report.text_redactions > 0 {
        push(SensitiveKind::Secret, &mut kinds);
    }
    if sanitized.report.pii_redactions > 0 {
        push(SensitiveKind::Pii, &mut kinds);
    }
    let mut text = sanitized.value;

    // 2. OTP codes.
    if ctx.source == ScrubSource::Clipboard && is_bare_otp(text.trim()) {
        text = OTP_MARKER.to_string();
        push(SensitiveKind::Otp, &mut kinds);
    } else {
        let (next, changed) = redact_otp(&text);
        if changed {
            text = next;
            push(SensitiveKind::Otp, &mut kinds);
        }
    }

    // 3. `password: ...` lines.
    if PASSWORD_LINE_RE.is_match(&text) {
        text = PASSWORD_LINE_RE
            .replace_all(&text, PASSWORD_MARKER)
            .into_owned();
        push(SensitiveKind::PasswordLine, &mut kinds);
    }

    if !kinds.is_empty() && marker_ratio_over_half(&text) {
        return ScrubResult::Drop(SensitiveKind::TooRedacted);
    }

    let cap = ctx.cap.unwrap_or_else(|| ctx.source.default_cap());
    let text = Scrubbed(cap_chars(text.trim(), cap));
    if kinds.is_empty() {
        ScrubResult::Clean(text)
    } else {
        ScrubResult::Redacted(text, kinds)
    }
}

/// Strip zero-width characters and bound the input at a whitespace boundary.
fn prepare(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .filter(|c| !matches!(*c, '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FEFF}'))
        .collect();
    if s.chars().count() > MAX_RAW_CHARS {
        let cut: String = s.chars().take(MAX_RAW_CHARS).collect();
        let keep = cut.rfind(char::is_whitespace).unwrap_or(0);
        s = cut[..keep].to_string();
    }
    s
}

/// At most `max_chars` characters (ellipsis included), cut at whitespace so no
/// partial token is left behind.
pub(crate) fn cap_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let head: String = text.chars().take(max_chars - 1).collect();
    let next_is_boundary = text
        .chars()
        .nth(max_chars - 1)
        .map(char::is_whitespace)
        .unwrap_or(true);
    let kept = if next_is_boundary {
        head.as_str()
    } else {
        match head.rfind(char::is_whitespace) {
            Some(i) if i > 0 => &head[..i],
            _ => "",
        }
    };
    format!("{}…", kept.trim_end())
}

fn is_bare_otp(t: &str) -> bool {
    t.chars().count() == 6 && t.chars().all(|c| c.is_ascii_digit())
}

fn redact_otp(text: &str) -> (String, bool) {
    let kws: Vec<(usize, usize)> = OTP_KW_RE
        .find_iter(text)
        .map(|m| (m.start(), m.end()))
        .collect();
    if kws.is_empty() {
        return (text.to_string(), false);
    }
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut changed = false;
    for m in OTP_NUM_RE.find_iter(text) {
        let near = kws.iter().any(|&(ks, ke)| {
            let dist = if ke <= m.start() {
                text[ke..m.start()].chars().count()
            } else if ks >= m.end() {
                text[m.end()..ks].chars().count()
            } else {
                0
            };
            dist <= OTP_KEYWORD_DISTANCE
        });
        if near {
            out.push_str(&text[last..m.start()]);
            out.push_str(OTP_MARKER);
            last = m.end();
            changed = true;
        }
    }
    out.push_str(&text[last..]);
    (out, changed)
}

fn marker_ratio_over_half(text: &str) -> bool {
    let total = text.chars().count();
    if total == 0 {
        return false;
    }
    let marked: usize = MARKER_RE
        .find_iter(text)
        .map(|m| m.as_str().chars().count())
        .sum();
    marked * 2 > total
}

fn is_seed_word(tok: &str) -> bool {
    (3..=8).contains(&tok.len()) && tok.bytes().all(|b| b.is_ascii_lowercase())
}

fn is_index_token(tok: &str) -> bool {
    let t = tok.trim_end_matches(['.', ')', ':', ',']);
    !t.is_empty() && t.len() <= 2 && t.bytes().all(|b| b.is_ascii_digit())
}

/// A run of `SEED_MIN_WORDS`+ consecutive lowercase 3-8 letter words (numbering
/// like `1.` or `01` between them is skipped, newlines count as spaces). Any
/// other token (capitalised, punctuated, longer, digits) resets the run, so
/// ordinary prose does not match. False positives (a long lowercase listing)
/// drop the text: the safe direction.
fn has_seed_run(text: &str) -> bool {
    let mut run = 0usize;
    for tok in text.split_whitespace() {
        if is_seed_word(tok) {
            run += 1;
            if run >= SEED_MIN_WORDS {
                return true;
            }
        } else if is_index_token(tok) {
            continue;
        } else {
            run = 0;
        }
    }
    false
}

/// A single token that looks like a password: 8..=128 chars, no whitespace,
/// at least 3 of upper / lower / digit / symbol, and not a URL, path, email,
/// identifier or an already redacted marker.
fn is_password_like_token(t: &str) -> bool {
    let n = t.chars().count();
    if !(8..=128).contains(&n) || t.chars().any(char::is_whitespace) {
        return false;
    }
    if t.contains("[REDACTED") || t.contains("://") {
        return false;
    }
    if t.starts_with('/') || t.starts_with("~/") || t.starts_with("./") || t.starts_with("../") {
        return false;
    }
    let simple = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-');
    if t.contains('/') && t.chars().all(|c| simple(c) || c == '/') {
        return false;
    }
    if let Some(at) = t.find('@') {
        if t[at..].contains('.') {
            return false;
        }
    }
    if t.chars().all(simple) && t.chars().any(|c| matches!(c, '_' | '.' | '-')) {
        return false;
    }
    let upper = t.chars().any(|c| c.is_uppercase());
    let lower = t.chars().any(|c| c.is_lowercase());
    let digit = t.chars().any(|c| c.is_ascii_digit());
    let symbol = t.chars().any(|c| !c.is_alphanumeric());
    [upper, lower, digit, symbol].iter().filter(|b| **b).count() >= 3
}

#[cfg(test)]
#[path = "sensitive_tests.rs"]
mod sensitive_tests;
