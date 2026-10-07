// The config env lock is a std mutex held across awaits on purpose (see
// `config::ops_tests`): it serialises `NEPPY_WORKSPACE` mutation.
#![allow(clippy::await_holding_lock)]

use super::*;
use crate::neppy::agent::debug_mode::ops::DebugCtx;
use crate::neppy::agent::debug_mode::test_util::repo;
use crate::neppy::config::TEST_ENV_LOCK as ENV_LOCK;

fn base() -> DebugModeConfig {
    DebugModeConfig::default()
}

fn patch(v: Value) -> SettingsPatch {
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn partial_patch_changes_only_the_named_fields() {
    let next = apply_patch(
        &base(),
        patch(serde_json::json!({"allow_git_push": true, "max_repair_iterations": 7})),
    )
    .await
    .unwrap();
    assert!(next.allow_git_push);
    assert_eq!(next.max_repair_iterations, 7);
    let expect = DebugModeConfig {
        allow_git_push: true,
        max_repair_iterations: 7,
        ..base()
    };
    assert_eq!(next, expect);
}

#[tokio::test]
async fn max_repair_iterations_is_range_checked() {
    for bad in [0, 21, -1, 1000] {
        let e = apply_patch(
            &base(),
            patch(serde_json::json!({ "max_repair_iterations": bad })),
        )
        .await
        .unwrap_err();
        assert!(e.contains("between 1 and 20"), "{bad}: {e}");
    }
    for ok in [1, 5, 20] {
        let n = apply_patch(
            &base(),
            patch(serde_json::json!({ "max_repair_iterations": ok })),
        )
        .await
        .unwrap();
        assert_eq!(n.max_repair_iterations, ok as u32);
    }
}

#[test]
fn unknown_fields_and_wrong_types_are_rejected() {
    assert!(serde_json::from_value::<SettingsPatch>(serde_json::json!({"bogus": 1})).is_err());
    assert!(
        serde_json::from_value::<SettingsPatch>(serde_json::json!({"enabled": "yes"})).is_err()
    );
    assert!(serde_json::from_value::<SettingsPatch>(
        serde_json::json!({"max_repair_iterations": 1.5})
    )
    .is_err());
}

#[tokio::test]
async fn project_root_must_be_a_git_work_tree_root_and_null_clears_it() {
    let repo = repo();
    let canon = std::fs::canonicalize(repo.path())
        .unwrap()
        .display()
        .to_string();
    let n = apply_patch(
        &base(),
        patch(serde_json::json!({ "project_root": repo.path() })),
    )
    .await
    .unwrap();
    assert_eq!(n.project_root.as_deref(), Some(canon.as_str()));

    let cleared = apply_patch(&n, patch(serde_json::json!({ "project_root": null })))
        .await
        .unwrap();
    assert!(cleared.project_root.is_none());
    let blank = apply_patch(&n, patch(serde_json::json!({ "project_root": "  " })))
        .await
        .unwrap();
    assert!(blank.project_root.is_none());
    let untouched = apply_patch(&n, patch(serde_json::json!({}))).await.unwrap();
    assert_eq!(untouched.project_root.as_deref(), Some(canon.as_str()));

    let plain = tempfile::tempdir().unwrap();
    let e = apply_patch(
        &base(),
        patch(serde_json::json!({ "project_root": plain.path() })),
    )
    .await
    .unwrap_err();
    assert!(e.starts_with("invalid project_root"), "{e}");
    let sub = repo.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    assert!(
        apply_patch(&base(), patch(serde_json::json!({ "project_root": sub })))
            .await
            .is_err()
    );
    assert!(apply_patch(
        &base(),
        patch(serde_json::json!({ "project_root": "/no/such/dir" }))
    )
    .await
    .is_err());
}

#[tokio::test]
async fn external_paths_are_validated_canonicalised_and_deduplicated() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().to_str().unwrap();
    let n = apply_patch(
        &base(),
        patch(serde_json::json!({ "external_paths": [p, p] })),
    )
    .await
    .unwrap();
    assert_eq!(
        n.external_paths,
        vec![std::fs::canonicalize(d.path())
            .unwrap()
            .display()
            .to_string()]
    );
    for bad in ["/", "rel/path", "/no/such/dir/xyz", "/etc"] {
        let e = apply_patch(
            &base(),
            patch(serde_json::json!({ "external_paths": [p, bad] })),
        )
        .await
        .unwrap_err();
        assert!(e.contains("external path"), "{bad}: {e}");
    }
    let cleared = apply_patch(&n, patch(serde_json::json!({ "external_paths": [] })))
        .await
        .unwrap();
    assert!(cleared.external_paths.is_empty());
}

#[test]
fn audit_names_fields_only() {
    let p = patch(
        serde_json::json!({"allow_git_push": true, "project_root": "/secret/place", "external_paths": ["/x"]}),
    );
    let names = p.field_names();
    assert_eq!(
        names,
        vec!["project_root", "external_paths", "allow_git_push"]
    );
}

#[tokio::test]
async fn update_persists_audits_names_only_and_get_reads_it_back() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("NEPPY_WORKSPACE", tmp.path()) };

    let ctx = DebugCtx::new(tmp.path());
    let before = get(&ctx).await.unwrap().value;
    assert_eq!(before, base());

    let mut m = Map::new();
    m.insert("enabled".into(), Value::Bool(false));
    m.insert("project_root".into(), Value::Null);
    m.insert("max_repair_iterations".into(), Value::from(9));
    let after = update(&ctx, m).await.unwrap().value;
    assert!(!after.enabled && after.max_repair_iterations == 9);
    assert_eq!(get(&ctx).await.unwrap().value, after, "persisted");
    assert_eq!(load().await.unwrap(), after);

    // A disabled Debug Mode refuses the real `turn::run` before doing anything.
    let ws = tempfile::tempdir().unwrap();
    let ran = std::sync::atomic::AtomicBool::new(false);
    let err = crate::neppy::agent::debug_mode::turn::run(ws.path(), "go", async {
        ran.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok::<_, String>(())
    })
    .await
    .unwrap_err();
    assert!(err.contains("turned off"), "{err}");
    assert!(!ran.load(std::sync::atomic::Ordering::SeqCst));

    // Invalid updates change nothing.
    let mut bad = Map::new();
    bad.insert("max_repair_iterations".into(), Value::from(99));
    assert!(update(&ctx, bad).await.is_err());
    let mut unknown = Map::new();
    unknown.insert("nope".into(), Value::Bool(true));
    assert!(update(&ctx, unknown).await.is_err());
    assert_eq!(load().await.unwrap(), after);

    // The audit log records field names, never values.
    let audit = ctx.store.audit_tail(50).unwrap();
    let upd: Vec<_> = audit.iter().filter(|a| a.op == "settings_update").collect();
    assert_eq!(upd.len(), 3);
    assert_eq!(upd[0].target, "enabled,project_root,max_repair_iterations");
    assert!(upd[0].outcome == "ok" && upd[1].outcome.starts_with("error"));
    assert!(!audit
        .iter()
        .any(|a| a.target.contains("false") || a.target.contains('9')));

    crate::neppy::util::env::remove_var("NEPPY_WORKSPACE");
}

#[tokio::test]
async fn commit_refuses_when_git_commit_is_disabled() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let cfg = DebugModeConfig {
        allow_git_commit: false,
        ..base()
    };
    let ctx = DebugCtx::new(ws.path()).with_settings(&cfg);
    let err = crate::neppy::agent::debug_mode::ops::task_start(&ctx, "x")
        .await
        .unwrap()
        .value;
    let e = super::super::commit::commit(&ctx, repo.path().to_str(), &err.id, "msg", true)
        .await
        .unwrap_err();
    assert!(e.contains("allow_git_commit"), "{e}");
}

#[tokio::test]
async fn configured_project_root_slots_between_env_and_default() {
    use crate::neppy::agent::debug_mode::git::resolve_project_root_with;
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (a, b) = (repo(), repo());
    let ca = std::fs::canonicalize(a.path()).unwrap();
    let cb = std::fs::canonicalize(b.path()).unwrap();
    let env = crate::neppy::agent::debug_mode::types::PROJECT_ROOT_ENV;
    crate::neppy::util::env::remove_var(env);

    // config only
    let r = resolve_project_root_with(None, a.path().to_str())
        .await
        .unwrap();
    assert_eq!(r, ca);
    // param beats config
    let r = resolve_project_root_with(b.path().to_str(), a.path().to_str())
        .await
        .unwrap();
    assert_eq!(r, cb);
    // env beats config
    unsafe { std::env::set_var(env, b.path()) };
    let r = resolve_project_root_with(None, a.path().to_str())
        .await
        .unwrap();
    assert_eq!(r, cb);
    crate::neppy::util::env::remove_var(env);
    // neither: the build-time default (this crate's own repo)
    let r = resolve_project_root_with(None, None).await;
    assert!(r.is_ok() || r.unwrap_err().contains("git work tree"));
}
