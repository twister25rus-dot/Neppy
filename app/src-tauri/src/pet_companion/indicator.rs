//! Tray indicator for Pet observation (§2.7, D2).
//!
//! The indicator is the user-visible half of the lease invariant: the core
//! only samples while the shell keeps telling it "the indicator is visible",
//! so what this module draws must be an honest picture of what the core is
//! doing. Four things matter and they are deliberately distinguishable at a
//! glance in the menu bar:
//!
//! - observing (app, window title, selected text, clipboard) — `●`
//! - observing **and** autonomously capturing the screen (D2) — `● screen`
//! - paused — `‖`
//! - off / suspended — nothing drawn
//!
//! The pure part (`indicator_for`, `IndicatorState::from_wire`) is headless
//! and unit-tested. The Tauri-bound part (`TauriTray`) is a thin adapter:
//! `set_title` / `set_tooltip` on the existing `openhuman-tray` icon and
//! `set_text` / `set_enabled` on the pause menu item. No webview, no JS.

use std::sync::Arc;

use tauri::menu::MenuItem;
use tauri::{AppHandle, Manager};

use crate::AppRuntime;

/// Id of the tray icon created by `setup_tray` in `lib.rs`.
pub(crate) const TRAY_ID: &str = "openhuman-tray";
/// Menu id of the pause / resume item.
pub(crate) const MENU_TOGGLE_ID: &str = "tray_pet_toggle";
/// Menu id of the "Pet…" item that opens the Now tab.
pub(crate) const MENU_OPEN_ID: &str = "tray_pet_open";

pub(crate) const LABEL_PAUSE: &str = "Pause Pet observation";
pub(crate) const LABEL_RESUME: &str = "Resume Pet observation";

/// What the user should be told the companion is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndicatorState {
    /// Companion disabled (or core said `off`). Nothing drawn.
    Off,
    /// Observing app / window / selection / clipboard, not capturing the screen.
    Observing,
    /// Observing and autonomously capturing the screen for on-device OCR (D2).
    ObservingScreen,
    /// Paused by the user. Nothing is sampled.
    Paused,
    /// Enabled but not sampling (no indicator lease, missing permission, quiet
    /// hours, …). Nothing drawn: nothing is being observed.
    Suspended,
}

impl IndicatorState {
    /// True for the states in which the core is actually sampling.
    pub(crate) fn is_observing(self) -> bool {
        matches!(self, Self::Observing | Self::ObservingScreen)
    }

    /// Map the core's wire fields onto an indicator state. `enabled` is
    /// `None` where the wire shape has no such field (pause/resume return a
    /// `CompanionStatus`, whose `state:"off"` already covers it).
    ///
    /// Precedence is conservative about *claiming* observation: off beats
    /// paused beats suspended, and `ObservingScreen` needs state `observing`
    /// plus an explicit `screen_capture_active`. An unknown state string is
    /// treated as off rather than guessed at.
    pub(crate) fn from_wire(
        enabled: Option<bool>,
        state: &str,
        paused: bool,
        screen_capture_active: bool,
    ) -> Self {
        if enabled == Some(false) || state == "off" {
            return Self::Off;
        }
        if paused || state == "paused" {
            return Self::Paused;
        }
        match state {
            "suspended" => Self::Suspended,
            "observing" if screen_capture_active => Self::ObservingScreen,
            "observing" => Self::Observing,
            other => {
                log::warn!("[pet::companion::shell] unknown core state {other:?}; showing off");
                Self::Off
            }
        }
    }
}

/// Everything the tray needs to render a state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndicatorSpec {
    /// macOS menu-bar title next to the icon. `None` clears it.
    pub(crate) title: Option<&'static str>,
    /// Tooltip. `None` clears it.
    pub(crate) tooltip: Option<&'static str>,
    /// Label of the pause / resume menu item.
    pub(crate) toggle_label: &'static str,
    /// The toggle is only meaningful when the companion is enabled.
    pub(crate) toggle_enabled: bool,
}

/// Pure state → presentation mapping.
pub(crate) fn indicator_for(state: IndicatorState) -> IndicatorSpec {
    match state {
        IndicatorState::Off => IndicatorSpec {
            title: None,
            tooltip: None,
            toggle_label: LABEL_PAUSE,
            toggle_enabled: false,
        },
        IndicatorState::Observing => IndicatorSpec {
            title: Some("●"),
            tooltip: Some("Neppy Pet is observing — ⌥⇧⌘P to pause"),
            toggle_label: LABEL_PAUSE,
            toggle_enabled: true,
        },
        IndicatorState::ObservingScreen => IndicatorSpec {
            title: Some("● screen"),
            tooltip: Some(
                "Neppy Pet is observing, including your screen (read on-device, images are not kept) — ⌥⇧⌘P to pause",
            ),
            toggle_label: LABEL_PAUSE,
            toggle_enabled: true,
        },
        IndicatorState::Paused => IndicatorSpec {
            title: Some("‖"),
            tooltip: Some("Pet paused — ⌥⇧⌘P to resume"),
            toggle_label: LABEL_RESUME,
            toggle_enabled: true,
        },
        IndicatorState::Suspended => IndicatorSpec {
            title: None,
            tooltip: Some("Neppy Pet is on standby"),
            toggle_label: LABEL_PAUSE,
            toggle_enabled: true,
        },
    }
}

/// Seam over the real tray so the lease loop and the pause ordering are
/// testable without a window system.
pub(crate) trait TrayIndicator: Send + Sync {
    /// Whether a tray icon exists to draw on. Linux (no tray) and a failed
    /// `setup_tray` both answer `false`, which means no lease is ever granted.
    fn exists(&self) -> bool;
    /// Draw `spec`. `Err` means the indicator could not be shown, so the
    /// caller must not claim it is visible.
    fn apply(&self, spec: &IndicatorSpec) -> Result<(), String>;
}

/// Pause menu item kept in Tauri-managed state so the lease loop can relabel it.
pub(crate) struct PetTrayMenu(pub(crate) std::sync::Mutex<Option<MenuItem<AppRuntime>>>);

/// Create the two Pet menu items for `setup_tray` and remember the toggle.
pub(crate) fn create_menu_items(
    app: &AppHandle<AppRuntime>,
) -> tauri::Result<(MenuItem<AppRuntime>, MenuItem<AppRuntime>)> {
    let spec = indicator_for(IndicatorState::Off);
    let toggle = MenuItem::with_id(
        app,
        MENU_TOGGLE_ID,
        spec.toggle_label,
        spec.toggle_enabled,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, MENU_OPEN_ID, "Pet…", true, None::<&str>)?;
    let slot = PetTrayMenu(std::sync::Mutex::new(Some(toggle.clone())));
    if !app.manage(slot) {
        log::debug!("[pet::companion::shell] tray menu state already managed");
    }
    Ok((toggle, open))
}

/// Production tray: the `openhuman-tray` icon plus the pause menu item.
pub(crate) struct TauriTray {
    app: AppHandle<AppRuntime>,
}

impl TauriTray {
    pub(crate) fn new(app: AppHandle<AppRuntime>) -> Arc<Self> {
        Arc::new(Self { app })
    }
}

impl TrayIndicator for TauriTray {
    fn exists(&self) -> bool {
        self.app.tray_by_id(TRAY_ID).is_some()
    }

    fn apply(&self, spec: &IndicatorSpec) -> Result<(), String> {
        let tray = self
            .app
            .tray_by_id(TRAY_ID)
            .ok_or_else(|| "tray icon not found".to_string())?;
        // The menu-bar title is a macOS concept; elsewhere only the tooltip is drawn.
        #[cfg(target_os = "macos")]
        tray.set_title(spec.title)
            .map_err(|e| format!("set_title failed: {e}"))?;
        tray.set_tooltip(spec.tooltip)
            .map_err(|e| format!("set_tooltip failed: {e}"))?;
        if let Some(menu) = self.app.try_state::<PetTrayMenu>() {
            if let Some(item) = menu.0.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                // Menu item failures do not make the indicator invisible.
                if let Err(e) = item.set_text(spec.toggle_label) {
                    log::debug!("[pet::companion::shell] toggle set_text failed: {e}");
                }
                if let Err(e) = item.set_enabled(spec.toggle_enabled) {
                    log::debug!("[pet::companion::shell] toggle set_enabled failed: {e}");
                }
            }
        }
        Ok(())
    }
}
