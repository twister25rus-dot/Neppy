//! `/teams/*` — a single-member local team.
//!
//! # What "usage" means with no metered account
//!
//! The hosted `/teams/me/usage` reports spend against a subscription budget.
//! Locally there is no budget, but there *is* real spend whenever a workload
//! runs on a BYOK provider, and the on-device cost tracker already records it.
//! So the payload carries the true numbers for the fields that have a local
//! meaning (`spentThisCycleUsd`, `spentTodayUsd`) and neutral values for the
//! ones that do not.
//!
//! # The one field that must be exactly right
//!
//! `bypassCycleLimit: true`. [`usage_budget_exhausted`](crate::openhuman::hosted::team)
//! treats a payload as exhausted when `remainingUsd <= 0.01` **and** any of the
//! cycle-limit fields is positive; the managed-tool budget gate then refuses
//! calls. A local install has `remainingUsd: 0` forever, so without an explicit
//! bypass any later change that made one of those fields non-zero would silently
//! disable managed tools. `bypassCycleLimit` states the invariant directly
//! rather than relying on the arithmetic staying favourable.

use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::openhuman::local_mode::backend::state::LocalBackendState;

pub(crate) fn router() -> Router<LocalBackendState> {
    Router::new()
        .route("/teams", get(list_teams))
        .route("/teams/me/usage", get(usage))
}

/// The one team: this device.
fn local_team(state: &LocalBackendState) -> serde_json::Value {
    json!({
        "id": state.identity.id,
        "_id": state.identity.id,
        "name": "Local",
        "role": "owner",
        "plan": "local",
        "members": [ &*state.identity ],
        "memberCount": 1,
    })
}

/// `GET /teams`
async fn list_teams(State(state): State<LocalBackendState>) -> impl IntoResponse {
    Json(json!({ "success": true, "data": [local_team(&state)] }))
}

/// Locally-recorded spend, as `(this cycle, today)` in USD.
///
/// Returns zeros when no tracker is installed — which is the honest answer for
/// a process that has not run a priced workload yet, and keeps this route
/// working in contexts (tests, a fresh boot) where the global tracker has not
/// been initialised. A tracker read that fails is logged and treated the same:
/// a usage readout is not worth failing the whole request over.
fn local_spend() -> (f64, f64) {
    let Some(tracker) = crate::openhuman::platform::cost::try_global() else {
        return (0.0, 0.0);
    };
    match tracker.get_summary() {
        Ok(summary) => (summary.monthly_cost_usd, summary.daily_cost_usd),
        Err(error) => {
            tracing::debug!(
                error = %error,
                "{} cost summary unavailable — reporting zero spend",
                crate::openhuman::local_mode::backend::LOG_PREFIX
            );
            (0.0, 0.0)
        }
    }
}

/// `GET /teams/me/usage`
async fn usage() -> impl IntoResponse {
    let (cycle_spent, today_spent) = local_spend();
    Json(json!({
        "success": true,
        "data": {
            // No budget exists locally, so no budget is reported.
            "cycleBudgetUsd": 0.0,
            "remainingUsd": 0.0,
            "cycleLimit5hr": 0.0,
            "cycleLimit7day": 0.0,
            "fiveHourCapUsd": 0.0,
            "fiveHourResetsAt": serde_json::Value::Null,
            "cycleStartDate": serde_json::Value::Null,
            "cycleEndsAt": serde_json::Value::Null,
            // Real numbers: BYOK spend is genuine spend and the tracker has it.
            "spentThisCycleUsd": cycle_spent,
            "spentTodayUsd": today_spent,
            "cycleSpentUsd": cycle_spent,
            // See the module docs — this is the field that keeps the managed
            // budget gate from ever refusing a call on a local install.
            "bypassCycleLimit": true,
            // Names the source so a UI or a support log can tell a local
            // reading from a hosted one without inferring it from the zeros.
            "source": "local",
        }
    }))
}
