use std::time::Duration;

use super::*;
use crate::neppy::agent::debug_mode::test_util::repo;
use crate::neppy::config::schema::debug_mode::DebugModeConfig;

const TEN_MIN_MS: u64 = 600_000;

fn cfg(secs: u64) -> DebugModeConfig {
    DebugModeConfig {
        turn_timeout_secs: secs,
        ..Default::default()
    }
}

#[test]
fn a_debug_turn_gets_an_hour_where_chat_gets_ten_minutes() {
    assert_eq!(
        turn_wall_clock_ms(Some(TEN_MIN_MS), &DebugModeConfig::default()),
        Some(3_600_000)
    );
}

#[test]
fn the_debug_budget_never_shrinks_below_the_base() {
    assert_eq!(
        turn_wall_clock_ms(Some(TEN_MIN_MS), &cfg(60)),
        Some(TEN_MIN_MS)
    );
}

#[test]
fn zero_means_no_ceiling_and_an_operator_opt_out_is_honoured() {
    assert_eq!(turn_wall_clock_ms(Some(TEN_MIN_MS), &cfg(0)), None);
    assert_eq!(turn_wall_clock_ms(None, &cfg(7200)), None);
}

#[test]
fn the_outer_backstop_sits_above_the_harness_budget() {
    let base = Some(Duration::from_secs(900));
    assert_eq!(
        web_backstop(base, &DebugModeConfig::default()),
        Some(Duration::from_secs(3600 + 300))
    );
    assert_eq!(web_backstop(base, &cfg(0)), None);
    assert_eq!(web_backstop(None, &cfg(3600)), None);
    assert_eq!(web_backstop(base, &cfg(10)), base, "never below the base");
}

#[test]
fn outside_a_debug_turn_nothing_changes() {
    assert_eq!(
        current_turn_wall_clock_ms(Some(TEN_MIN_MS)),
        Some(TEN_MIN_MS)
    );
    assert_eq!(current_turn_wall_clock_ms(None), None);
    assert_eq!(default_tool_timeout_secs(), None);
}

#[tokio::test]
async fn inside_a_debug_turn_the_larger_budget_and_a_tool_deadline_apply() {
    let (repo, ws) = (repo(), tempfile::tempdir().unwrap());
    let root = turn::resolve_root(Some(repo.path().to_str().unwrap()))
        .await
        .unwrap();
    let seen = turn::run_in_root(ws.path(), root, "build it", async {
        Ok::<_, String>((
            current_turn_wall_clock_ms(Some(TEN_MIN_MS)),
            default_tool_timeout_secs(),
        ))
    })
    .await
    .unwrap();
    assert_eq!(seen.0, Some(3_600_000));
    assert_eq!(seen.1, Some(DEFAULT_TOOL_TIMEOUT_SECS));
}
