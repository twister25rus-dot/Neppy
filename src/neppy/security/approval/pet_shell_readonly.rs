//! Pet companion (user decision D4): the shell commands that are ordinary.
//!
//! A deliberately small allowlist of commands that only read: list, print,
//! search and inspect. Each entry also refuses the flags that make it write a
//! file or run another program (`sort -o`, `rg --pre`, `find -exec`,
//! `git diff --output`, `git -c alias.x=!cmd`, …). Anything not matched here
//! parks (see `super::pet_shell`). Redirects, assignments and paths in command
//! position are rejected by the caller before this is consulted.

/// Commands that only read, whatever their arguments.
const ALWAYS_READ_ONLY: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "less",
    "more",
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
    let positional = || rest.iter().filter(|a| !a.starts_with('-')).count();
    match base {
        b if ALWAYS_READ_ONLY.contains(&b) => true,
        // `--pre` runs a program on every file searched.
        "rg" => !has_prefix("--pre"),
        // `file -C` compiles a magic file (writes).
        "file" => !(has("-c") || has("--compile")),
        // `date -s` sets the clock.
        "date" => !(has("-s") || has_prefix("--set")),
        // `sort -o` writes; `--compress-program` runs a program.
        "sort" => !(has("-o") || has_prefix("--output") || has_prefix("--compress-program")),
        // `uniq in out` writes `out`.
        "uniq" => positional() <= 1,
        // `tree -o` writes.
        "tree" => !has("-o"),
        "command" => matches!(rest.first().map(String::as_str), Some("-v")),
        "find" => ![
            "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprint0", "-fprintf",
            "-fls",
        ]
        .iter()
        .any(|f| has(f)),
        "git" => git_read_only(rest_raw),
        _ => false,
    }
}

/// Read-only `git` invocations: `status`, `log`, `diff`, `show`, `blame`,
/// `rev-parse`, `ls-files`, `describe`, `shortlog`, `branch` (listing only),
/// `remote` / `remote -v`, `stash list`. Only `--no-pager` and `-C <dir>` may
/// precede the verb — `-c` (config, incl. `alias.x=!cmd`), `--exec-path`,
/// `--git-dir`, `--work-tree` never. `--output` and `--ext-diff` are refused
/// (they write a file / run a program).
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
    if args
        .iter()
        .any(|a| a.starts_with("--output") || *a == "--ext-diff" || a.starts_with("--exec"))
    {
        return false;
    }
    let flags_only = args.iter().all(|a| a.starts_with('-'));
    match *verb {
        "status" | "log" | "diff" | "show" | "blame" | "rev-parse" | "ls-files" | "describe"
        | "shortlog" => true,
        "branch" => {
            flags_only
                && args.iter().all(|a| {
                    matches!(
                        *a,
                        "-a" | "-r"
                            | "-v"
                            | "-vv"
                            | "-l"
                            | "--list"
                            | "--all"
                            | "--remotes"
                            | "--verbose"
                            | "--show-current"
                            | "--no-color"
                    )
                })
        }
        "remote" => args.is_empty() || args == ["-v"],
        "stash" => args == ["list"],
        _ => false,
    }
}
