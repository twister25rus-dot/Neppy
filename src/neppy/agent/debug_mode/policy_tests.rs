use super::*;

fn cfg() -> DebugModeConfig {
    DebugModeConfig::default()
}

fn check(cmd: &str) -> DebugCommandDecision {
    check_debug_command(cmd, &cfg())
}

fn kind(d: &DebugCommandDecision) -> &'static str {
    match d {
        Allow => "allow",
        Ask(_) => "ask",
        Deny(_) => "deny",
    }
}

#[track_caller]
fn expect(cmd: &str, want: &str) {
    let got = check(cmd);
    assert_eq!(kind(&got), want, "`{cmd}` -> {got:?}");
}

#[test]
fn safe_commands_are_allowed() {
    for c in [
        "ls -la",
        "pwd",
        "cat src/main.rs",
        "rg foo src",
        "git status",
        "git diff HEAD",
        "git log --oneline -5",
        "git add -A",
        "git commit -m 'fix'",
        "git stash",
        "git stash pop",
        "git checkout -b feature/x",
        "git branch -d merged",
        "git restore --staged src/a.rs",
        "git clean -n",
        "npm test",
        "npm run lint",
        "cargo check",
        "cargo test --lib",
        "cargo build --release",
        "rm file.txt",
        "rm -f file.txt",
        "pnpm run build && pnpm test",
        "echo 'sudo rm -rf /' | cat",
        "FOO=1 cargo test",
    ] {
        expect(c, "allow");
    }
}

#[test]
fn restoring_a_lockfile_is_not_a_dependency_install() {
    for c in [
        "pnpm install",
        "pnpm i",
        "npm install",
        "npm ci",
        "yarn",
        "yarn install",
        "bun install",
        "pnpm install --frozen-lockfile",
        "pnpm --filter app install",
        "pip install -r requirements.txt",
        "uv sync",
        "cargo fetch",
    ] {
        expect(c, "allow");
    }
}

#[test]
fn dependency_installs_are_denied_by_default() {
    for c in [
        "npm install left-pad",
        "npm i left-pad",
        "npm install -D typescript",
        "pnpm add zustand",
        "pnpm --filter app add zustand",
        "pnpm install zustand",
        "yarn add react",
        "bun add react",
        "bun install react",
        "cargo add serde",
        "cargo +nightly add serde",
        "cargo install ripgrep",
        "pip install requests",
        "pip3 install requests",
        "python3 -m pip install requests",
        "uv pip install requests",
        "uv add requests",
        "poetry add requests",
        "brew install jq",
        "go get github.com/x/y",
        "cd app && npm install zod",
    ] {
        expect(c, "deny");
    }
    assert!(matches!(check("cargo add serde"), Deny(r) if r.contains("allow_dependency_install")));
}

#[test]
fn dependency_installs_pass_when_allowed() {
    let c = DebugModeConfig {
        allow_dependency_install: true,
        ..cfg()
    };
    for cmd in [
        "npm install left-pad",
        "cargo add serde",
        "pnpm add x",
        "uv add x",
    ] {
        assert_eq!(check_debug_command(cmd, &c), Allow, "{cmd}");
    }
}

#[test]
fn git_push_is_denied_in_every_form() {
    for c in [
        "git push",
        "git push origin main",
        "git -C /tmp/x push",
        "git -c user.name=a push origin HEAD",
        "git --no-pager push",
        "cd repo && git push -u origin feat",
        "git push --force",
        "git push -f origin main",
        "git push --force-with-lease",
        "git push origin +main",
        "FOO=1 git push",
        "env GIT_SSH=x git push",
        "sh -c 'git push'",
        "echo ok; git push",
    ] {
        expect(c, "deny");
    }
}

#[test]
fn push_when_allowed_asks_only_for_force() {
    let c = DebugModeConfig {
        allow_git_push: true,
        ..cfg()
    };
    assert_eq!(check_debug_command("git push origin feat", &c), Allow);
    assert_eq!(kind(&check_debug_command("git push --force", &c)), "ask");
    assert_eq!(kind(&check_debug_command("git push -f", &c)), "ask");
    let lax = DebugModeConfig {
        allow_git_push: true,
        dangerous_commands_require_confirmation: false,
        ..cfg()
    };
    assert_eq!(check_debug_command("git push --force", &lax), Allow);
}

#[test]
fn system_commands_are_denied_by_default() {
    for c in [
        "sudo ls",
        "sudo -n rm x",
        "/usr/bin/sudo ls",
        "su root",
        "launchctl unload x",
        "systemctl stop y",
        "diskutil eraseDisk x",
        "defaults write com.apple.dock autohide -bool true",
        "chmod -R 777 /usr/local",
        "chmod -R 755 ~/x",
        "chown root file",
        "mkfs.ext4 /dev/sda1",
        "mkfs /dev/x",
        "dd if=/dev/zero of=/dev/disk2",
        "timeout 5 sudo ls",
        "xargs sudo rm",
    ] {
        expect(c, "deny");
    }
    // Not system commands.
    expect("chmod +x script.sh", "allow");
    expect("chmod -R 755 build", "allow");
    expect("defaults read com.apple.dock", "allow");
    let c = DebugModeConfig {
        allow_system_commands: true,
        ..cfg()
    };
    assert_eq!(check_debug_command("sudo ls", &c), Allow);
}

#[test]
fn dangerous_commands_ask_when_confirmation_is_required() {
    for c in [
        "rm -rf build",
        "rm -fr build",
        "rm -r build",
        "rm -Rf build",
        "rm --recursive --force build",
        "git reset --hard",
        "git reset --hard HEAD~1",
        "git clean -fd",
        "git clean -f",
        "git clean -xdf",
        "git checkout -- .",
        "git checkout .",
        "git restore .",
        "git stash drop",
        "git stash clear",
        "git branch -D old",
        "xargs rm -rf",
        "find . -name x | xargs rm -rf",
    ] {
        expect(c, "ask");
    }
    let c = DebugModeConfig {
        dangerous_commands_require_confirmation: false,
        ..cfg()
    };
    for cmd in [
        "rm -rf build",
        "git reset --hard",
        "git clean -fd",
        "git branch -D x",
    ] {
        assert_eq!(check_debug_command(cmd, &c), Allow, "{cmd}");
    }
}

#[test]
fn compound_commands_check_every_segment_and_the_worst_wins() {
    expect("ls && git push", "deny");
    expect("ls; rm -rf x", "ask");
    expect("rm -rf x && sudo ls", "deny");
    expect("true || npm install x", "deny");
    expect("(cd x && git push)", "deny");
    expect("echo $(sudo ls)", "deny");
    expect("echo `git push`", "deny");
    expect("bash -c 'cargo add serde'", "deny");
    expect("bash -lc \"git reset --hard\"", "ask");
    expect("eval git push", "deny");
    expect("sh -c \"sh -c 'sh -c \\\"sh -c true\\\"'\"", "ask");
    expect("ls\ngit push", "deny");
}

#[test]
fn unparseable_or_opaque_commands_ask() {
    expect("echo 'unterminated", "ask");
    expect("echo \"unterminated", "ask");
    expect("echo \"$(date)\"", "ask");
    expect("echo \"`date`\"", "ask");
}

#[test]
fn quoted_text_is_not_executed() {
    expect("echo 'git push'", "allow");
    expect("git commit -m \"npm install x; sudo rm\"", "allow");
    expect("rg 'rm -rf' src", "allow");
}

#[test]
fn commit_is_denied_only_when_disabled() {
    let c = DebugModeConfig {
        allow_git_commit: false,
        ..cfg()
    };
    assert!(matches!(
        check_debug_command("git commit -m x", &c),
        Deny(_)
    ));
    assert!(matches!(
        check_debug_command("git -C . commit -m x", &c),
        Deny(_)
    ));
    assert_eq!(check_debug_command("git status", &c), Allow);
}

#[test]
fn ensure_enabled_and_commit_helpers() {
    assert!(ensure_enabled(&cfg()).is_ok());
    let off = DebugModeConfig {
        enabled: false,
        ..cfg()
    };
    assert!(ensure_enabled(&off).unwrap_err().contains("turned off"));
    assert!(ensure_commit_allowed(&cfg()).is_ok());
    let nc = DebugModeConfig {
        allow_git_commit: false,
        ..cfg()
    };
    assert!(ensure_commit_allowed(&nc)
        .unwrap_err()
        .contains("allow_git_commit"));
}

#[test]
fn prompt_addendum_reflects_the_settings() {
    let a = prompt_addendum(&cfg());
    assert!(a.contains("5 at most"));
    assert!(a.contains("run tests: yes") && a.contains("run the build: yes"));
    assert!(a.contains("Installing dependencies: not allowed"));
    assert!(a.contains("Git push: not allowed"));
    let c = DebugModeConfig {
        max_repair_iterations: 99,
        auto_repair: false,
        run_tests_after_changes: false,
        allow_dependency_install: true,
        allow_git_push: true,
        ..cfg()
    };
    let a = prompt_addendum(&c);
    assert!(a.contains("20 at most") && a.contains("auto-repair is off"));
    assert!(a.contains("run tests: no"));
    assert!(a.contains("Installing dependencies: allowed") && a.contains("Git push: allowed"));
}

// ── publishing a release is never the agent's to do ────────────────────

/// Every setting loosened as far as it goes: the release rule must not bend.
fn permissive() -> DebugModeConfig {
    DebugModeConfig {
        allow_git_push: true,
        allow_git_commit: true,
        allow_dependency_install: true,
        allow_system_commands: true,
        dangerous_commands_require_confirmation: false,
        ..cfg()
    }
}

#[test]
fn executing_the_release_script_or_publishing_is_denied_however_it_is_invoked() {
    for c in [
        "bash scripts/release-neppy.sh 0.69.0",
        "sh scripts/release-neppy.sh 0.69.0",
        "zsh scripts/release-neppy.sh 0.69.0 --dry-run",
        "bash -x scripts/release-neppy.sh 1.0.0",
        "source scripts/release-neppy.sh 1.0.0",
        ". scripts/release-neppy.sh 1.0.0",
        "./scripts/release-neppy.sh 0.69.0",
        "scripts/release-neppy.sh 0.69.0",
        "/Users/alex/Neppy/scripts/release-neppy.sh 0.69.0",
        "bash /Users/alex/Neppy/scripts/release-neppy.sh 0.69.0",
        "cd scripts && bash release-neppy.sh 0.69.0",
        "env FOO=1 bash scripts/release-neppy.sh 0.69.0",
        "nohup bash scripts/release-neppy.sh 0.69.0",
        "time bash scripts/release-neppy.sh 0.69.0",
        "timeout 600 bash scripts/release-neppy.sh 0.69.0",
        "setsid nohup bash scripts/release-neppy.sh 0.69.0 &",
        "echo 0.69.0 | xargs bash scripts/release-neppy.sh",
        "sudo bash scripts/release-neppy.sh 0.69.0",
        "ls && bash scripts/release-neppy.sh 0.69.0",
        r"find . -exec bash scripts/release-neppy.sh 1.0.0 \;",
        "bash -c 'bash scripts/release-neppy.sh 0.69.0'",
        "sh -c \"./scripts/release-neppy.sh 0.69.0\"",
        "bash -lc 'cd /x && scripts/release-neppy.sh 0.69.0'",
        "bash -c \"bash -c 'sh -c \\\"bash -c \\\\\\\"bash scripts/release-neppy.sh 1\\\\\\\"\\\"'\"",
        "eval bash scripts/release-neppy.sh 0.69.0",
        "pnpm exec bash scripts/release-neppy.sh 0.69.0",
        "npx bash scripts/release-neppy.sh 0.69.0",
        "gh release create v0.69.0 --notes x",
        "gh release upload v0.69.0 a.tar.gz",
        "gh release delete v0.69.0 --yes",
        "gh release edit v0.69.0 --draft=false",
        "gh -R acme/widgets release create v1 a.tgz",
        "gh --repo acme/widgets release delete v1",
        "gh release",
        "gh api repos/acme/widgets/releases -X POST -f tag_name=v1",
        "gh api repos/acme/widgets/releases/1 --method DELETE",
        "gh api repos/acme/widgets/releases -f tag_name=v1",
        "pnpm release",
        "pnpm run release",
        "pnpm run release:patch",
        "pnpm --filter neppy-app run release:x",
        "pnpm -C app release",
        "npm run release",
        "npm run-script release:minor",
        "yarn release",
        "bun run release",
    ] {
        for cfg in [cfg(), permissive()] {
            let got = check_debug_command(c, &cfg);
            assert_eq!(
                got,
                Deny(policy_release::DENY_MESSAGE.to_string()),
                "`{c}` -> {got:?}"
            );
        }
    }
}

#[test]
fn reading_or_searching_the_release_machinery_stays_allowed() {
    for c in [
        "cat scripts/release-neppy.sh",
        "grep REPO scripts/release-neppy.sh",
        "grep x scripts/release-neppy.sh",
        "rg release-neppy",
        "rg -n 'release-neppy.sh' scripts src",
        "head -40 scripts/release-neppy.sh",
        "bash -n scripts/neppy-recover.sh",
        "bash -c 'cat scripts/release-neppy.sh'",
        "git log --oneline -- scripts/release-neppy.sh",
        "git diff HEAD -- scripts/release-neppy.sh",
        "ls scripts | grep release",
        "shellcheck scripts/release-neppy.sh",
        "gh release list",
        "gh release view v0.68.5",
        "gh release list -L 5",
        "gh -R acme/widgets release view v1 --json tagName",
        "gh pr view 12",
        "gh pr list --search release",
        "gh api repos/acme/widgets/releases",
        "gh api repos/acme/widgets/releases -X GET",
        "cargo build --release",
        "pnpm run build",
        "pnpm test",
        "npm run lint",
        "pnpm -C app typecheck",
    ] {
        for cfg in [cfg(), permissive()] {
            let got = check_debug_command(c, &cfg);
            assert!(
                !matches!(got, Deny(_)),
                "`{c}` must not be denied -> {got:?}"
            );
        }
    }
}

#[test]
fn the_release_deny_is_not_an_ask_and_needs_no_setting() {
    // Denied even though `dangerous_commands_require_confirmation` would
    // otherwise turn things into a question, and with push allowed.
    let d = check_debug_command("bash scripts/release-neppy.sh 0.69.0", &permissive());
    assert_eq!(kind(&d), "deny");
    assert!(matches!(&d, Deny(m) if m.contains("Publish release card")));
}

#[test]
fn package_scripts_that_wrap_a_release_are_found_and_denied() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("app")).unwrap();
    std::fs::write(
        root.path().join("package.json"),
        r#"{"scripts":{"ship":"bash scripts/release-neppy.sh $1","build:release":"cargo build --release","test":"vitest","cut":"pnpm release:patch","gh-publish":"gh release create v1"}}"#,
    )
    .unwrap();
    std::fs::write(
        root.path().join("app/package.json"),
        r#"{"scripts":{"deploy":"pnpm run release","dev":"vite"}}"#,
    )
    .unwrap();
    let mut names = policy_release::wrapping_scripts(root.path());
    names.sort();
    assert_eq!(names, ["cut", "deploy", "gh-publish", "ship"]);

    for c in [
        "pnpm ship",
        "npm run cut",
        "pnpm -C app run deploy",
        "yarn gh-publish",
    ] {
        assert_eq!(
            kind(&check_debug_command_with(c, &cfg(), &names)),
            "deny",
            "{c}"
        );
        assert_eq!(
            kind(&check_debug_command(c, &cfg())),
            "allow",
            "{c}: unknown without the scan"
        );
    }
    for c in ["pnpm test", "pnpm run build:release", "npm run dev"] {
        assert_eq!(
            kind(&check_debug_command_with(c, &cfg(), &names)),
            "allow",
            "{c}"
        );
    }
    // No package.json at all is simply "no wrapping scripts".
    assert!(policy_release::wrapping_scripts(tempfile::tempdir().unwrap().path()).is_empty());
}
