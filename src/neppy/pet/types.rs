//! Serde types, constants and limits for Pet mode.
//!
//! The wire contract (RPC `neppy.pet_*`) is documented on each type; every
//! timestamp is an RFC3339 UTC string on the wire.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::neppy::security::approval::PendingApproval;

/// Built-in agent id of the background research lane.
pub const PET_RESEARCH_AGENT_ID: &str = "pet_research";
/// `job_name` of the proactive chat message that carries the pet digest (the
/// in-app thread is `proactive:pet_digest`).
pub const PET_DIGEST_JOB_NAME: &str = "pet_digest";
/// Prefix of a pet's `ProactiveMessageRequested.source` (`pet:<pet_id>`).
pub const PET_PROACTIVE_SOURCE_PREFIX: &str = "pet:";
/// Prefix of the research cron job name — the full name is `pet:<pet_id>:research`.
pub const PET_JOB_NAME_PREFIX: &str = "pet:";
/// Prompt the research cron job carries. The agent's own system prompt holds
/// the procedure and hard rules; this is only the per-run kick-off.
pub const PET_RESEARCH_JOB_PROMPT: &str = "Run your research pass now: call pet_context first, \
     follow your procedure, record one pet_note per distinct finding, then reply with one line: \
     `Recorded N notes.`";

/// The closed tool allowlist of the `pet_research` agent. Must equal the
/// `[tools] named` list in `agent/registry/agents/pet_research/agent.toml`
/// (test-enforced). `web_fetch` is deliberately absent: a read-only fetch of an
/// attacker-chosen URL is an exfiltration channel for a lane reading untrusted
/// mail and pages.
///
/// The `composio_*` read tools are ALSO absent, deliberately: they belong to the
/// `composio` tool pack, and a named agent that lists a packed tool it does not
/// own has that schema withheld and `load_skill` / `use_skill` added instead
/// (`tools::toolpacks::strip_packed_from_visible`). `use_skill` dispatches any
/// tool of any pack from the full registry — including non-external config and
/// skill-install tools the approval gate never sees — which would break this
/// closed allowlist. Re-adding them requires making `pet_research` an owner of
/// the `composio` pack first (test-enforced: no allowlisted tool may be packed
/// for this agent). Mail and calendar still reach the lane through synced
/// memory (`pet_recent_memory`).
pub const PET_RESEARCH_TOOL_ALLOWLIST: &[&str] = &[
    "pet_context",
    "pet_recent_memory",
    "pet_note",
    "memory_recall",
    "task_source_list",
    "task_source_list_tasks",
    "task_source_status",
    "web_search_tool",
];

/// Notes one research pass may record.
pub const MAX_NOTES_PER_PASS: usize = 25;
pub const MAX_TITLE: usize = 140;
pub const MAX_BODY: usize = 600;
pub const MAX_ACTION: usize = 280;
pub const MAX_FINGERPRINT: usize = 120;
pub const MAX_GOALS: usize = 20;
pub const MAX_GOAL_TEXT: usize = 280;
pub const MAX_NAME: usize = 40;
pub const MAX_PERSONA: usize = 1000;
pub const MAX_NOTIFY_BUDGET: i64 = 10;
pub const DIGEST_MAX_ITEMS: usize = 12;
/// Default display name of a freshly created pet.
pub const DEFAULT_PET_NAME: &str = "Pet";

/// Declares a lowercase string-backed enum with `as_str`, `parse` and `ALL`.
macro_rules! str_enum {
    ($(#[$m:meta])* $name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "lowercase")]
        pub enum $name { $($variant),+ }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $($name::$variant => $s),+ }
            }

            pub fn parse(raw: &str) -> Option<Self> {
                match raw { $($s => Some($name::$variant),)+ _ => None }
            }
        }
    };
}

str_enum!(
    /// How often the research lane runs (device-local times).
    ResearchPreset { Light => "light", Standard => "standard", Frequent => "frequent" }
);

impl ResearchPreset {
    /// Cron expression for this preset (5-field, device-local timezone).
    pub fn cron_expr(self) -> &'static str {
        match self {
            ResearchPreset::Light => "0 7 * * *",
            ResearchPreset::Standard => "0 7,12,17 * * *",
            ResearchPreset::Frequent => "0 8-20/2 * * *",
        }
    }

    /// Local hours (minute 0) at which this preset runs a pass. Always agrees
    /// with [`Self::cron_expr`] (test-enforced).
    pub fn hours(self) -> Vec<u32> {
        match self {
            ResearchPreset::Light => vec![7],
            ResearchPreset::Standard => vec![7, 12, 17],
            ResearchPreset::Frequent => (8..=20).step_by(2).collect(),
        }
    }

    /// Cron expression for this preset PLUS one extra pass at `digest_hour`.
    /// A digest is only delivered when a pass completes at or after the pet's
    /// digest time, so without this a digest time between two passes (Light
    /// preset at 07:00, digest at 18:00) arrives at the next morning's pass.
    /// The preset's own expression is returned unchanged when it already runs at
    /// that hour.
    pub fn cron_expr_with_digest_hour(self, digest_hour: u32) -> String {
        let mut hours = self.hours();
        if hours.contains(&digest_hour) {
            return self.cron_expr().to_string();
        }
        hours.push(digest_hour);
        hours.sort_unstable();
        let list: Vec<String> = hours.iter().map(u32::to_string).collect();
        format!("0 {} * * *", list.join(","))
    }
}

str_enum!(
    /// A source family the pet may scan (enforced at prompt level in the MVP).
    PetSource { Memory => "memory", Tasks => "tasks", Composio => "composio", Web => "web" }
);

str_enum!(
    /// What a note is about.
    PetNoteKind {
        Deadline => "deadline", Request => "request", Meeting => "meeting", Change => "change",
        Fyi => "fyi", Idea => "idea", Proposal => "proposal",
    }
);

str_enum!(
    /// Where a note's finding came from.
    PetNoteSource {
        Memory => "memory", Tasks => "tasks", Calendar => "calendar", Email => "email",
        // `desktop`: saved from a desktop companion suggestion.
        Web => "web", Other => "other", Desktop => "desktop",
    }
);

str_enum!(
    /// Lifecycle of a note.
    PetNoteState {
        New => "new", Notified => "notified", Queued => "queued", Digested => "digested",
        Dropped => "dropped", Dismissed => "dismissed",
    }
);

str_enum!(
    /// Where the surfacer routed a note.
    Bucket { Notify => "notify", Digest => "digest", Drop => "drop", Duplicate => "duplicate" }
);

str_enum!(
    /// Lifecycle of a proposal.
    ProposalState {
        Pending => "pending", Accepted => "accepted", Dismissed => "dismissed", Expired => "expired",
    }
);

str_enum!(
    /// User decision on a proposal (`pet_proposal_decide`).
    ProposalDecision { Accept => "accept", Dismiss => "dismiss" }
);

/// A user-authored goal that directs the pet's background research.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PetGoal {
    pub id: String,
    pub text: String,
    pub created_at: DateTime<Utc>,
}

/// The pet as returned by `pet_get` / `pet_update`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PetProfile {
    pub id: String,
    pub name: String,
    pub persona: String,
    pub enabled: bool,
    pub research_preset: ResearchPreset,
    /// `HH:MM`, device-local.
    pub digest_time: String,
    pub quiet_start: String,
    pub quiet_end: String,
    pub notify_budget_per_day: u32,
    pub sources: Vec<PetSource>,
    pub goals: Vec<PetGoal>,
    pub research_job_id: Option<String>,
    /// The research cron job's `next_run` while the pet is enabled.
    pub next_research_at: Option<DateTime<Utc>>,
    pub next_digest_at: Option<DateTime<Utc>>,
    pub last_pass_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Partial update for `pet_update`. Unknown keys are rejected.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PetProfilePatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persona: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Validated against [`ResearchPreset`] by `ops` (field-named error).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub research_preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quiet_end: Option<String>,
    /// Signed so an out-of-range value gets a field-named error, not a serde one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_budget_per_day: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<String>>,
}

/// One private research finding recorded by the `pet_note` tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PetNote {
    pub id: String,
    pub pet_id: String,
    pub source: PetNoteSource,
    pub kind: PetNoteKind,
    pub title: String,
    pub body: String,
    pub urgency: u8,
    pub due_at: Option<DateTime<Utc>>,
    pub goal_ids: Vec<String>,
    pub proposed_action: Option<String>,
    /// Dedupe key (`email:<thread_id>`, `task:<id>`, or a title hash).
    pub fingerprint: String,
    pub injection_flagged: bool,
    pub score: Option<u8>,
    pub bucket: Option<Bucket>,
    pub state: PetNoteState,
    pub digest_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub surfaced_at: Option<DateTime<Utc>>,
    pub notified_at: Option<DateTime<Utc>>,
}

/// An assembled digest (markdown body, links and images neutralised).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PetDigest {
    pub id: String,
    pub pet_id: String,
    pub created_at: DateTime<Utc>,
    /// `YYYY-MM-DD`, device-local.
    pub local_date: String,
    pub body_md: String,
    pub item_count: u32,
    pub withheld_count: u32,
}

/// An action the research lane suggested. Accepting it never executes
/// anything: it returns a chat prompt the user reviews and sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PetProposal {
    pub id: String,
    pub pet_id: String,
    pub note_id: String,
    pub note_title: String,
    pub action_text: String,
    pub state: ProposalState,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

/// Outcome of one research pass plus its surfacing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PetRunSummary {
    pub run_id: Option<String>,
    /// `started` (async `pet_run_now`), `completed` or `failed`.
    pub status: String,
    /// `scheduled` or `manual`.
    pub trigger: String,
    pub finished_at: Option<DateTime<Utc>>,
    pub notes_seen: u32,
    pub notified: u32,
    pub queued: u32,
    pub dropped: u32,
    pub digest_id: Option<String>,
}

impl PetRunSummary {
    /// The placeholder returned by a fire-and-forget `pet_run_now`.
    pub fn started(trigger: &str) -> Self {
        Self {
            run_id: None,
            status: "started".into(),
            trigger: trigger.into(),
            finished_at: None,
            notes_seen: 0,
            notified: 0,
            queued: 0,
            dropped: 0,
            digest_id: None,
        }
    }
}

/// `pet_feed` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetFeed {
    /// Newest 7 digests, newest first.
    pub digests: Vec<PetDigest>,
    /// Notes in state `notified | queued | digested`, newest first.
    pub notes: Vec<PetNote>,
    pub last_run: Option<PetRunSummary>,
}

/// `pet_inbox_list` result.
#[derive(Debug, Clone, Serialize)]
pub struct PetInbox {
    pub proposals: Vec<PetProposal>,
    /// Pending background approvals that have no chat card.
    pub approvals: Vec<PendingApproval>,
}

/// `pet_proposal_decide` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalDecisionResult {
    pub proposal: PetProposal,
    /// Set only on `accept` — seeded into a new chat composer, never sent.
    pub chat_prompt: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_round_trip_through_as_str_and_serde() {
        for kind in PetNoteKind::ALL {
            assert_eq!(PetNoteKind::parse(kind.as_str()), Some(*kind));
            let wire = serde_json::to_value(kind).unwrap();
            assert_eq!(wire, serde_json::json!(kind.as_str()));
        }
        for state in PetNoteState::ALL {
            assert_eq!(PetNoteState::parse(state.as_str()), Some(*state));
        }
        for s in PetNoteSource::ALL {
            assert_eq!(PetNoteSource::parse(s.as_str()), Some(*s));
        }
        for b in Bucket::ALL {
            assert_eq!(Bucket::parse(b.as_str()), Some(*b));
        }
        assert_eq!(PetNoteKind::parse("bogus"), None);
    }

    #[test]
    fn presets_map_to_cron_expressions() {
        assert_eq!(ResearchPreset::Light.cron_expr(), "0 7 * * *");
        assert_eq!(ResearchPreset::Standard.cron_expr(), "0 7,12,17 * * *");
        assert_eq!(ResearchPreset::Frequent.cron_expr(), "0 8-20/2 * * *");
        for preset in ResearchPreset::ALL {
            let schedule = crate::neppy::cron::Schedule::Cron {
                expr: preset.cron_expr().into(),
                tz: None,
                active_hours: None,
            };
            crate::neppy::cron::validate_schedule(&schedule, Utc::now())
                .unwrap_or_else(|e| panic!("preset {preset:?} must be a valid schedule: {e}"));
        }
    }

    #[test]
    fn allowlist_excludes_web_fetch_and_write_tools() {
        assert!(!PET_RESEARCH_TOOL_ALLOWLIST.contains(&"web_fetch"));
        assert!(!PET_RESEARCH_TOOL_ALLOWLIST.contains(&"http_request"));
        assert!(!PET_RESEARCH_TOOL_ALLOWLIST.contains(&"memory_tree"));
        assert!(!PET_RESEARCH_TOOL_ALLOWLIST.contains(&"memory_store"));
        assert!(PET_RESEARCH_TOOL_ALLOWLIST.contains(&"pet_note"));
    }

    #[test]
    fn patch_rejects_unknown_fields() {
        let err = serde_json::from_value::<PetProfilePatch>(serde_json::json!({ "bogus": 1 }))
            .unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        let ok: PetProfilePatch =
            serde_json::from_value(serde_json::json!({ "enabled": true })).unwrap();
        assert_eq!(ok.enabled, Some(true));
    }
}
