//! Side-effect-free frontmost-app snapshot for Pet Mode (native AX FFI, no subprocess).
//!
//! Hot path: one call is a handful of AX attribute reads with a 0.25 s messaging
//! timeout, so a hung app cannot stall the sampler.
//!
//! Hard rules (pinned by tests and review):
//! - it never reads `kAXValueAttribute` (no whole-field or terminal-buffer reads);
//! - it never sets `AXEnhancedUserInterface` / `AXManualAccessibility`;
//! - selected text is read only when requested **and** the focused element is not a
//!   secure text field and secure event input is off (password entry).
//!
//! Without Accessibility permission every AX read fails; the function returns
//! [`SensorError::PermissionMissing`]. There is deliberately no NSWorkspace fallback.
// Parts are only reached from macOS-only code paths; the rest is tested everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use super::types::SensorError;
use std::path::{Path, PathBuf};

/// What the user is looking at right now. Contains no field contents beyond the
/// optional, capped selection.
#[derive(Debug, Clone, PartialEq)]
pub struct FrontmostSnapshot {
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub pid: i32,
    pub window_title: Option<String>,
    pub focused_role: Option<String>,
    pub focused_subrole: Option<String>,
    /// Focused element is an `AXSecureTextField`, or system secure event input is on
    /// (which also covers browser password fields that do not expose AX roles).
    pub is_secure_field: bool,
    /// Raw secure-event-input state, reported separately so a policy can tell the
    /// two signals apart.
    pub secure_input_active: bool,
    pub selected_text: Option<String>,
    /// Seconds since the last keyboard/mouse event in this session.
    pub idle_secs: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotOpts {
    pub want_selection: bool,
    pub max_selection_chars: usize,
}

impl Default for SnapshotOpts {
    fn default() -> Self {
        Self {
            want_selection: true,
            max_selection_chars: 2_000,
        }
    }
}

/// Innermost `*.app` directory containing the executable at `exe_path`.
///
/// `/Applications/Foo.app/Contents/MacOS/Foo` gives `/Applications/Foo.app`; a nested
/// helper `.../Foo.app/Contents/Frameworks/Bar.app/Contents/MacOS/Bar` gives
/// `.../Bar.app`. A path with no `.app` ancestor gives `None`.
pub fn bundle_dir_from_exe_path(exe_path: &Path) -> Option<PathBuf> {
    exe_path
        .ancestors()
        .skip(1)
        .find(|dir| {
            dir.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.len() > 4 && n.to_ascii_lowercase().ends_with(".app"))
                .unwrap_or(false)
        })
        .map(Path::to_path_buf)
}

/// True for the AX role/subrole that marks a password field.
pub(super) fn is_secure_role(role: Option<&str>, subrole: Option<&str>) -> bool {
    role == Some("AXSecureTextField") || subrole == Some("AXSecureTextField")
}

#[cfg(target_os = "macos")]
pub fn frontmost_snapshot(opts: SnapshotOpts) -> Result<FrontmostSnapshot, SensorError> {
    native::snapshot(opts)
}

/// Cheap guard for the capture path: is a secure field focused or secure event input on?
#[cfg(target_os = "macos")]
pub fn secure_entry_active() -> Result<bool, SensorError> {
    native::secure_entry_active()
}

/// Frontmost process id only (for "did the target change" checks).
#[cfg(target_os = "macos")]
pub fn frontmost_pid() -> Result<i32, SensorError> {
    native::frontmost_pid()
}

#[cfg(not(target_os = "macos"))]
pub fn frontmost_snapshot(_opts: SnapshotOpts) -> Result<FrontmostSnapshot, SensorError> {
    Err(SensorError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
pub fn secure_entry_active() -> Result<bool, SensorError> {
    Err(SensorError::Unsupported)
}

#[cfg(not(target_os = "macos"))]
pub fn frontmost_pid() -> Result<i32, SensorError> {
    Err(SensorError::Unsupported)
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use once_cell::sync::{Lazy, OnceCell};
    use std::collections::HashMap;
    use std::ffi::{c_char, c_void};
    use std::sync::Mutex;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type AXUIElementRef = *const c_void;
    type AXError = i32;

    const AX_SUCCESS: AXError = 0;
    const AX_ERR_CANNOT_COMPLETE: AXError = -25204;
    const AX_ERR_API_DISABLED: AXError = -25211;
    const AX_ERR_NO_VALUE: AXError = -25212;

    const UTF8: u32 = 0x0800_0100;
    const MESSAGING_TIMEOUT_SECS: f32 = 0.25;
    const TITLE_MAX_CHARS: usize = 512;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CFRange {
        location: isize,
        length: isize,
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, secs: f32) -> AXError;
        fn AXUIElementCopyAttributeValue(
            el: AXUIElementRef,
            attr: CFStringRef,
            out: *mut CFTypeRef,
        ) -> AXError;
        fn AXUIElementGetPid(el: AXUIElementRef, pid: *mut i32) -> AXError;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        static kCFAllocatorDefault: *const c_void;
        fn CFRelease(cf: CFTypeRef);
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            s: *const c_char,
            enc: u32,
        ) -> CFStringRef;
        fn CFStringGetLength(s: CFStringRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(len: isize, enc: u32) -> isize;
        fn CFStringGetBytes(
            s: CFStringRef,
            range: CFRange,
            enc: u32,
            loss_byte: u8,
            is_external: bool,
            buf: *mut u8,
            max_buf: isize,
            used: *mut isize,
        ) -> isize;
        fn CFURLCreateFromFileSystemRepresentation(
            alloc: *const c_void,
            buf: *const u8,
            len: isize,
            is_dir: bool,
        ) -> *const c_void;
        fn CFBundleCreate(alloc: *const c_void, url: *const c_void) -> *const c_void;
        fn CFBundleGetIdentifier(bundle: *const c_void) -> CFStringRef;
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFTypeRef;
    }

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn IsSecureEventInputEnabled() -> bool;
    }

    /// Owned CoreFoundation object, released on drop.
    struct Cf(CFTypeRef);
    impl Cf {
        fn new(p: CFTypeRef) -> Option<Self> {
            (!p.is_null()).then_some(Self(p))
        }
    }
    impl Drop for Cf {
        fn drop(&mut self) {
            unsafe { CFRelease(self.0) }
        }
    }

    struct Attrs {
        focused_app: usize,
        title: usize,
        focused_window: usize,
        focused_ui: usize,
        role: usize,
        subrole: usize,
        selected_text: usize,
    }

    static ATTRS: OnceCell<Attrs> = OnceCell::new();

    /// Attribute-name CFStrings, created once and kept for the process lifetime.
    fn attrs() -> &'static Attrs {
        ATTRS.get_or_init(|| {
            let mk = |s: &str| {
                let c = std::ffi::CString::new(s).expect("static attribute name");
                unsafe { CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), UTF8) as usize }
            };
            Attrs {
                focused_app: mk("AXFocusedApplication"),
                title: mk("AXTitle"),
                focused_window: mk("AXFocusedWindow"),
                focused_ui: mk("AXFocusedUIElement"),
                role: mk("AXRole"),
                subrole: mk("AXSubrole"),
                selected_text: mk("AXSelectedText"),
            }
        })
    }

    fn copy_attr(el: &Cf, attr: usize) -> Result<Cf, AXError> {
        let mut out: CFTypeRef = std::ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(el.0, attr as CFStringRef, &mut out) };
        if err != AX_SUCCESS {
            return Err(err);
        }
        Cf::new(out).ok_or(AX_ERR_NO_VALUE)
    }

    /// Read a string attribute, capped at `max_chars` UTF-16 units. Non-string
    /// values (e.g. a missing value placeholder) yield `None`.
    fn copy_string(el: &Cf, attr: usize, max_chars: usize) -> Option<String> {
        let v = copy_attr(el, attr).ok()?;
        unsafe {
            if CFGetTypeID(v.0) != CFStringGetTypeID() {
                return None;
            }
            let len = CFStringGetLength(v.0).max(0).min(max_chars as isize);
            if len == 0 {
                return Some(String::new());
            }
            let cap = CFStringGetMaximumSizeForEncoding(len, UTF8).max(0) + 1;
            let mut buf = vec![0u8; cap as usize];
            let mut used: isize = 0;
            let n = CFStringGetBytes(
                v.0,
                CFRange {
                    location: 0,
                    length: len,
                },
                UTF8,
                b'?',
                false,
                buf.as_mut_ptr(),
                cap,
                &mut used,
            );
            if n <= 0 {
                return None;
            }
            buf.truncate(used.max(0) as usize);
            Some(String::from_utf8_lossy(&buf).into_owned())
        }
    }

    fn map_ax_error(e: AXError) -> SensorError {
        match e {
            AX_ERR_API_DISABLED => SensorError::PermissionMissing,
            AX_ERR_NO_VALUE => SensorError::NoFocusedApp,
            AX_ERR_CANNOT_COMPLETE => SensorError::Timeout,
            other => SensorError::Failed(format!("ax error {other}")),
        }
    }

    /// System-wide AX queries fail with `kAXErrorCannotComplete` in a process that has no
    /// WindowServer connection yet (a CLI, a test binary). The Tauri app already has one;
    /// for everything else one window-list call establishes it (verified on macOS 27:
    /// CGMainDisplayID / CGSessionCopyCurrentDictionary do not). Runs once, ids only.
    fn ensure_window_server_connection() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| unsafe {
            // kCGWindowListOptionOnScreenOnly = 1, kCGNullWindowID = 0
            if let Some(list) = Cf::new(CGWindowListCopyWindowInfo(1, 0)) {
                drop(list);
            }
        });
    }

    fn focused_app() -> Result<Cf, SensorError> {
        if !unsafe { AXIsProcessTrusted() } {
            return Err(SensorError::PermissionMissing);
        }
        ensure_window_server_connection();
        let sys = Cf::new(unsafe { AXUIElementCreateSystemWide() })
            .ok_or_else(|| SensorError::Failed("system-wide element unavailable".into()))?;
        unsafe { AXUIElementSetMessagingTimeout(sys.0, MESSAGING_TIMEOUT_SECS) };
        let app = copy_attr(&sys, attrs().focused_app).map_err(map_ax_error)?;
        unsafe { AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT_SECS) };
        Ok(app)
    }

    pub(super) fn frontmost_pid() -> Result<i32, SensorError> {
        let app = focused_app()?;
        pid_of(&app)
    }

    fn pid_of(app: &Cf) -> Result<i32, SensorError> {
        let mut pid: i32 = 0;
        let err = unsafe { AXUIElementGetPid(app.0, &mut pid) };
        if err != AX_SUCCESS {
            return Err(map_ax_error(err));
        }
        Ok(pid)
    }

    fn focused_element(app: &Cf) -> Option<Cf> {
        let el = copy_attr(app, attrs().focused_ui).ok()?;
        unsafe { AXUIElementSetMessagingTimeout(el.0, MESSAGING_TIMEOUT_SECS) };
        Some(el)
    }

    pub(super) fn secure_entry_active() -> Result<bool, SensorError> {
        let secure_input = unsafe { IsSecureEventInputEnabled() };
        let app = focused_app()?;
        let a = attrs();
        let (role, subrole) = match focused_element(&app) {
            Some(el) => (
                copy_string(&el, a.role, 64),
                copy_string(&el, a.subrole, 64),
            ),
            None => (None, None),
        };
        Ok(secure_input || is_secure_role(role.as_deref(), subrole.as_deref()))
    }

    fn exe_path(pid: i32) -> Option<PathBuf> {
        let mut buf = vec![0u8; 4096];
        let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut c_void, 4096) };
        if n <= 0 {
            return None;
        }
        buf.truncate(n as usize);
        Some(PathBuf::from(String::from_utf8_lossy(&buf).into_owned()))
    }

    static BUNDLE_ID_CACHE: Lazy<Mutex<HashMap<PathBuf, Option<String>>>> =
        Lazy::new(|| Mutex::new(HashMap::new()));

    fn bundle_id_of_dir(dir: &Path) -> Option<String> {
        use std::os::unix::ffi::OsStrExt;
        let bytes = dir.as_os_str().as_bytes();
        unsafe {
            let url = Cf::new(CFURLCreateFromFileSystemRepresentation(
                kCFAllocatorDefault,
                bytes.as_ptr(),
                bytes.len() as isize,
                true,
            ))?;
            let bundle = Cf::new(CFBundleCreate(kCFAllocatorDefault, url.0))?;
            // Get rule: owned by the bundle, copied out before the bundle is released.
            let id = CFBundleGetIdentifier(bundle.0);
            if id.is_null() {
                return None;
            }
            let len = CFStringGetLength(id).clamp(0, 256);
            let cap = CFStringGetMaximumSizeForEncoding(len, UTF8).max(0) + 1;
            let mut buf = vec![0u8; cap as usize];
            let mut used: isize = 0;
            let n = CFStringGetBytes(
                id,
                CFRange {
                    location: 0,
                    length: len,
                },
                UTF8,
                b'?',
                false,
                buf.as_mut_ptr(),
                cap,
                &mut used,
            );
            if n <= 0 {
                return None;
            }
            buf.truncate(used.max(0) as usize);
            Some(String::from_utf8_lossy(&buf).into_owned())
        }
    }

    /// Cached by bundle directory (not pid), so a recycled pid cannot return a stale id.
    fn bundle_id_for(exe: &Path) -> Option<String> {
        let dir = bundle_dir_from_exe_path(exe)?;
        if let Ok(cache) = BUNDLE_ID_CACHE.lock() {
            if let Some(hit) = cache.get(&dir) {
                return hit.clone();
            }
        }
        let id = bundle_id_of_dir(&dir);
        if let Ok(mut cache) = BUNDLE_ID_CACHE.lock() {
            if cache.len() >= 256 {
                cache.clear();
            }
            cache.insert(dir, id.clone());
        }
        id
    }

    pub(super) fn snapshot(opts: SnapshotOpts) -> Result<FrontmostSnapshot, SensorError> {
        let app = focused_app()?;
        let a = attrs();
        let pid = pid_of(&app)?;
        let exe = exe_path(pid);
        let bundle_id = exe.as_deref().and_then(bundle_id_for);

        let mut app_name = copy_string(&app, a.title, TITLE_MAX_CHARS).unwrap_or_default();
        if app_name.is_empty() {
            app_name = exe
                .as_deref()
                .and_then(|e| bundle_dir_from_exe_path(e).or_else(|| Some(e.to_path_buf())))
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "unknown".to_string());
        }

        let window_title = copy_attr(&app, a.focused_window)
            .ok()
            .and_then(|w| {
                unsafe { AXUIElementSetMessagingTimeout(w.0, MESSAGING_TIMEOUT_SECS) };
                copy_string(&w, a.title, TITLE_MAX_CHARS)
            })
            .filter(|t| !t.is_empty());

        let (focused_role, focused_subrole, element) = match focused_element(&app) {
            Some(el) => (
                copy_string(&el, a.role, 64),
                copy_string(&el, a.subrole, 64),
                Some(el),
            ),
            None => (None, None, None),
        };
        let secure_input_active = unsafe { IsSecureEventInputEnabled() };
        let is_secure_field = secure_input_active
            || is_secure_role(focused_role.as_deref(), focused_subrole.as_deref());

        // Selection only: never the whole value. Skipped for any secure context.
        let selected_text = if opts.want_selection && !is_secure_field {
            element
                .as_ref()
                .and_then(|el| copy_string(el, a.selected_text, opts.max_selection_chars))
                .filter(|t| !t.is_empty())
        } else {
            None
        };

        // 0 = kCGEventSourceStateCombinedSessionState, !0 = kCGAnyInputEventType.
        let idle_secs = unsafe { CGEventSourceSecondsSinceLastEventType(0, u32::MAX) };

        log::trace!(
            "[accessibility][sensors] frontmost pid={pid} bundle={:?} role={:?} secure={is_secure_field} sel_chars={}",
            bundle_id,
            focused_role,
            selected_text.as_ref().map(|s| s.chars().count()).unwrap_or(0)
        );
        Ok(FrontmostSnapshot {
            app_name,
            bundle_id,
            pid,
            window_title,
            focused_role,
            focused_subrole,
            is_secure_field,
            secure_input_active,
            selected_text,
            idle_secs: if idle_secs.is_finite() {
                idle_secs.max(0.0)
            } else {
                0.0
            },
        })
    }
}

#[cfg(test)]
#[path = "frontmost_tests.rs"]
mod tests;
