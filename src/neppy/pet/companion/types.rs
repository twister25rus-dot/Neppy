//! Wire-stable types for the Pet desktop companion.
//!
//! Two families live here, and the split is the privacy boundary:
//!
//! * **Serialisable** types (`CompanionSuggestion`, `ObservationSummary`,
//!   `CompanionActionLog`, the enums) carry only scrubbed excerpts and metadata.
//! * **[`ObservationEvent`]** carries the scrubbed observed text and has NO
//!   `Serialize` impl, so it cannot be persisted, logged as JSON or put on a
//!   socket by accident (compile-time guard in `types_tests.rs`).
//!
//! Timestamps are RFC3339 UTC on the wire.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};

use super::sensitive::Scrubbed;

/// Declares a string-backed enum with explicit wire names, `as_str`, `parse`, `ALL`.
macro_rules! str_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $variant:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum $name { $($(#[$vm])* #[serde(rename = $s)] $variant),+ }

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

/// Log-target prefix for every companion log line.
pub const LOG_PREFIX: &str = "[pet::companion]";

/// Autonomy level of the companion. Serialised as the number 0..=3.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize_repr, Deserialize_repr,
)]
#[repr(u8)]
pub enum CompanionLevel {
    /// Watches and learns; shows nothing proactively.
    Observe = 0,
    /// Proposes suggestions; the user clicks to act.
    Suggest = 1,
    /// May run low-risk reversible categories without a click.
    Assist = 2,
    /// Per-category trusted automation (needs a `full` global tier).
    Trusted = 3,
}

impl CompanionLevel {
    pub const ALL: &'static [CompanionLevel] = &[
        CompanionLevel::Observe,
        CompanionLevel::Suggest,
        CompanionLevel::Assist,
        CompanionLevel::Trusted,
    ];

    pub fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::Observe),
            1 => Some(Self::Suggest),
            2 => Some(Self::Assist),
            3 => Some(Self::Trusted),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Suggest => "suggest",
            Self::Assist => "assist",
            Self::Trusted => "trusted",
        }
    }
}

str_enum!(
    /// What the companion may observe. `ScreenCapture` is autonomous, rate
    /// limited screen capture + on-device OCR (images are never stored).
    CompanionSource {
        AppWindow => "app_window",
        Selection => "selection",
        Clipboard => "clipboard",
        ScreenCapture => "screen_capture",
    }
);

/// Sources reported read-only as "not available in this version".
pub const UNAVAILABLE_SOURCES: &[&str] = &[
    "browser_content",
    "notifications",
    "recent_actions",
    "related_files",
];

str_enum!(
    /// What an action does. The last nine are high-risk: fixed at "ask", the
    /// companion never executes them, at any level, for anyone.
    ActionCategory {
        Explain => "explain",
        DraftText => "draft_text",
        FormatText => "format_text",
        SaveNote => "save_note",
        PrepareCommand => "prepare_command",
        OpenChat => "open_chat",
        HandoffTask => "handoff_task",
        SendMessage => "send_message",
        Delete => "delete",
        Purchase => "purchase",
        Publish => "publish",
        SystemSettings => "system_settings",
        Install => "install",
        PrivilegedCommand => "privileged_command",
        SharePersonalInfo => "share_personal_info",
        Irreversible => "irreversible",
    }
);

impl ActionCategory {
    /// Categories that must ALWAYS be confirmed and are never executed
    /// autonomously (send message/email, delete, purchase, publish, system
    /// settings, install, privileged command, share personal info, irreversible).
    pub const HIGH_RISK: &'static [ActionCategory] = &[
        ActionCategory::SendMessage,
        ActionCategory::Delete,
        ActionCategory::Purchase,
        ActionCategory::Publish,
        ActionCategory::SystemSettings,
        ActionCategory::Install,
        ActionCategory::PrivilegedCommand,
        ActionCategory::SharePersonalInfo,
        ActionCategory::Irreversible,
    ];

    pub fn is_high_risk(self) -> bool {
        Self::HIGH_RISK.contains(&self)
    }

    /// Medium risk: runs a background hand-off under the normal approval gate.
    pub fn is_medium_risk(self) -> bool {
        self == ActionCategory::HandoffTask
    }

    /// Default per-category level (high-risk pinned at 1 = ask).
    pub fn default_level(self) -> CompanionLevel {
        match self {
            Self::Explain | Self::DraftText | Self::SaveNote => CompanionLevel::Assist,
            _ => CompanionLevel::Suggest,
        }
    }
}

str_enum!(
    /// What kind of situation a suggestion is about.
    TriggerKind {
        BuildError => "build_error",
        EmailDraft => "email_draft",
        Term => "term",
        Ask => "ask",
        Capture => "capture",
    }
);

impl TriggerKind {
    /// Kinds the companion may raise on its own (and the user may mute).
    pub fn is_proactive(self) -> bool {
        matches!(self, Self::BuildError | Self::EmailDraft | Self::Term)
    }

    /// The action category a suggestion of this kind belongs to by default.
    pub fn default_category(self) -> ActionCategory {
        match self {
            Self::BuildError => ActionCategory::Explain,
            Self::EmailDraft => ActionCategory::DraftText,
            Self::Term => ActionCategory::Explain,
            Self::Ask | Self::Capture => ActionCategory::Explain,
        }
    }
}

str_enum!(
    /// How a suggestion came to exist.
    SuggestionTrigger { Proactive => "proactive", Ask => "ask", Capture => "capture" }
);

str_enum!(
    SuggestionState {
        New => "new",
        Shown => "shown",
        Acted => "acted",
        Saved => "saved",
        Dismissed => "dismissed",
        Expired => "expired",
    }
);

str_enum!(
    /// Actions a suggestion offers / the user may take on it.
    SuggestionAction {
        Explain => "explain",
        Draft => "draft",
        CopyText => "copy_text",
        SaveNote => "save_note",
        OpenChat => "open_chat",
        PrepareCommand => "prepare_command",
        Handoff => "handoff",
        Dismiss => "dismiss",
        MuteKind => "mute_kind",
        MuteApp => "mute_app",
    }
);

str_enum!(
    /// How chatty proactive suggestions are (usefulness threshold 75/60/45).
    Chattiness { Quiet => "quiet", Normal => "normal", Eager => "eager" }
);

str_enum!(
    HandoffStatus { Running => "running", Done => "done", Failed => "failed" }
);

str_enum!(
    /// How an audited action was decided.
    ActionDecision {
        Auto => "auto",
        Confirmed => "confirmed",
        RefusedHighRisk => "refused_high_risk",
        BlockedPolicy => "blocked_policy",
    }
);

str_enum!(
    ActionOutcome { Ok => "ok", Error => "error" }
);

str_enum!(
    /// What changed on screen.
    ObservationKind {
        AppSwitch => "app_switch",
        TitleChange => "title_change",
        Selection => "selection",
        Clipboard => "clipboard",
        Capture => "capture",
        Ask => "ask",
    }
);

str_enum!(
    /// Why an observation was discarded before it became an event.
    DropReason {
        Paused => "paused",
        NoIndicator => "no_indicator",
        ExcludedApp => "excluded_app",
        TitleRule => "title_rule",
        SecureField => "secure_field",
        ConcealedClipboard => "concealed_clipboard",
        SensitiveContent => "sensitive_content",
        SourceOff => "source_off",
        PermissionMissing => "permission_missing",
    }
);

/// Flags carried on an observation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ObservationFlags {
    /// Something was redacted from `title` / `text`.
    pub redacted: bool,
    /// The user explicitly asked (hotkey ask / manual capture). Autonomous
    /// captures leave this false.
    pub user_initiated: bool,
}

/// One accepted observation. In memory only: **no `Serialize` impl**.
/// `title` / `text` are [`Scrubbed`], which only `sensitive::scrub` can build.
#[derive(Debug, Clone)]
pub struct ObservationEvent {
    pub at: DateTime<Utc>,
    pub kind: ObservationKind,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub title: Option<Scrubbed>,
    pub text: Option<Scrubbed>,
    pub flags: ObservationFlags,
}

/// Max chars of a title excerpt in a summary.
pub const SUMMARY_TITLE_CHARS: usize = 80;

impl ObservationEvent {
    /// Serialisable "what the pet sees" row (scrubbed title excerpt only).
    pub fn summary(&self) -> ObservationSummary {
        ObservationSummary {
            at: self.at,
            kind: self.kind,
            app_name: self.app_name.clone(),
            title_excerpt: self
                .title
                .as_ref()
                .map(|t| t.excerpt(SUMMARY_TITLE_CHARS))
                .filter(|t| !t.is_empty()),
            dropped: None,
        }
    }

    /// Approximate in-memory text size, for the buffer budget.
    pub fn text_bytes(&self) -> usize {
        self.title.as_ref().map_or(0, Scrubbed::len) + self.text.as_ref().map_or(0, Scrubbed::len)
    }
}

/// Serialisable summary for the Now tab. Never carries body text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservationSummary {
    pub at: DateTime<Utc>,
    pub kind: ObservationKind,
    pub app_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<DropReason>,
}

/// Background hand-off attached to a suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handoff {
    pub thread_id: String,
    pub status: HandoffStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_excerpt: Option<String>,
}

/// A stored suggestion (wire shape of `CompanionSuggestion`). All text fields
/// are scrubbed and capped before they reach storage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanionSuggestion {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub trigger: SuggestionTrigger,
    pub kind: TriggerKind,
    pub category: ActionCategory,
    pub app_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    pub title_excerpt: String,
    pub context_excerpt: String,
    pub headline: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub state: SuggestionState,
    pub score: i32,
    pub actions: Vec<SuggestionAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<Handoff>,
}

/// Audit row. Never carries content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionActionLog {
    pub id: String,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion_id: Option<String>,
    pub category: ActionCategory,
    pub decision: ActionDecision,
    pub level: CompanionLevel,
    pub outcome: ActionOutcome,
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod types_tests;
