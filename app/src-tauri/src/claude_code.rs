//! Tauri commands for the Claude Code CLI provider.
//!
//! Provides a cross-platform "open a terminal and run `claude login`"
//! helper. The CLI's OAuth flow is interactive (it prints a URL and
//! waits for the user to paste a code), so we can't host it in-app — we
//! detach into the user's native terminal so they complete login there,
//! then return to Neppy and click Recheck in the settings card.

use std::process::Command;

/// Open the user's native terminal and run `claude login` inside it.
///
/// Returns the name of the terminal emulator we launched (for UI
/// confirmation) or an error string if no terminal could be opened.
///
/// Platform behaviour:
///   - Windows: `cmd /c start "" cmd /k claude login`
///   - macOS:   `osascript` → Terminal.app `do script "claude login"`
///   - Linux:   try `x-terminal-emulator`, then `gnome-terminal`,
///              `konsole`, `xterm` in that order
#[tauri::command]
pub fn claude_code_login_launch() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        // `start ""` opens a new console window; the empty quoted title
        // prevents cmd from interpreting the first arg as a title.
        // `cmd /k` keeps the window open after `claude login` exits so
        // the user can read any final output.
        Command::new("cmd")
            .args(["/c", "start", "", "cmd", "/k", "claude login"])
            .spawn()
            .map_err(|e| format!("failed to open cmd: {e}"))?;
        return Ok("cmd".into());
    }

    #[cfg(target_os = "macos")]
    {
        // The resolved path, not the bare name. Terminal runs a login shell so
        // `claude` is usually on its PATH, but the app found a specific binary
        // and a second, different one in the user's shell would sign the wrong
        // install in — which is precisely the state that made this feature look
        // broken in the first place.
        let program = neppy_core::neppy::inference::provider::claude_code::resolved_cli_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "claude".to_string());
        // AppleScript string literal: a path with a quote or backslash would
        // otherwise end the literal and change the command.
        let quoted = program.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!(
            "tell application \"Terminal\"\n    activate\n    do script \"{quoted} login\"\nend tell"
        );

        // Wait for it. `spawn` alone returns Ok as soon as the process starts,
        // so an Automation (TCC) denial — this app is ad-hoc signed, which is
        // exactly when that happens — produced a success here and a window that
        // never appeared.
        let output = Command::new("osascript")
            .args(["-e", &script])
            .output()
            .map_err(|e| format!("failed to run osascript: {e}"))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            let detail = detail.trim();
            return Err(if detail.is_empty() {
                format!("Terminal.app did not open (osascript exited {})", output.status)
            } else {
                format!("Terminal.app did not open: {detail}")
            });
        }
        Ok("Terminal.app".into())
    }

    #[cfg(target_os = "linux")]
    {
        let terminals: &[(&str, &[&str])] = &[
            ("x-terminal-emulator", &["-e", "claude", "login"]),
            ("gnome-terminal", &["--", "claude", "login"]),
            ("konsole", &["-e", "claude", "login"]),
            ("xfce4-terminal", &["-e", "claude login"]),
            ("xterm", &["-e", "claude", "login"]),
        ];
        for (term, args) in terminals {
            match Command::new(term).args(*args).spawn() {
                Ok(_) => return Ok(term.to_string()),
                Err(_) => continue,
            }
        }
        Err("no terminal emulator found (tried x-terminal-emulator, gnome-terminal, konsole, xfce4-terminal, xterm). Run `claude login` manually.".into())
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err("claude_code_login_launch is not supported on this platform".into())
    }
}
