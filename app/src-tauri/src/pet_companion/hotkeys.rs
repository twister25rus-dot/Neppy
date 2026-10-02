//! Global hotkeys for the Pet companion (§2.7, D6).
//!
//! Registered by the shell itself from the lease response, so they work
//! without React and re-register when the core's configured accelerators
//! change. Everything that decides *what* to register is pure
//! (`desired_hotkeys`, `filter_conflicts`, `hotkey_plan`) and unit-tested;
//! `ShortcutHost` is the seam over the global-shortcut plugin.
//!
//! Defaults (D6): pause `⌥⇧⌘P`, ask `⌥⇧⌘Space`, capture `⌥⇧⌘S`.

use std::collections::BTreeMap;
use std::sync::Arc;

use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use super::rpc::{HotkeyConfig, HotkeyError};
use crate::AppRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HotkeyAction {
    Pause,
    Ask,
    Capture,
}

impl HotkeyAction {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Ask => "ask",
            Self::Capture => "capture",
        }
    }
}

/// Expanded accelerator variants per action (e.g. `CmdOrCtrl` → two variants).
pub(crate) type Registered = BTreeMap<HotkeyAction, Vec<String>>;

/// Turn user-facing glyphs and aliases into plugin accelerator syntax:
/// `⌥⇧⌘P` → `Alt+Shift+Cmd+P`.
pub(crate) fn normalize_accelerator(raw: &str) -> String {
    let mut s = String::with_capacity(raw.len() + 8);
    for ch in raw.trim().chars() {
        match ch {
            '⌥' => s.push_str("Alt+"),
            '⇧' => s.push_str("Shift+"),
            '⌘' => s.push_str("Cmd+"),
            '⌃' => s.push_str("Ctrl+"),
            c => s.push(c),
        }
    }
    s.split('+')
        .map(str::trim)
        .map(|t| match t.to_ascii_lowercase().as_str() {
            "option" | "opt" => "Alt".to_string(),
            "command" => "Cmd".to_string(),
            "control" => "Ctrl".to_string(),
            _ => t.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn is_modifier(token: &str) -> bool {
    matches!(
        token.to_ascii_lowercase().as_str(),
        "ctrl" | "cmd" | "alt" | "shift" | "meta" | "super" | "cmdorctrl"
    )
}

/// Normalize, validate and expand one accelerator. Companion hotkeys must
/// carry at least one modifier, otherwise a bare letter would be swallowed
/// system-wide.
pub(crate) fn expand_accelerator(raw: &str) -> Result<Vec<String>, String> {
    let norm = normalize_accelerator(raw);
    let tokens: Vec<&str> = norm.split('+').collect();
    if tokens.iter().any(|t| t.is_empty()) {
        return Err("shortcut is empty or malformed".to_string());
    }
    if !tokens.iter().any(|t| is_modifier(t)) {
        return Err("shortcut needs at least one modifier key".to_string());
    }
    crate::ptt_hotkeys::expand_ptt_shortcuts(&norm).map_err(|e| e.to_string())
}

/// What should be registered for this lease response, before conflict
/// checks. `sensors_supported` is false off macOS (ask/capture rely on the
/// macOS sensors), in which case only pause is registered. Disabled
/// companion registers nothing.
pub(crate) fn desired_hotkeys(
    cfg: &HotkeyConfig,
    enabled: bool,
    sensors_supported: bool,
) -> (Registered, Vec<HotkeyError>) {
    let mut out = Registered::new();
    let mut errors = Vec::new();
    if !enabled {
        return (out, errors);
    }
    let entries = [
        (HotkeyAction::Pause, &cfg.pause, true),
        (HotkeyAction::Ask, &cfg.ask, sensors_supported),
        (HotkeyAction::Capture, &cfg.capture, sensors_supported),
    ];
    for (action, accel, supported) in entries {
        let Some(raw) = accel.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        if !supported {
            continue;
        }
        match expand_accelerator(raw) {
            Ok(variants) => {
                out.insert(action, variants);
            }
            Err(error) => errors.push(HotkeyError {
                name: action.name().to_string(),
                error,
            }),
        }
    }
    (out, errors)
}

fn overlaps(a: &[String], b: &[String]) -> Option<String> {
    crate::ptt_hotkeys::first_conflict_with(a, b)
}

/// Drop companion hotkeys that collide with the dictation / PTT hotkeys or
/// with an earlier companion hotkey; each drop is reported, never silent.
pub(crate) fn filter_conflicts(
    desired: Registered,
    dictation: &[String],
    ptt: &[String],
) -> (Registered, Vec<HotkeyError>) {
    let mut accepted = Registered::new();
    let mut errors = Vec::new();
    for (action, variants) in desired {
        let reason = if let Some(c) = overlaps(&variants, dictation) {
            Some(format!("'{c}' conflicts with the dictation hotkey"))
        } else if let Some(c) = overlaps(&variants, ptt) {
            Some(format!("'{c}' conflicts with the push-to-talk hotkey"))
        } else {
            accepted.iter().find_map(|(other, other_variants)| {
                overlaps(&variants, other_variants)
                    .map(|c| format!("'{c}' duplicates the {} hotkey", other.name()))
            })
        };
        match reason {
            Some(error) => errors.push(HotkeyError {
                name: action.name().to_string(),
                error,
            }),
            None => {
                accepted.insert(action, variants);
            }
        }
    }
    (accepted, errors)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActionPlan {
    pub(crate) action: HotkeyAction,
    pub(crate) unregister: Vec<String>,
    pub(crate) register: Vec<String>,
}

/// Diff what is registered against what is wanted. Unchanged actions
/// (case-insensitively) produce no entry.
pub(crate) fn hotkey_plan(old: &Registered, new: &Registered) -> Vec<ActionPlan> {
    let lc =
        |v: &Vec<String>| -> Vec<String> { v.iter().map(|s| s.to_ascii_lowercase()).collect() };
    let mut plan = Vec::new();
    for action in [
        HotkeyAction::Pause,
        HotkeyAction::Ask,
        HotkeyAction::Capture,
    ] {
        let (o, n) = (old.get(&action), new.get(&action));
        if o.map(lc) == n.map(lc) {
            continue;
        }
        plan.push(ActionPlan {
            action,
            unregister: o.cloned().unwrap_or_default(),
            register: n.cloned().unwrap_or_default(),
        });
    }
    plan
}

pub(crate) type PressHandler = Arc<dyn Fn() + Send + Sync>;

/// Seam over the global-shortcut plugin and the already-registered
/// dictation / PTT state.
pub(crate) trait ShortcutHost: Send + Sync {
    fn register(&self, accelerator: &str, on_press: PressHandler) -> Result<(), String>;
    fn unregister(&self, accelerator: &str) -> Result<(), String>;
    fn dictation_current(&self) -> Vec<String>;
    fn ptt_current(&self) -> Vec<String>;
}

/// Execute a plan: unregister first, then register; a variant that fails to
/// register rolls its siblings back and is reported. `registered` and
/// `errors` are updated in place.
pub(crate) fn apply_plan(
    host: &dyn ShortcutHost,
    plan: &[ActionPlan],
    handler_for: &dyn Fn(HotkeyAction) -> PressHandler,
    registered: &mut Registered,
    errors: &mut Vec<HotkeyError>,
) {
    for step in plan {
        for old in &step.unregister {
            if let Err(e) = host.unregister(old) {
                log::warn!("[pet::companion::shell] unregister {old} failed: {e}");
            }
        }
        registered.remove(&step.action);
        let mut done: Vec<String> = Vec::new();
        let mut failed: Option<String> = None;
        for variant in &step.register {
            match host.register(variant, handler_for(step.action)) {
                Ok(()) => done.push(variant.clone()),
                Err(e) => {
                    failed = Some(format!("could not register '{variant}': {e}"));
                    break;
                }
            }
        }
        match failed {
            Some(error) => {
                for v in &done {
                    let _ = host.unregister(v);
                }
                log::warn!(
                    "[pet::companion::shell] hotkey {} not registered: {error}",
                    step.action.name()
                );
                errors.push(HotkeyError {
                    name: step.action.name().to_string(),
                    error,
                });
            }
            None if !done.is_empty() => {
                log::info!(
                    "[pet::companion::shell] hotkey {} registered ({})",
                    step.action.name(),
                    done.join(", ")
                );
                registered.insert(step.action, done);
            }
            None => {}
        }
    }
}

/// Production host backed by `tauri-plugin-global-shortcut`.
pub(crate) struct TauriShortcutHost {
    app: AppHandle<AppRuntime>,
}

impl TauriShortcutHost {
    pub(crate) fn new(app: AppHandle<AppRuntime>) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl ShortcutHost for TauriShortcutHost {
    fn register(&self, accelerator: &str, on_press: PressHandler) -> Result<(), String> {
        self.app
            .global_shortcut()
            .on_shortcut(accelerator, move |_app, _sc, event| {
                if event.state == ShortcutState::Pressed {
                    on_press();
                }
            })
            .map_err(|e| e.to_string())
    }

    fn unregister(&self, accelerator: &str) -> Result<(), String> {
        self.app
            .global_shortcut()
            .unregister(accelerator)
            .map_err(|e| e.to_string())
    }

    fn dictation_current(&self) -> Vec<String> {
        self.app
            .try_state::<crate::dictation_hotkeys::DictationHotkeyState>()
            .map(|s| s.0.lock().unwrap_or_else(|p| p.into_inner()).clone())
            .unwrap_or_default()
    }

    fn ptt_current(&self) -> Vec<String> {
        self.app
            .try_state::<crate::ptt_hotkeys::PttHotkeyState>()
            .map(|s| s.shortcut.lock().unwrap_or_else(|p| p.into_inner()).clone())
            .unwrap_or_default()
    }
}
