use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::turn::{with_turn, DebugTurn};
use crate::neppy::config::schema::debug_mode::DebugModeConfig;
use DebugCommandDecision::{Allow, Ask, Deny};

fn turn_with(settings: DebugModeConfig) -> DebugTurn {
    DebugTurn {
        ctx: DebugCtx::new(&std::env::temp_dir()),
        root: std::env::temp_dir(),
        task_id: None,
        checkpoint_id: None,
        settings,
    }
}

fn words(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

fn is_publish_deny(d: &DebugCommandDecision) -> bool {
    matches!(d, Deny(m) if m == policy_release::DENY_MESSAGE)
}

#[test]
fn npm_command_line_quotes_every_word() {
    assert_eq!(
        npm_command_line("run", &words(&["build", "it's"])),
        "npm 'run' 'build' 'it'\\''s'"
    );
    assert_eq!(npm_command_line("ci", &[]), "npm 'ci'");
}

#[tokio::test]
async fn npm_exec_goes_through_the_shell_gate_in_a_debug_turn() {
    // (subcommand, args, expected verdict kind, needle in the deny reason)
    let table: &[(&str, &[&str], &str, &str)] = &[
        ("install", &["left-pad"], "deny", "allow_dependency_install"),
        ("i", &["left-pad"], "deny", "allow_dependency_install"),
        ("add", &["left-pad"], "deny", "allow_dependency_install"),
        ("run", &["release"], "deny", "Publishing a release"),
        ("run", &["release:patch"], "deny", "Publishing a release"),
        (
            "exec",
            &["--", "bash", "scripts/release-neppy.sh", "1.0.0"],
            "deny",
            "Publishing a release",
        ),
        ("release", &[], "deny", "Publishing a release"),
        ("ci", &[], "allow", ""),
        ("install", &[], "allow", ""),
        ("run", &["build"], "allow", ""),
        ("run", &["test", "--", "--run"], "allow", ""),
        ("test", &[], "allow", ""),
    ];
    with_turn(turn_with(DebugModeConfig::default()), async {
        for (sub, args, want, needle) in table {
            let d = gate_npm_exec_current(sub, &words(args));
            match (*want, &d) {
                ("allow", Allow) => {}
                ("deny", Deny(m)) => assert!(m.contains(needle), "{sub} {args:?}: {m}"),
                _ => panic!("{sub} {args:?} -> {d:?}, wanted {want}"),
            }
        }
    })
    .await;
}

#[tokio::test]
async fn npm_exec_installs_are_allowed_when_the_setting_allows_them() {
    let cfg = DebugModeConfig {
        allow_dependency_install: true,
        ..DebugModeConfig::default()
    };
    with_turn(turn_with(cfg), async {
        assert_eq!(
            gate_npm_exec_current("install", &words(&["left-pad"])),
            Allow
        );
        // The release rule is not configurable.
        assert!(is_publish_deny(&gate_npm_exec_current(
            "run",
            &words(&["release"])
        )));
    })
    .await;
}

#[test]
fn both_gates_are_inert_outside_a_debug_turn() {
    assert_eq!(
        gate_npm_exec_current("install", &words(&["left-pad"])),
        Allow
    );
    assert_eq!(gate_npm_exec_current("run", &words(&["release"])), Allow);
    assert_eq!(
        gate_node_exec_current(
            "require('child_process').execSync('bash scripts/release-neppy.sh 1')"
        ),
        Allow
    );
}

#[test]
fn node_source_table() {
    let denied = [
        "require('child_process').execSync('bash scripts/release-neppy.sh 0.69.0')",
        "const {spawn}=require('node:child_process'); spawn('bash',['scripts/release-neppy.sh'])",
        "import {execSync} from 'child_process'; execSync('gh release create v1')",
        "require('child_process').exec('gh workflow run release-production.yml')",
        "require('child_process').spawnSync('gh',['release','create','v1'])",
        "require('child_process').spawnSync('gh',['workflow','run','promote-main-to-release.yml'])",
        "require('child_process').execFileSync('gh',['api','repos/a/b/actions/workflows/x.yml/dispatches','-f','ref=main'])",
        "const cp=require('child_process'); cp.execSync('GH release create v1')",
        "require('child_process').execSync('gh -R a/b workflow run release-production.yml')",
        "require('child_process').execSync('gh run rerun 123')",
        "require('child_process').execSync('cd x && gh release upload v1 a.tgz')",
        "require('child_process').spawnSync('gh',['api','-X','POST','repos/o/r/actions/runs/1/rerun'])",
        "require('child_process').execSync('pnpm release')",
        "const {fork}=require('child_process'); fork('scripts/release-neppy.sh')",
        // promisify / aliases / other object names: no call has to be recognised
        "const {promisify}=require('util'); const {exec}=require('child_process'); const execAsync=promisify(exec); await execAsync('gh release create v1')",
        "const {execSync: x}=require('child_process'); x('gh release create v1')",
        "const cp=require('child_process'); const run=cp.execSync.bind(cp); run('gh release create v1')",
        "const run=require('child_process').execSync; run('gh release create v1')",
        "const x=require('child_process'); x.exec('gh release create v1')",
        "const m=await import('node:child_process'); m.execSync('gh release create v1')",
        // nested shells inside a literal
        r#"require('child_process').execSync('sh -c "gh release create v1"')"#,
        r#"require('child_process').execSync("bash -c 'gh release create v1'")"#,
        r#"require('child_process').execSync("bash -lc 'cd x && gh release create v1'")"#,
        r#"require('child_process').execSync("sh -c \"bash -c 'bash scripts/release-neppy.sh 1'\"")"#,
        // argv through a variable
        "const {spawnSync}=require('child_process'); const args=['release','create','v1']; spawnSync('gh', args)",
        "const {spawnSync}=require('child_process'); const a=['workflow','run','release-production.yml']; spawnSync('/usr/bin/gh', a)",
        "const {spawnSync}=require('child_process'); const a=['run','rerun','1']; spawnSync('gh', a)",
        // graphql mutation
        r#"require('child_process').execSync("gh api graphql -f query='mutation { createRelease(input:{}) { id } }'")"#,
        "require('child_process').spawnSync('gh',['api','graphql','-f','query=mutation { deleteRelease(input:{}) { id } }'])",
        // execa / zx
        "const {execa}=require('execa'); await execa('gh',['release','create','v1'])",
        "import {$} from 'zx'; await $`gh release create v1`",
        "import {$} from 'zx'; await $`bash scripts/release-neppy.sh 1`",
    ];
    for c in denied {
        assert!(is_publish_deny(&node_source_decision(c)), "{c}");
    }
    let allowed = [
        "console.log(1+1)",
        "require('fs').readFileSync('scripts/release-neppy.sh','utf8')",
        "console.log('gh release create is what the script does')",
        "require('child_process').execSync('git status')",
        "require('child_process').execSync('pnpm test')",
        // `gh` as a substring of a longer word is not the GitHub CLI
        "require('child_process').execSync('echo high release')",
        "require('child_process').execSync('echo github release notes')",
        // read-only gh calls that merely mention workflow / release
        "require('child_process').execSync('gh run list --workflow ci.yml')",
        "require('child_process').execSync('gh release list')",
        "require('child_process').execSync('gh release view v1 --json tagName')",
        "require('child_process').spawnSync('gh',['run','list','--workflow','release.yml'])",
        "require('child_process').execSync('gh workflow run ci.yml')",
        "require('child_process').execSync('gh api repos/o/r/issues/1/comments -f body=release')",
        // reading the script, a regex .exec( and 'execute' are not spawns
        "const t=require('fs').readFileSync('scripts/release-neppy.sh','utf8'); /x/.exec(t); require('child_process');",
        "const re=/release/; re.exec(s); const cp=require('child_process'); cp.execSync('git log')",
        "require('child_process'); function execute(){ return readFileSync('scripts/release-neppy.sh') }",
        // promisified read-only calls, reads of the script, copies, cat
        "const {promisify}=require('util'); const {exec}=require('child_process'); await promisify(exec)('gh run list --workflow ci.yml')",
        "const cp=require('child_process'); cp.spawnSync('cat',['scripts/release-neppy.sh'])",
        "require('child_process'); require('fs').copyFileSync('scripts/release-neppy.sh','/tmp/x')",
        "require('child_process'); const t=require('fs').readFileSync(require('path').join('scripts','release-neppy.sh'),'utf8')",
        "const cp=require('child_process'); const a=['release','list']; cp.spawnSync('gh', a)",
        "const cp=require('child_process'); const a=['run','list','--workflow','release.yml']; cp.spawnSync('gh', a)",
        "const cp=require('child_process'); cp.spawnSync('bash',['-c','gh run view 1'])",
        r#"require('child_process').execSync("gh api graphql -f query='{ repository(owner:\"o\",name:\"r\") { releases(first:1) { nodes { tagName } } } }'")"#,
        // no process API at all: strings are just strings
        "console.log('sh -c \"gh release create v1\"')",
    ];
    for c in allowed {
        assert_eq!(node_source_decision(c), Allow, "{c}");
    }
    // Not a question: `Ask` never comes out of the source scan.
    assert!(!matches!(
        node_source_decision("spawn('gh',['release'])"),
        Ask(_)
    ));
}

#[tokio::test]
async fn node_exec_gate_denies_in_a_debug_turn_only_for_publish_shaped_code() {
    with_turn(turn_with(DebugModeConfig::default()), async {
        assert!(is_publish_deny(&gate_node_exec_current(
            "require('child_process').execSync('gh release create v1')"
        )));
        assert_eq!(gate_node_exec_current("console.log(2)"), Allow);
    })
    .await;
}
