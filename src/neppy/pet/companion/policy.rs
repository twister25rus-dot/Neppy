//! Autonomy policy: how a category at a level becomes a decision.
//!
//! `effective_level = min(companion level, category level, global tier cap)`
//! (`ReadOnly`->1, `Supervised`->2, `Full`->3). High-risk categories are pinned
//! at <= 1 and `decide` returns [`Decision::Refuse`] for them at EVERY level,
//! tier, initiator and approval setting, including "approve everything":
//! send message/email, delete, purchase, publish, system settings, install,
//! privileged command, share personal info and irreversible changes are never
//! executed autonomously. Any such work happens only inside a hand-off run,
//! where the approval gate parks it for a human decision.
//!
//! Low and medium categories follow the configured level and the user's global
//! settings: the tier caps the level, and "approve everything" lets the medium
//! `handoff_task` run automatically from `Assist` up (without it, hand-off
//! needs `Trusted`).
//!
//! Levels: 0 observe (drop proactive), 1 suggest (click to act), 2 assist
//! (explain / draft_text / save_note run without a click), 3 trusted (plus
//! `handoff_task` for categories the user marked 3).

use crate::neppy::security::policy::AutonomyLevel;

use super::settings::CompanionSettings;
use super::types::{ActionCategory, ActionDecision, CompanionLevel};

/// What to do with a candidate action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Discard (level 0, proactive).
    Drop,
    /// Show it; the user decides.
    Suggest,
    /// Execute once the user clicks / confirms.
    ExecuteOnConfirm,
    /// Execute now, no click (never for high-risk).
    ExecuteAuto,
    /// High-risk: the companion never executes it.
    Refuse,
}

impl Decision {
    /// Executes without a human decision.
    pub fn is_autonomous(self) -> bool {
        self == Decision::ExecuteAuto
    }

    /// The audit-log decision matching this outcome.
    pub fn audit(self) -> ActionDecision {
        match self {
            Decision::ExecuteAuto => ActionDecision::Auto,
            Decision::ExecuteOnConfirm => ActionDecision::Confirmed,
            Decision::Refuse => ActionDecision::RefusedHighRisk,
            Decision::Drop | Decision::Suggest => ActionDecision::BlockedPolicy,
        }
    }
}

/// The user's global settings the companion follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PolicyCtx {
    /// Global agent tier (`security.autonomy.level`).
    pub tier: AutonomyLevel,
    /// The user's global "approve everything" switch.
    pub auto_approve_all: bool,
    /// The user clicked / asked (vs a proactive companion decision).
    pub initiated_by_user: bool,
}

pub fn tier_cap(tier: AutonomyLevel) -> CompanionLevel {
    match tier {
        AutonomyLevel::ReadOnly => CompanionLevel::Suggest,
        AutonomyLevel::Supervised => CompanionLevel::Assist,
        AutonomyLevel::Full => CompanionLevel::Trusted,
    }
}

pub fn effective_level(
    settings: &CompanionSettings,
    category: ActionCategory,
    tier: AutonomyLevel,
) -> CompanionLevel {
    let e = settings
        .level
        .min(settings.category_level(category))
        .min(tier_cap(tier));
    if category.is_high_risk() {
        e.min(CompanionLevel::Suggest)
    } else {
        e
    }
}

/// Categories that may run without a click at `level`.
fn auto_capable(category: ActionCategory, level: CompanionLevel, auto_approve_all: bool) -> bool {
    match category {
        ActionCategory::Explain | ActionCategory::DraftText | ActionCategory::SaveNote => {
            level >= CompanionLevel::Assist
        }
        ActionCategory::HandoffTask => {
            level >= CompanionLevel::Trusted
                || (level >= CompanionLevel::Assist && auto_approve_all)
        }
        // format_text / prepare_command / open_chat produce output the user
        // still has to click to use.
        _ => false,
    }
}

pub fn decide(settings: &CompanionSettings, category: ActionCategory, ctx: &PolicyCtx) -> Decision {
    if category.is_high_risk() {
        return Decision::Refuse;
    }
    let level = effective_level(settings, category, ctx.tier);
    match level {
        CompanionLevel::Observe => {
            if ctx.initiated_by_user {
                Decision::Suggest
            } else {
                Decision::Drop
            }
        }
        CompanionLevel::Suggest => {
            if ctx.initiated_by_user {
                Decision::ExecuteOnConfirm
            } else {
                Decision::Suggest
            }
        }
        CompanionLevel::Assist | CompanionLevel::Trusted => {
            if auto_capable(category, level, ctx.auto_approve_all) {
                Decision::ExecuteAuto
            } else if ctx.initiated_by_user {
                Decision::ExecuteOnConfirm
            } else {
                Decision::Suggest
            }
        }
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod policy_tests;
