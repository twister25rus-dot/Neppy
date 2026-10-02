//! Companion settings: defaults, wire shapes, patch validation.
//!
//! Defaults (user decision D1-D6): the companion itself is OFF until the user
//! enables it through the consent dialog; once enabled EVERY source is on
//! (app + window title, selection, clipboard, autonomous screen capture + OCR),
//! and each stays individually switchable. Only private information is
//! excluded (see `sensitive`, `exclusions`). Cloud models may receive scrubbed
//! text by default (`allow_cloud_model = true`); a setting switches to
//! local-only.
//!
//! High-risk categories are pinned at level 1 ("ask"): a patch that sets one
//! higher is rejected with `"<category> is high-risk and always asks"`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::exclusions::{validate_rules, MAX_RULES, MAX_RULE_CHARS};
use super::types::{ActionCategory, Chattiness, CompanionLevel, CompanionSource, TriggerKind};

pub const DEFAULT_HOTKEY_PAUSE: &str = "CmdOrCtrl+Alt+Shift+P";
pub const DEFAULT_HOTKEY_ASK: &str = "CmdOrCtrl+Alt+Shift+Space";
pub const DEFAULT_HOTKEY_CAPTURE: &str = "CmdOrCtrl+Alt+Shift+S";

/// Password managers and keychains, excluded by default.
pub const DEFAULT_EXCLUDED_APPS: &[&str] = &[
    "com.1password.1password",
    "com.agilebits.onepassword7",
    "com.bitwarden.desktop",
    "com.apple.keychainaccess",
    "com.apple.Passwords",
    "com.lastpass.LastPass",
    "org.keepassxc.keepassxc",
    "com.dashlane.dashlanephonefinal",
];

pub const DEFAULT_EXCLUDED_TITLE_PATTERNS: &[&str] = &[
    "incognito",
    "private browsing",
    "inprivate",
    "private window",
];

pub const DEFAULT_RETENTION_DAYS: u32 = 7;
pub const DEFAULT_MIN_INTERVAL_MIN: u32 = 10;
pub const DEFAULT_MAX_PER_HOUR: u32 = 3;
pub const DEFAULT_APP_COOLDOWN_MIN: u32 = 30;
pub const DEFAULT_SCREEN_MIN_INTERVAL_SECS: u32 = 30;
/// On-device OCR languages (explicit: Vision auto-detect costs ~19 s a frame).
pub const DEFAULT_OCR_LANGUAGES: &[&str] = &["en-US"];
pub const MAX_OCR_LANGUAGES: usize = 4;
/// Actions older than this are pruned regardless of `retention_days`.
pub const ACTION_RETENTION_DAYS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceToggles {
    pub app_window: bool,
    pub selection: bool,
    pub clipboard: bool,
    /// Autonomous, rate-limited screen capture + on-device OCR.
    pub screen_capture: bool,
}

impl Default for SourceToggles {
    fn default() -> Self {
        Self {
            app_window: true,
            selection: true,
            clipboard: true,
            screen_capture: true,
        }
    }
}

impl SourceToggles {
    pub fn is_on(&self, source: CompanionSource) -> bool {
        match source {
            CompanionSource::AppWindow => self.app_window,
            CompanionSource::Selection => self.selection,
            CompanionSource::Clipboard => self.clipboard,
            CompanionSource::ScreenCapture => self.screen_capture,
        }
    }

    pub fn any_on(&self) -> bool {
        self.app_window || self.selection || self.clipboard || self.screen_capture
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub pause: String,
    pub ask: String,
    pub capture: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            pause: DEFAULT_HOTKEY_PAUSE.into(),
            ask: DEFAULT_HOTKEY_ASK.into(),
            capture: DEFAULT_HOTKEY_CAPTURE.into(),
        }
    }
}

/// Wire shape of `CompanionSettings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CompanionSettings {
    /// Master switch. OFF until the user consents.
    pub enabled: bool,
    pub level: CompanionLevel,
    pub sources: SourceToggles,
    /// Seconds between autonomous captures while the screen changes.
    pub screen_min_interval_secs: u32,
    /// Cloud chat model may be used for suggestions (scrubbed text only).
    pub allow_cloud_model: bool,
    pub retention_days: u32,
    pub chattiness: Chattiness,
    pub min_interval_min: u32,
    pub max_per_hour: u32,
    pub app_cooldown_min: u32,
    /// Per-category level (all categories always present after load).
    pub category_levels: BTreeMap<ActionCategory, CompanionLevel>,
    pub excluded_apps: Vec<String>,
    pub excluded_title_patterns: Vec<String>,
    /// Proactive kinds the user muted.
    pub muted_kinds: Vec<TriggerKind>,
    /// Bundle ids the user muted for proactive suggestions.
    pub muted_apps: Vec<String>,
    pub hotkeys: Hotkeys,
    /// BCP-47 language tags for on-device OCR, in priority order.
    pub ocr_languages: Vec<String>,
}

pub fn default_category_levels() -> BTreeMap<ActionCategory, CompanionLevel> {
    ActionCategory::ALL
        .iter()
        .map(|c| (*c, c.default_level()))
        .collect()
}

impl Default for CompanionSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            level: CompanionLevel::Suggest,
            sources: SourceToggles::default(),
            screen_min_interval_secs: DEFAULT_SCREEN_MIN_INTERVAL_SECS,
            allow_cloud_model: true,
            retention_days: DEFAULT_RETENTION_DAYS,
            chattiness: Chattiness::Normal,
            min_interval_min: DEFAULT_MIN_INTERVAL_MIN,
            max_per_hour: DEFAULT_MAX_PER_HOUR,
            app_cooldown_min: DEFAULT_APP_COOLDOWN_MIN,
            category_levels: default_category_levels(),
            excluded_apps: DEFAULT_EXCLUDED_APPS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            excluded_title_patterns: DEFAULT_EXCLUDED_TITLE_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            muted_kinds: Vec::new(),
            muted_apps: Vec::new(),
            hotkeys: Hotkeys::default(),
            ocr_languages: DEFAULT_OCR_LANGUAGES
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

impl CompanionSettings {
    /// The configured level of one category (high-risk never above Suggest).
    pub fn category_level(&self, category: ActionCategory) -> CompanionLevel {
        let l = self
            .category_levels
            .get(&category)
            .copied()
            .unwrap_or_else(|| category.default_level());
        if category.is_high_risk() {
            l.min(CompanionLevel::Suggest)
        } else {
            l
        }
    }

    /// Fill missing categories and clamp high-risk ones (used after load).
    pub fn normalized(mut self) -> Self {
        for c in ActionCategory::ALL {
            let l = self.category_level(*c);
            self.category_levels.insert(*c, l);
        }
        self
    }
}

/// Sources patch (all optional).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcesPatch {
    pub app_window: Option<bool>,
    pub selection: Option<bool>,
    pub clipboard: Option<bool>,
    pub screen_capture: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HotkeysPatch {
    pub pause: Option<String>,
    pub ask: Option<String>,
    pub capture: Option<String>,
}

/// Wire shape of `CompanionSettingsPatch`. Numbers arrive as `i64` and are
/// validated here so errors name the field.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionSettingsPatch {
    pub enabled: Option<bool>,
    pub level: Option<i64>,
    pub sources: Option<SourcesPatch>,
    pub screen_min_interval_secs: Option<i64>,
    pub allow_cloud_model: Option<bool>,
    pub retention_days: Option<i64>,
    pub chattiness: Option<String>,
    pub min_interval_min: Option<i64>,
    pub max_per_hour: Option<i64>,
    pub app_cooldown_min: Option<i64>,
    /// Partial map `category -> 0..=3`, merged onto the current table.
    pub category_levels: Option<BTreeMap<String, i64>>,
    pub excluded_apps: Option<Vec<String>>,
    pub excluded_title_patterns: Option<Vec<String>>,
    pub muted_kinds: Option<Vec<String>>,
    pub muted_apps: Option<Vec<String>>,
    pub hotkeys: Option<HotkeysPatch>,
    pub ocr_languages: Option<Vec<String>>,
}

/// A language tag such as `en-US`, `de`, `zh-Hans`.
fn is_language_tag(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let primary = parts.next().unwrap_or("");
    tag.len() <= 16
        && (2..=3).contains(&primary.len())
        && primary.chars().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (2..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

fn ranged(field: &str, v: i64, lo: i64, hi: i64) -> Result<u32, String> {
    if (lo..=hi).contains(&v) {
        Ok(v as u32)
    } else {
        Err(format!("invalid '{field}': must be {lo}..={hi}"))
    }
}

const MODIFIERS: &[&str] = &[
    "cmdorctrl",
    "cmd",
    "command",
    "ctrl",
    "control",
    "alt",
    "option",
    "shift",
    "super",
    "meta",
];

/// Validate an accelerator string such as `CmdOrCtrl+Alt+Shift+P`.
pub fn validate_hotkey(field: &str, raw: &str) -> Result<String, String> {
    let s = raw.trim();
    if s.is_empty() || s.chars().count() > 64 {
        return Err(format!(
            "invalid 'hotkeys.{field}': must be 1..64 characters"
        ));
    }
    let parts: Vec<&str> = s.split('+').collect();
    if parts.len() < 2
        || parts
            .iter()
            .any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric()))
    {
        return Err(format!(
            "invalid 'hotkeys.{field}': expected modifiers and a key, like CmdOrCtrl+Alt+Shift+P"
        ));
    }
    let (key, mods) = parts.split_last().expect("len >= 2");
    if mods
        .iter()
        .any(|m| !MODIFIERS.contains(&m.to_ascii_lowercase().as_str()))
    {
        return Err(format!("invalid 'hotkeys.{field}': unknown modifier"));
    }
    if MODIFIERS.contains(&key.to_ascii_lowercase().as_str()) {
        return Err(format!(
            "invalid 'hotkeys.{field}': the last part must be a key"
        ));
    }
    Ok(s.to_string())
}

fn dedup_strings(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in v {
        let s = s.trim().to_string();
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&s)) {
            out.push(s);
        }
    }
    out
}

/// Apply `patch` onto `current`, validating every field. Errors are
/// `invalid '<field>': ...` strings.
pub fn apply_patch(
    current: &CompanionSettings,
    patch: &CompanionSettingsPatch,
) -> Result<CompanionSettings, String> {
    let mut s = current.clone();
    if let Some(v) = patch.enabled {
        s.enabled = v;
    }
    if let Some(l) = patch.level {
        s.level = u8::try_from(l)
            .ok()
            .and_then(CompanionLevel::from_u8)
            .ok_or("invalid 'level': must be 0..=3")?;
    }
    if let Some(p) = &patch.sources {
        if let Some(v) = p.app_window {
            s.sources.app_window = v;
        }
        if let Some(v) = p.selection {
            s.sources.selection = v;
        }
        if let Some(v) = p.clipboard {
            s.sources.clipboard = v;
        }
        if let Some(v) = p.screen_capture {
            s.sources.screen_capture = v;
        }
    }
    if let Some(v) = patch.screen_min_interval_secs {
        s.screen_min_interval_secs = ranged("screen_min_interval_secs", v, 5, 3600)?;
    }
    if let Some(v) = patch.allow_cloud_model {
        s.allow_cloud_model = v;
    }
    if let Some(v) = patch.retention_days {
        s.retention_days = ranged("retention_days", v, 1, 90)?;
    }
    if let Some(c) = &patch.chattiness {
        s.chattiness =
            Chattiness::parse(c).ok_or("invalid 'chattiness': expected quiet|normal|eager")?;
    }
    if let Some(v) = patch.min_interval_min {
        s.min_interval_min = ranged("min_interval_min", v, 1, 240)?;
    }
    if let Some(v) = patch.max_per_hour {
        s.max_per_hour = ranged("max_per_hour", v, 1, 20)?;
    }
    if let Some(v) = patch.app_cooldown_min {
        s.app_cooldown_min = ranged("app_cooldown_min", v, 0, 480)?;
    }
    if let Some(map) = &patch.category_levels {
        for (name, lvl) in map {
            let cat = ActionCategory::parse(name)
                .ok_or_else(|| format!("invalid 'category_levels': unknown category '{name}'"))?;
            let level = u8::try_from(*lvl)
                .ok()
                .and_then(CompanionLevel::from_u8)
                .ok_or_else(|| format!("invalid 'category_levels': {name} must be 0..=3"))?;
            if cat.is_high_risk() && level > CompanionLevel::Suggest {
                return Err(format!(
                    "invalid 'category_levels': {name} is high-risk and always asks"
                ));
            }
            s.category_levels.insert(cat, level);
        }
    }
    if patch.excluded_apps.is_some() || patch.excluded_title_patterns.is_some() {
        let apps = patch
            .excluded_apps
            .as_ref()
            .map(|v| dedup_strings(v))
            .unwrap_or_else(|| s.excluded_apps.clone());
        let titles = patch
            .excluded_title_patterns
            .as_ref()
            .map(|v| dedup_strings(v))
            .unwrap_or_else(|| s.excluded_title_patterns.clone());
        validate_rules(&apps, &titles)?;
        s.excluded_apps = apps;
        s.excluded_title_patterns = titles;
    }
    if let Some(kinds) = &patch.muted_kinds {
        let mut out: Vec<TriggerKind> = Vec::new();
        for k in kinds {
            let kind = TriggerKind::parse(k)
                .filter(|k| k.is_proactive())
                .ok_or_else(|| {
                    format!("invalid 'muted_kinds': unknown or non-mutable kind '{k}'")
                })?;
            if !out.contains(&kind) {
                out.push(kind);
            }
        }
        s.muted_kinds = out;
    }
    if let Some(apps) = &patch.muted_apps {
        let apps = dedup_strings(apps);
        if apps.len() > MAX_RULES
            || apps.iter().any(|a| {
                a.is_empty()
                    || a.chars().count() > MAX_RULE_CHARS
                    || a.chars().any(char::is_control)
            })
        {
            return Err(format!(
                "invalid 'muted_apps': at most {MAX_RULES} non-empty entries of {MAX_RULE_CHARS} characters"
            ));
        }
        s.muted_apps = apps;
    }
    if let Some(h) = &patch.hotkeys {
        if let Some(v) = &h.pause {
            s.hotkeys.pause = validate_hotkey("pause", v)?;
        }
        if let Some(v) = &h.ask {
            s.hotkeys.ask = validate_hotkey("ask", v)?;
        }
        if let Some(v) = &h.capture {
            s.hotkeys.capture = validate_hotkey("capture", v)?;
        }
        let keys = [&s.hotkeys.pause, &s.hotkeys.ask, &s.hotkeys.capture];
        for i in 0..3 {
            for j in (i + 1)..3 {
                if keys[i].eq_ignore_ascii_case(keys[j]) {
                    return Err("invalid 'hotkeys': pause, ask and capture must differ".into());
                }
            }
        }
    }
    if let Some(langs) = &patch.ocr_languages {
        let langs = dedup_strings(langs);
        if langs.is_empty()
            || langs.len() > MAX_OCR_LANGUAGES
            || langs.iter().any(|l| !is_language_tag(l))
        {
            return Err(format!(
                "invalid 'ocr_languages': 1..={MAX_OCR_LANGUAGES} language tags like en-US"
            ));
        }
        s.ocr_languages = langs;
    }
    Ok(s)
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod settings_tests;
