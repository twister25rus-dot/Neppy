# Accessibility

Cross-platform accessibility middleware. Owns macOS AX / CGEvent / IOKit FFI, the unified Swift helper-process bridge, focused-text inspection, system-permission detection (Accessibility, Input Monitoring, Microphone), the Globe-key listener, the floating overlay window, paste / backspace key synthesis, terminal heuristics, and AX-string normalization. Centralises platform-specific code so that `voice` never touches FFI directly.

## Public surface

- `pub fn focused_text_context` / `focused_text_context_verbose` / `validate_focused_target` — `focus.rs` — query the OS for the currently focused text field.
- `pub fn globe_listener_start` / `globe_listener_stop` / `globe_listener_poll` / `pub struct GlobeHotkeyPollResult` / `pub enum GlobeHotkeyStatus` — `globe.rs` — macOS Globe-key (Fn) hotkey monitor.
- `pub fn precompile_helper_background` — `helper.rs` — warm the Swift helper process at startup.
- `pub fn any_modifier_down` / `is_escape_key_down` / `is_tab_key_down` — `keys.rs` — modifier polling for cancellation gestures.
- `pub fn show_overlay` / `hide_overlay` / `quit_overlay` — `overlay.rs` — floating completion overlay control.
- `pub fn apply_text_to_focused_field` / `pub fn send_backspace` — `paste.rs` — programmatic text insertion.
- Permission detection: `detect_permissions`, `detect_microphone_permission`, `microphone_denied_message`, `permission_to_str`, `request_microphone_access` (cross-platform); macOS-only `detect_accessibility_permission`, `detect_input_monitoring_permission`, `open_macos_privacy_pane`, `request_accessibility_access` — `permissions.rs`.
- `pub fn extract_terminal_input_context` / `is_terminal_app` / `is_text_role` / `looks_like_terminal_buffer` — `terminal.rs` — terminal-window heuristics.
- `pub fn normalize_ax_value` / `parse_ax_number` / `truncate_tail` — `text_util.rs` — AX value normalization.
- `pub struct ElementBounds` / `FocusedTextContext` / `PermissionKind` / `PermissionState` / `PermissionStatus` — `types.rs`.

## Pet Mode sensors

Side-effect-free sensing for the Pet Mode companion. All blocking; call from `spawn_blocking`. Non-macOS builds return `SensorError::Unsupported`.

- `frontmost_snapshot(SnapshotOpts) -> Result<FrontmostSnapshot, SensorError>` / `secure_entry_active` / `frontmost_pid` / `bundle_dir_from_exe_path` — `frontmost.rs` — native AX FFI (0.25 s messaging timeout). Never reads `AXValue`, never sets `AXEnhancedUserInterface`. Selection is read only when requested and no secure field / secure event input is active.
- `clipboard_peek` (changeCount only) / `clipboard_read(max_chars)` — `clipboard.rs` — via the Swift helper; concealed / transient / auto-generated pasteboards (password managers) yield no text (checked in the helper and again host-side).
- `capture_region_interactive(timeout, &OcrOpts)` — `capture.rs` — user drags a region (`screencapture -i -x`), OCR'd on-device, image deleted at once.
- `ScreenWatcher::observe(&AutonomousCaptureOpts)` — `capture.rs` — autonomous silent capture (`screencapture -x -o -l <window>` / `-m`) after Screen Recording is granted; downscaled-grid change detection (`change_detector.rs`) so Vision OCR (`ocr.rs`) runs only when the frame changed. Images live in a 0600 `TempPng` deleted on drop (also on error / panic) and are never returned: only `OcrText` leaves.
- `detect_screen_recording_permission` / `request_screen_recording_access` (prompts at most once per process) — `permissions.rs`.
- `helper_sensors_swift.rs` holds the Swift commands (`clipboard_peek`, `clipboard_read`, `frontmost_window`, `frame_signature`, `ocr`) spliced into the helper; `helper_send_receive_with_timeout` gives them per-call deadlines (1.5 s / OCR 15 s) that also bound the wait for the helper.
- OCR languages are always explicit (default `en-US`): `automaticallyDetectsLanguage` measured ~19 s on a Retina frame versus ~0.2 s with an explicit list.

## Calls into

- macOS frameworks (`ApplicationServices`, `CoreGraphics`, `IOKit`, `AVFoundation`) via FFI.
- Bundled Swift helper process for AX queries that require a separate process.
- `src/neppy/config/` — overlay sizing and helper paths (light dependency).

## Called by

- `src/neppy/voice/` — microphone permission and focused-text helpers (indirect, via re-exports).

## Tests

- Permission and focus coverage runs through `permissions_tests.rs`, inline module tests, and retained consumers.
- AX FFI surface is best validated end-to-end on a real macOS host — most CI runs are Linux and skip platform-gated paths.
