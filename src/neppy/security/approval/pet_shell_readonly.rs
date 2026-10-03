//! Pet companion (user decision D4): the shell commands that are ordinary.
//!
//! A deliberately small allowlist of commands that only read: list, print,
//! search and inspect. Commands whose flag surface is too large to vet (`sort`,
//! `uniq`, `rg`, `less`, `more`) are not on it. Each entry refuses the flags
//! that make it write a file or run another program (`find -exec`,
//! `tree -o`, `date -s`, …); `git` is limited to `status`, `log`, and the
//! external-program-free forms of `diff` / `show`. Anything not matched here
//! parks (see `super::pet_shell`). Redirects, assignments and paths in command
//! position are rejected by the caller before this is consulted.

/// Commands that only read, whatever their arguments.
const ALWAYS_READ_ONLY: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "grep",
    "egrep",
    "fgrep",
    "wc",
    "pwd",
    "which",
    "stat",
    "du",
    "df",
    "echo",
    "printf",
    "cut",
    "jq",
    "basename",
    "dirname",
    "realpath",
    "whoami",
    "uname",
    "nl",
    "tac",
    "cd",
    "test",
    "true",
    "false",
    "sleep",
    "type",
    "diff",
    "cmp",
    "md5",
    "md5sum",
    "shasum",
    "sha256sum",
    "column",
];

/// `base` with lowercased `rest` (and the original-case `rest_raw`, for flags
/// whose case matters) is an ordinary read-only command.
pub(super) fn is_read_only(base: &str, rest: &[String], rest_raw: &[&str]) -> bool {
    let has = |w: &str| rest.iter().any(|a| a == w);
    let has_prefix = |p: &str| rest.iter().any(|a| a.starts_with(p));
    match base {
        b if ALWAYS_READ_ONLY.contains(&b) => true,
        // `file -C` compiles a magic file (writes).
        "file" => !(has("-c") || has("--compile")),
        // `date -s` sets the clock.
        "date" => !(has("-s") || has_prefix("--set")),
        // `tree -o` writes.
        "tree" => !has("-o"),
        "command" => matches!(rest.first().map(String::as_str), Some("-v")),
        "find" => ![
            "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf",
            "-fls",
        ]
        .iter()
        .any(|f| has(f)),
        // git honours repository-local config that can launch programs
        // (core.fsmonitor, diff/textconv drivers); it always parks.
        "git" => false,
        _ => false,
    }
}

/// Read-only `git` invocations, limited to forms that run no external program
/// configured by the repository: `git status`; `git log` (a patch-producing
/// `log` only with `--no-textconv --no-ext-diff`); `git diff --no-ext-diff
/// --no-textconv`; `git show --no-textconv`. Only `--no-pager` and `-C <dir>`
/// may precede the verb — `-c` (config incl. `alias.x=!cmd` / `core.pager`),
/// `--exec-path`, `--git-dir` and `--work-tree` never. `--output` is refused.
/// Every other git command parks.
fn git_read_only(rest: &[&str]) -> bool {
    let mut i = 0;
    while let Some(arg) = rest.get(i) {
        match *arg {
            "--no-pager" => i += 1,
            "-C" => i += 2,
            a if a.starts_with('-') => return false,
            _ => break,
        }
    }
    let Some(verb) = rest.get(i) else {
        return false;
    };
    let args = &rest[i + 1..];
    let has = |w: &str| args.contains(&w);
    if args.iter().any(|a| {
        a.starts_with("--output") || a.starts_with("--ext-diff") || a.starts_with("--textconv")
    }) {
        return false;
    }
    let no_external = has("--no-ext-diff") && has("--no-textconv");
    match *verb {
        "status" => true,
        "log" => {
            let patch = args.iter().any(|a| {
                matches!(
                    *a,
                    "-p" | "-u" | "--patch" | "--cc" | "-c" | "-m" | "--word-diff" | "--stat"
                ) || a.starts_with("-L")
                    || a.starts_with("--word-diff")
            });
            !patch || no_external
        }
        "diff" => no_external,
        "show" => has("--no-textconv"),
        _ => false,
    }
}
