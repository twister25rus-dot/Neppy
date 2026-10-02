//! App and window-title exclusions. Evaluated BEFORE any text is read: the
//! sampler learns the frontmost app, calls [`Exclusions::check_app`], and only
//! then reads the title ([`Exclusions::check_title`]) and selection.
//!
//! App rules match a bundle id (case-insensitive exact, or a trailing `*`
//! prefix wildcard such as `com.jetbrains.*`) or an exact app name. Title rules
//! are case-insensitive substrings, or `re:`-prefixed regexes (validated at
//! save time). Neppy's own bundle id is always excluded.

use regex::{Regex, RegexBuilder};

use super::types::DropReason;

/// Neppy's own bundle id: never observed, not removable.
pub const NEPPY_BUNDLE_ID: &str = "com.neppy.app";
pub const MAX_RULES: usize = 100;
pub const MAX_RULE_CHARS: usize = 200;
const REGEX_SIZE_LIMIT: usize = 1 << 20;

enum AppRule {
    Exact(String),
    Prefix(String),
}

enum TitleRule {
    Substring(String),
    Pattern(Regex),
}

/// Compiled exclusion rules.
pub struct Exclusions {
    apps: Vec<AppRule>,
    titles: Vec<TitleRule>,
}

fn check_rule_text(field: &str, rule: &str) -> Result<(), String> {
    let t = rule.trim();
    if t.is_empty() {
        return Err(format!("invalid '{field}': empty rule"));
    }
    if rule.chars().count() > MAX_RULE_CHARS {
        return Err(format!(
            "invalid '{field}': a rule is longer than {MAX_RULE_CHARS} characters"
        ));
    }
    if rule.chars().any(char::is_control) {
        return Err(format!(
            "invalid '{field}': a rule contains control characters"
        ));
    }
    Ok(())
}

fn compile_regex(pattern: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern)
        .case_insensitive(true)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
}

/// Validate user rules; errors are field-named.
pub fn validate_rules(apps: &[String], title_patterns: &[String]) -> Result<(), String> {
    if apps.len() > MAX_RULES {
        return Err(format!(
            "invalid 'excluded_apps': at most {MAX_RULES} rules"
        ));
    }
    if title_patterns.len() > MAX_RULES {
        return Err(format!(
            "invalid 'excluded_title_patterns': at most {MAX_RULES} rules"
        ));
    }
    for a in apps {
        check_rule_text("excluded_apps", a)?;
    }
    for t in title_patterns {
        check_rule_text("excluded_title_patterns", t)?;
        if let Some(re) = t.trim().strip_prefix("re:") {
            compile_regex(re)
                .map_err(|e| format!("invalid 'excluded_title_patterns': bad regex '{re}': {e}"))?;
        }
    }
    Ok(())
}

impl Exclusions {
    /// Compile rules. Fails on the first invalid rule (use [`validate_rules`]
    /// first at the boundary).
    pub fn compile(apps: &[String], title_patterns: &[String]) -> Result<Self, String> {
        validate_rules(apps, title_patterns)?;
        let mut app_rules = vec![AppRule::Exact(NEPPY_BUNDLE_ID.to_ascii_lowercase())];
        for a in apps {
            let a = a.trim().to_lowercase();
            match a.strip_suffix('*') {
                Some(prefix) if !prefix.is_empty() => {
                    app_rules.push(AppRule::Prefix(prefix.into()))
                }
                _ => app_rules.push(AppRule::Exact(a)),
            }
        }
        let mut title_rules = Vec::new();
        for t in title_patterns {
            let t = t.trim();
            if let Some(re) = t.strip_prefix("re:") {
                title_rules
                    .push(TitleRule::Pattern(compile_regex(re).map_err(|e| {
                        format!("invalid 'excluded_title_patterns': {e}")
                    })?));
            } else {
                title_rules.push(TitleRule::Substring(t.to_lowercase()));
            }
        }
        Ok(Self {
            apps: app_rules,
            titles: title_rules,
        })
    }

    /// Compile from settings; invalid stored rules are skipped (logged without
    /// content) rather than disabling the whole list.
    pub fn from_settings(settings: &super::settings::CompanionSettings) -> Self {
        match Self::compile(&settings.excluded_apps, &settings.excluded_title_patterns) {
            Ok(e) => e,
            Err(err) => {
                log::warn!(
                    "[pet::companion] exclusion rules invalid, using per-rule fallback: {err}"
                );
                let apps: Vec<String> = settings
                    .excluded_apps
                    .iter()
                    .filter(|a| check_rule_text("excluded_apps", a).is_ok())
                    .cloned()
                    .collect();
                let titles: Vec<String> = settings
                    .excluded_title_patterns
                    .iter()
                    .filter(|t| validate_rules(&[], std::slice::from_ref(*t)).is_ok())
                    .cloned()
                    .collect();
                Self::compile(&apps, &titles).unwrap_or_else(|_| {
                    Self::compile(&[], &[]).expect("empty rules always compile")
                })
            }
        }
    }

    /// App-level check (before reading any text).
    pub fn check_app(&self, bundle_id: Option<&str>, app_name: &str) -> Option<DropReason> {
        let bundle = bundle_id.map(str::to_lowercase);
        let name = app_name.trim().to_lowercase();
        let hit = self.apps.iter().any(|rule| match rule {
            AppRule::Exact(x) => bundle.as_deref() == Some(x.as_str()) || name == *x,
            AppRule::Prefix(p) => bundle.as_deref().is_some_and(|b| b.starts_with(p.as_str())),
        });
        hit.then_some(DropReason::ExcludedApp)
    }

    /// Window-title check.
    pub fn check_title(&self, title: &str) -> Option<DropReason> {
        let lower = title.to_lowercase();
        let hit = self.titles.iter().any(|rule| match rule {
            TitleRule::Substring(s) => lower.contains(s.as_str()),
            TitleRule::Pattern(re) => re.is_match(title),
        });
        hit.then_some(DropReason::TitleRule)
    }

    /// App rules first, then the title rules.
    pub fn excluded(
        &self,
        bundle_id: Option<&str>,
        app_name: &str,
        title: Option<&str>,
    ) -> Option<DropReason> {
        self.check_app(bundle_id, app_name)
            .or_else(|| title.and_then(|t| self.check_title(t)))
    }
}

#[cfg(test)]
#[path = "exclusions_tests.rs"]
mod exclusions_tests;
