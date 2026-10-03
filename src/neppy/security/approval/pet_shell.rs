//! Pet companion (user decision D4): the high-risk class of a `shell` call.
//!
//! The gate sees the *redacted* `command` (home paths scrubbed, otherwise
//! intact). It is lexed with shell quoting rules into simple commands, each
//! classified by its base command and the few flags that change what it does.
//! Anything the lexer cannot read unambiguously parks: an unbalanced quote, a
//! dangling redirect, hidden execution (`$(…)`, backticks, `<(…)`), a command
//! named by a variable, or an interpreter / wrapper that runs code the gate
//! cannot inspect (`bash -c`, `python3 -c`, `xargs`, `env rm`, `sudo`, …).

use std::sync::OnceLock;

use crate::neppy::pet::companion::types::ActionCategory;
use crate::neppy::security::{CommandClass, SecurityPolicy};

/// The class of a `shell` call from its (path-scrubbed, otherwise intact)
/// `command` argument, or `None` when the call is ordinary. See the table on
/// [`super::pet_classifier::pet_companion_high_risk`].
pub(super) fn pet_companion_shell_class(args: &serde_json::Value) -> Option<ActionCategory> {
    let command = args
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    if command.is_empty() {
        return Some(ActionCategory::PrivilegedCommand);
    }
    // Hidden execution runs an inner command no classification can see.
    if ["`", "$(", "<(", ">("].iter().any(|p| command.contains(p)) {
        return Some(ActionCategory::PrivilegedCommand);
    }
    let Some(segments) = lex(command) else {
        tracing::debug!("[approval::pet_shell] ambiguous shell command — parking");
        return Some(ActionCategory::PrivilegedCommand);
    };
    // The first high-risk class found is reported; any hit already parks.
    if let Some(category) = segments.iter().find_map(classify_segment) {
        return Some(category);
    }

    static POLICY: OnceLock<SecurityPolicy> = OnceLock::new();
    let mut class = POLICY
        .get_or_init(SecurityPolicy::default)
        .classify_command(command);
    if let Some(declared) = args
        .get("category")
        .and_then(serde_json::Value::as_str)
        .and_then(SecurityPolicy::parse_declared_class)
    {
        class = class.max(declared);
    }
    match class {
        CommandClass::Destructive => Some(ActionCategory::PrivilegedCommand),
        CommandClass::Install => Some(ActionCategory::Install),
        CommandClass::Network => Some(ActionCategory::SharePersonalInfo),
        CommandClass::Write | CommandClass::Read => None,
    }
}

/// One simple command: its words (quotes removed) and whether it truncates or
/// overwrites a file through an output redirect (`>`, `>|`, `&>`).
#[derive(Debug, Default)]
struct Segment {
    words: Vec<String>,
    truncates: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Redirect {
    Truncate,
    Append,
    Input,
}

#[derive(Default)]
struct Lexer {
    segments: Vec<Segment>,
    seg: Segment,
    word: String,
    in_word: bool,
    redirect: Option<Redirect>,
}

impl Lexer {
    fn end_word(&mut self) {
        if !self.in_word {
            return;
        }
        self.in_word = false;
        let word = std::mem::take(&mut self.word);
        match self.redirect.take() {
            Some(Redirect::Truncate) => {
                if !is_harmless_sink(&word) {
                    self.seg.truncates = true;
                }
            }
            Some(Redirect::Append | Redirect::Input) => {}
            None => self.seg.words.push(word),
        }
    }

    /// Ends the current simple command. `false` when a redirect has no target.
    fn end_segment(&mut self) -> bool {
        self.end_word();
        if self.redirect.is_some() {
            return false;
        }
        let seg = std::mem::take(&mut self.seg);
        if !seg.words.is_empty() || seg.truncates {
            self.segments.push(seg);
        }
        true
    }
}

/// Output sinks a truncating redirect cannot damage.
fn is_harmless_sink(target: &str) -> bool {
    target == "/dev/null" || target.starts_with("/dev/std") || target.starts_with("/dev/fd/")
}

/// Splits `command` into simple commands with shell quoting rules. `None` when
/// the command cannot be read unambiguously.
fn lex(command: &str) -> Option<Vec<Segment>> {
    let chars: Vec<char> = command.chars().collect();
    let mut lx = Lexer::default();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                let next = *chars.get(i + 1)?;
                if next != '\n' {
                    lx.word.push(next);
                    lx.in_word = true;
                }
                i += 2;
                continue;
            }
            '\'' => {
                let close = chars[i + 1..].iter().position(|&ch| ch == '\'')? + i + 1;
                lx.word.extend(&chars[i + 1..close]);
                lx.in_word = true;
                i = close + 1;
                continue;
            }
            '"' => {
                let mut j = i + 1;
                loop {
                    match *chars.get(j)? {
                        '"' => break,
                        '\\' => {
                            lx.word.push(*chars.get(j + 1)?);
                            j += 2;
                        }
                        ch => {
                            lx.word.push(ch);
                            j += 1;
                        }
                    }
                }
                lx.in_word = true;
                i = j + 1;
                continue;
            }
            ' ' | '\t' | '\r' => lx.end_word(),
            ';' | '\n' | '|' | '(' | ')' => {
                if !lx.end_segment() {
                    return None;
                }
            }
            '&' if chars.get(i + 1) == Some(&'>') => {
                // `&>file` / `&>>file`: both streams into a file.
                lx.end_word();
                i += 1;
                continue;
            }
            '&' => {
                if !lx.end_segment() {
                    return None;
                }
            }
            '>' => {
                // `2>file`: a numeric word right before `>` is the fd, not an arg.
                if lx.in_word && lx.word.chars().all(|ch| ch.is_ascii_digit()) {
                    lx.word.clear();
                    lx.in_word = false;
                } else {
                    lx.end_word();
                }
                if lx.redirect.is_some() {
                    return None;
                }
                match chars.get(i + 1) {
                    Some('>') => {
                        lx.redirect = Some(Redirect::Append);
                        i += 1;
                    }
                    Some('|') => {
                        lx.redirect = Some(Redirect::Truncate);
                        i += 1;
                    }
                    Some('&') => {
                        // `>&2` / `>&-` duplicates or closes an fd; `>&file`
                        // writes both streams into a file.
                        let mut j = i + 2;
                        while chars
                            .get(j)
                            .is_some_and(|ch| ch.is_ascii_digit() || *ch == '-')
                        {
                            j += 1;
                        }
                        if j == i + 2 {
                            lx.redirect = Some(Redirect::Truncate);
                            i += 2;
                        } else {
                            i = j;
                        }
                        continue;
                    }
                    _ => lx.redirect = Some(Redirect::Truncate),
                }
            }
            '<' => {
                lx.end_word();
                if lx.redirect.is_some() {
                    return None;
                }
                while chars.get(i + 1) == Some(&'<') {
                    i += 1;
                }
                lx.redirect = Some(Redirect::Input);
            }
            _ => {
                lx.word.push(c);
                lx.in_word = true;
            }
        }
        i += 1;
    }
    if !lx.end_segment() {
        return None;
    }
    Some(lx.segments)
}

/// Shell grammar words that precede the real command of a simple command.
const CONTROL_WORDS: &[&str] = &[
    "{", "}", "!", "if", "then", "else", "elif", "fi", "do", "done", "while", "until", "time",
];

/// Interpreters, command executors and wrappers: each runs a command or code
/// the gate cannot see, so each parks as `privileged_command`.
const EXECUTORS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "fish",
    "csh",
    "tcsh",
    "ash",
    "perl",
    "ruby",
    "node",
    "nodejs",
    "deno",
    "bun",
    "php",
    "lua",
    "tclsh",
    "awk",
    "gawk",
    "mawk",
    "nawk",
    "osascript",
    "open",
    "xdg-open",
    "eval",
    "exec",
    "source",
    ".",
    "sudo",
    "doas",
    "su",
    "pkexec",
    "nice",
    "nohup",
    "timeout",
    "gtimeout",
    "command",
    "builtin",
    "xargs",
    "parallel",
    "watch",
    "script",
    "stdbuf",
    "unbuffer",
    "caffeinate",
    "chroot",
    "ionice",
    "taskset",
    "flock",
    "setsid",
    "iex",
    "pwsh",
    "powershell",
    "cmd",
    "kill",
    "killall",
    "pkill",
    "chmod",
    "chown",
    "chgrp",
];

const SYSTEM_SETTINGS_COMMANDS: &[&str] = &[
    "defaults",
    "launchctl",
    "systemctl",
    "crontab",
    "scutil",
    "networksetup",
    "pmset",
    "csrutil",
    "spctl",
    "tccutil",
    "security",
];

fn is_python(base: &str) -> bool {
    base.starts_with("python") || base.starts_with("pypy")
}

fn classify_segment(seg: &Segment) -> Option<ActionCategory> {
    let mut words = seg
        .words
        .iter()
        .map(String::as_str)
        .skip_while(|w| CONTROL_WORDS.contains(w) || (w.contains('=') && !w.starts_with('-')));
    let Some(raw_base) = words.next() else {
        return seg.truncates.then_some(ActionCategory::Irreversible);
    };
    // A command named by a variable (`$CMD x`) is hidden from classification.
    if raw_base.starts_with('$') {
        return Some(ActionCategory::PrivilegedCommand);
    }
    let base = raw_base
        .rsplit('/')
        .next()
        .unwrap_or(raw_base)
        .to_ascii_lowercase();
    let rest: Vec<String> = words.map(str::to_ascii_lowercase).collect();
    command_class(&base, &rest).or(seg.truncates.then_some(ActionCategory::Irreversible))
}

fn command_class(base: &str, rest: &[String]) -> Option<ActionCategory> {
    use ActionCategory as C;
    let has = |w: &str| rest.iter().any(|a| a == w);
    let has_prefix = |p: &str| rest.iter().any(|a| a.starts_with(p));
    // A short-flag cluster carrying `flag` (`-sf` carries `f`).
    let short_flag = |flag: char| {
        rest.iter()
            .any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains(flag))
    };
    match base {
        "command" if matches!(rest.first().map(String::as_str), Some("-v")) => None,
        // `env` alone prints the environment; `env X=1 cmd` runs `cmd`.
        "env" => rest
            .iter()
            .any(|a| !a.starts_with('-') && !a.contains('='))
            .then_some(C::PrivilegedCommand),
        b if EXECUTORS.contains(&b) || is_python(b) => Some(C::PrivilegedCommand),
        "rm" | "rmdir" | "unlink" | "trash" | "srm" | "shred" => Some(C::Delete),
        "find" if has("-delete") => Some(C::Delete),
        "find" if has("-exec") || has("-execdir") || has("-ok") || has("-okdir") => {
            Some(C::PrivilegedCommand)
        }
        "mv" if has("/dev/null") => Some(C::Delete),
        // A move or copy may overwrite its target; the gate cannot see whether
        // it exists, so only a no-clobber form is ordinary.
        "mv" | "cp" if !(has("--no-clobber") || short_flag('n')) => Some(C::Irreversible),
        "dd" | "truncate" => Some(C::Irreversible),
        "ln" if short_flag('f') || has("--force") => Some(C::Irreversible),
        "tee"
            if !(has("-a") || has("--append"))
                && rest
                    .iter()
                    .any(|a| !a.starts_with('-') && !is_harmless_sink(a)) =>
        {
            Some(C::Irreversible)
        }
        "rsync" if has_prefix("--delete") || has("--remove-source-files") => Some(C::Delete),
        "git" => git_class(rest),
        "mail" | "sendmail" | "mailx" | "msmtp" => Some(C::SendMessage),
        b if SYSTEM_SETTINGS_COMMANDS.contains(&b) => Some(C::SystemSettings),
        _ => None,
    }
}

/// `git` verbs that publish, delete or rewrite history beyond easy recovery.
fn git_class(rest: &[String]) -> Option<ActionCategory> {
    use ActionCategory as C;
    // Skip global options (`-C <dir>`, `-c k=v`, `--git-dir=…`, `--no-pager`).
    let mut i = 0;
    while let Some(arg) = rest.get(i) {
        // `rest` is lowercased, so `-c` also covers `-C <dir>`.
        if arg == "-c" || arg == "--git-dir" || arg == "--work-tree" {
            i += 2;
        } else if arg.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    let verb = rest.get(i)?.as_str();
    let args = &rest[i + 1..];
    let has = |w: &str| args.iter().any(|a| a == w);
    let sub = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(String::as_str);
    match verb {
        "push"
            if has("--force")
                || has("-f")
                || args
                    .iter()
                    .any(|a| a.starts_with("--force") || a.starts_with('+')) =>
        {
            Some(C::Irreversible)
        }
        "push" if has("--delete") || has("-d") => Some(C::Delete),
        "push" => Some(C::Publish),
        "reset" | "clean" | "checkout" | "restore" | "rebase" | "filter-branch" | "filter-repo" => {
            Some(C::Irreversible)
        }
        "switch" if has("-f") || has("--force") || has("--discard-changes") => {
            Some(C::Irreversible)
        }
        "rm" => Some(C::Delete),
        // Lowercased: `-d` also covers `-D`.
        "branch" | "tag" if has("-d") || has("--delete") => Some(C::Delete),
        "branch" if has("-f") || has("--force") => Some(C::Irreversible),
        "stash" if matches!(sub, Some("drop" | "clear")) => Some(C::Delete),
        "reflog" if matches!(sub, Some("expire" | "delete")) => Some(C::Delete),
        "worktree" if matches!(sub, Some("remove" | "prune")) => Some(C::Delete),
        "update-ref" if has("-d") => Some(C::Delete),
        "gc" if args.iter().any(|a| a.starts_with("--prune")) => Some(C::Irreversible),
        _ => None,
    }
}
