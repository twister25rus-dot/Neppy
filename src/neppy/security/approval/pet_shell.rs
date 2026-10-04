//! Pet companion (user decision D4): the label of a parked `shell` call.
//!
//! Under the Pet companion origin **every** `shell` call parks for
//! confirmation, on every platform: shell syntax (comments, `$'..'` / `${..}`
//! expansion of flags, clustered flags, fd sinks, `printf -v`, Windows
//! `cmd /C` quoting, …) is too subtle to prove any command read-only. This
//! module only picks the most useful category name for the approval card:
//! a recognisable command keeps a precise label (`rm` → `delete`, `git push`
//! → `publish`, `> file` → `irreversible`, `curl` → `share_personal_info`,
//! a package install → `install`), and everything else is
//! `privileged_command`.
//!
//! It reads the RAW command (never the redacted audit copy).

use std::sync::OnceLock;

use crate::neppy::pet::companion::types::ActionCategory;
use crate::neppy::security::{CommandClass, SecurityPolicy};

/// The category a Pet companion `shell` call parks under. Always a category —
/// no shell command is ordinary under the Pet companion origin.
pub(super) fn pet_companion_shell_label(args: &serde_json::Value) -> ActionCategory {
    let command = args
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    if command.is_empty() {
        return ActionCategory::PrivilegedCommand;
    }
    // Bash's network pseudo-devices open a socket from a plain redirect.
    if command.contains("/dev/tcp") || command.contains("/dev/udp") {
        return ActionCategory::SharePersonalInfo;
    }
    // Hidden execution runs an inner command the label cannot describe.
    if ["`", "$(", "<(", ">("].iter().any(|p| command.contains(p)) {
        return ActionCategory::PrivilegedCommand;
    }
    let Some(segments) = lex(command) else {
        return ActionCategory::PrivilegedCommand;
    };
    if let Some(category) = segments.iter().find_map(segment_label) {
        return category;
    }
    static POLICY: OnceLock<SecurityPolicy> = OnceLock::new();
    match POLICY
        .get_or_init(SecurityPolicy::default)
        .classify_command(command)
    {
        CommandClass::Install => ActionCategory::Install,
        CommandClass::Network => ActionCategory::SharePersonalInfo,
        _ => ActionCategory::PrivilegedCommand,
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

/// The precise label of one simple command, when it is recognisable.
fn segment_label(seg: &Segment) -> Option<ActionCategory> {
    let mut words = seg
        .words
        .iter()
        .map(String::as_str)
        .skip_while(|w| CONTROL_WORDS.contains(w) || (w.contains('=') && !w.starts_with('-')));
    let label = words.next().and_then(|raw_base| {
        let base = raw_base
            .rsplit('/')
            .next()
            .unwrap_or(raw_base)
            .to_ascii_lowercase();
        let rest: Vec<String> = words.map(str::to_ascii_lowercase).collect();
        command_class(&base, &rest)
    });
    label.or(seg.truncates.then_some(ActionCategory::Irreversible))
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
        "rm" | "rmdir" | "unlink" | "trash" | "srm" | "shred" => Some(C::Delete),
        "find" if has("-delete") => Some(C::Delete),
        "find" if has("-exec") || has("-execdir") || has("-ok") || has("-okdir") => {
            Some(C::PrivilegedCommand)
        }
        "mv" if has("/dev/null") => Some(C::Delete),
        // A move or copy may overwrite its target.
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
