use crate::neppy::security::policy::AutonomyLevel;

use super::*;

fn policy(level: AutonomyLevel) -> SecurityPolicy {
    SecurityPolicy {
        autonomy: level,
        ..SecurityPolicy::default()
    }
}

#[test]
fn the_full_tier_runs_ordinary_commands() {
    let full = policy(AutonomyLevel::Full);
    for command in ["cargo test", "pnpm test", "echo run >> log.txt; git status"] {
        assert_eq!(check_command(&full, command), Ok(()), "{command}");
    }
}

#[test]
fn a_destructive_command_is_refused_in_every_tier() {
    for level in [AutonomyLevel::Full, AutonomyLevel::Supervised] {
        let err = check_command(&policy(level), "rm -rf /").unwrap_err();
        assert!(!err.is_empty(), "{level:?}");
    }
    let err = check_command(&policy(AutonomyLevel::Full), "rm -rf /").unwrap_err();
    assert!(err.contains("destructive"), "{err}");
}

#[test]
fn supervised_refuses_a_command_the_gate_would_ask_about() {
    let supervised = policy(AutonomyLevel::Supervised);
    // `touch` is on the allow list but is not a read, so the harness gate
    // would stop to ask. A task cannot ask, and must not assume the answer.
    let class = supervised.classify_command("touch scratch.txt");
    assert_eq!(supervised.gate_decision(class), GateDecision::Prompt);
    let err = check_command(&supervised, "touch scratch.txt").unwrap_err();
    assert!(err.contains("needs approval"), "{err}");
    assert!(err.contains("Supervised"), "{err}");
    // A read goes through.
    assert_eq!(check_command(&supervised, "git status"), Ok(()));
    assert_eq!(check_command(&supervised, "ls -la"), Ok(()));
}

#[test]
fn the_full_tier_runs_a_prompt_class_command_the_user_wrote_into_the_task() {
    let full = policy(AutonomyLevel::Full);
    let fetch = full.classify_command("curl https://example.com");
    assert_eq!(full.gate_decision(fetch), GateDecision::Prompt);
    assert_eq!(check_command(&full, "curl https://example.com"), Ok(()));
    // The same command in Supervised is refused, as are the allow-list misses.
    assert!(check_command(
        &policy(AutonomyLevel::Supervised),
        "curl https://example.com"
    )
    .is_err());
}

#[test]
fn a_command_that_is_not_on_the_allow_list_is_refused_in_supervised() {
    let supervised = policy(AutonomyLevel::Supervised);
    let err = check_command(&supervised, "python3 -c 'print(1)'").unwrap_err();
    assert!(err.contains("allow list"), "{err}");
    let err = check_command(&supervised, "git status; echo $(whoami)").unwrap_err();
    assert!(err.contains("allow list"), "{err}");
}

#[test]
fn the_read_only_tier_runs_nothing_not_even_a_read() {
    // Exactly what a cron shell job gets: `can_act` comes first.
    let read_only = policy(AutonomyLevel::ReadOnly);
    for command in ["cargo test", "git status", "ls"] {
        let err = check_command(&read_only, command).unwrap_err();
        assert!(err.contains("read-only"), "{command}: {err}");
    }
}

#[test]
fn a_forbidden_path_argument_is_refused_whatever_the_tier() {
    let full = policy(AutonomyLevel::Full);
    for command in [
        "cat /etc/hosts",
        "cat ~/.ssh/id_ed25519",
        "echo ok; cat ../../etc/passwd",
        "ls ./a/../../secret",
    ] {
        let err = check_command(&full, command).unwrap_err();
        assert!(err.contains("forbidden path argument"), "{command}: {err}");
    }
    assert_eq!(
        check_command(&full, "cargo test --manifest-path app/Cargo.toml"),
        Ok(())
    );
}

#[test]
fn the_path_argument_scan_matches_the_cron_scheduler() {
    // The cron scheduler's own cases for its private copy.
    let policy = SecurityPolicy::default();
    assert!(forbidden_path_argument(&policy, "echo hello").is_none());
    assert!(forbidden_path_argument(&policy, "date").is_none());
    assert!(forbidden_path_argument(&policy, "curl https://example.com").is_none());
    assert!(forbidden_path_argument(&policy, "ls -la").is_none());
    // And the shapes that matter here: a variable assignment is not the
    // executable, a quoted path is unquoted, every segment is scanned.
    assert_eq!(
        forbidden_path_argument(&policy, "FOO=1 cat \"/etc/hosts\"").as_deref(),
        Some("/etc/hosts")
    );
    assert_eq!(
        forbidden_path_argument(&policy, "true && cat /etc/hosts").as_deref(),
        Some("/etc/hosts")
    );
    assert!(forbidden_path_argument(&policy, "true; false | cat -n").is_none());
}

#[test]
fn authorizing_a_run_charges_the_action_budget_and_stops_at_the_limit() {
    let limited = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        max_actions_per_hour: 2,
        ..SecurityPolicy::default()
    };
    // The budget-free check can be repeated without cost.
    for _ in 0..5 {
        assert_eq!(check_command(&limited, "cargo test"), Ok(()));
    }
    assert_eq!(authorize_run(&limited, "cargo test"), Ok(()));
    assert_eq!(authorize_run(&limited, "cargo test"), Ok(()));
    let err = authorize_run(&limited, "cargo test").unwrap_err();
    assert!(err.contains("rate limit"), "{err}");
}

#[test]
fn authorizing_a_run_applies_every_check_again() {
    // The policy at run time, not at accept time, decides.
    let was_fine = policy(AutonomyLevel::Full);
    assert_eq!(authorize_run(&was_fine, "touch x"), Ok(()));
    let tightened = policy(AutonomyLevel::Supervised);
    assert!(authorize_run(&tightened, "touch x").is_err());
    assert!(authorize_run(&policy(AutonomyLevel::ReadOnly), "git status").is_err());
}
