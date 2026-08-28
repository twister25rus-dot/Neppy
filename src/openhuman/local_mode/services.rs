//! The hosted-service → local-replacement inventory.
//!
//! This is the **single source of truth** for what running without the hosted
//! backend actually means, and it has three consumers that must never
//! disagree:
//!
//! 1. [`backend`](super::backend) — the local backend answers an unimplemented
//!    hosted route with the entry's `local_alternative`, so the client gets
//!    guidance instead of a 404 or a fabricated success.
//! 2. The `local_mode.services` RPC — the Settings UI renders this list.
//! 3. `docs/LOCAL_MODE.md` — the user-facing table. It is hand-written (the
//!    prose per entry is longer than a generator would produce well), and
//!    [`tests`] asserts every entry id appears in it, so an entry added here
//!    without a doc row fails the suite rather than shipping undocumented.
//!
//! ## Why entries are honest about what is missing
//!
//! Several hosted features are thin wrappers over a third-party SaaS. We may
//! not reimplement Composio's action catalogue, Exa's crawl index, Stripe's
//! billing, or the weights behind Seedream / Seedance / Veo, and pretending
//! otherwise would be worse than saying so: an agent that receives a
//! plausible-looking fake search result acts on it. So each entry declares a
//! [`LocalReplacementKind`], and only [`LocalReplacementKind::Replaced`] means
//! "this works, locally, with the same UI".

use serde::{Deserialize, Serialize};

/// How completely a hosted service is replaced when local mode is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalReplacementKind {
    /// A local implementation ships in the app and is wired up automatically.
    /// The feature works with no extra setup beyond what local mode already
    /// arranges.
    Replaced,
    /// A local implementation ships, but it needs something the user must
    /// stand up or supply first (a model pulled into Ollama, a SearXNG
    /// instance, their own third-party API key). The UI should surface the
    /// `setup` string as the next action.
    RequiresSetup,
    /// The hosted feature is a wrapper over a third-party service that cannot
    /// be reproduced locally. `local_alternative` names the different thing
    /// the app offers instead; the feature itself stays off.
    Unavailable,
    /// The hosted feature has no meaning without the hosted service and is
    /// simply absent (billing, referrals, the hosted agent marketplace).
    /// Not a degradation — there is nothing to pay for or refer to.
    NotApplicable,
}

impl LocalReplacementKind {
    /// Whether the app should keep offering the feature in local mode.
    ///
    /// [`Self::RequiresSetup`] counts as available: the surface stays, and the
    /// UI prompts for the missing piece rather than hiding the feature.
    pub fn is_available(self) -> bool {
        matches!(self, Self::Replaced | Self::RequiresSetup)
    }
}

/// One hosted service and what stands in for it locally.
///
/// `Serialize` only. The entries are a `&'static` table compiled into the
/// binary — nothing ever reconstructs one from JSON, and deriving
/// `Deserialize` over `&'static str` fields would force every field to an
/// owned `String` for a round-trip that has no caller.
#[derive(Debug, Clone, Serialize)]
pub struct LocalServiceEntry {
    /// Stable dotted id — the key the UI and docs join on. Never renamed once
    /// shipped; a rename is a new id plus a migration.
    pub id: &'static str,
    /// Human-readable name of the hosted capability.
    pub hosted: &'static str,
    /// The backend route prefixes this capability owns. Used by the local
    /// backend to attach the right entry to an unimplemented route, so the
    /// error a client receives names the alternative for *that* call.
    pub routes: &'static [&'static str],
    /// What replaces it locally.
    pub kind: LocalReplacementKind,
    /// The local replacement, in one line.
    pub local_alternative: &'static str,
    /// What the user has to do, when [`LocalReplacementKind::RequiresSetup`].
    /// Empty for every other kind.
    pub setup: &'static str,
}

/// The whole inventory, plus the counts the UI headlines with.
#[derive(Debug, Clone, Serialize)]
pub struct LocalServiceInventory {
    pub entries: Vec<LocalServiceEntry>,
    pub replaced: usize,
    pub requires_setup: usize,
    pub unavailable: usize,
    pub not_applicable: usize,
}

/// Every hosted capability the app talks to, and its local standing.
///
/// Ordered by how much a user notices it going missing, not alphabetically:
/// inference and auth first, billing last.
const ENTRIES: &[LocalServiceEntry] = &[
    LocalServiceEntry {
        id: "auth.session",
        hosted: "Hosted sign-in and the session token every client carries",
        routes: &["/auth"],
        kind: LocalReplacementKind::Replaced,
        local_alternative:
            "A device-local session issued by the local backend and stored in the OS keyring. \
             The app is single-user on this machine, so there is no account to sign in to.",
        setup: "",
    },
    LocalServiceEntry {
        id: "inference.chat",
        hosted: "Managed model routing over the backend's OpenAI-compatible proxy",
        routes: &["/openai/v1/chat/completions", "/openai/v1/models"],
        kind: LocalReplacementKind::RequiresSetup,
        local_alternative:
            "The local backend forwards /openai/v1/* to the model runtime configured under \
             [local_ai] — Ollama, LM Studio, vLLM or any OpenAI-compatible server. Model \
             routing tiers map onto the local model you choose.",
        setup: "Install a local runtime and pull a model (e.g. `ollama pull llama3.1`), or \
                point `inference_url` at your own provider key.",
    },
    LocalServiceEntry {
        id: "inference.embeddings",
        hosted: "Managed embeddings (Voyage-backed) over the backend proxy",
        routes: &["/openai/v1/embeddings"],
        kind: LocalReplacementKind::RequiresSetup,
        local_alternative:
            "The `ollama` embedding provider, selected automatically when local mode applies \
             local defaults. Memory search falls back to keyword-only if no embedding model \
             is available, so the memory tree still works.",
        setup: "Pull an embedding model (e.g. `ollama pull nomic-embed-text`).",
    },
    LocalServiceEntry {
        id: "search.web",
        hosted: "Managed web search, powered by Exa and included with the subscription",
        routes: &["/search", "/images/search", "/videos/search"],
        kind: LocalReplacementKind::RequiresSetup,
        local_alternative:
            "The `searxng` search engine: `web_search_tool` is served by your own SearXNG \
             instance. Exa's index itself is a paid third-party service and is not \
             reproduced — bring your own Exa/Brave/Querit key to use those directly.",
        setup: "Run SearXNG (`docker run -p 8080:8080 searxng/searxng`) and set \
                `[searxng] base_url`.",
    },
    LocalServiceEntry {
        id: "memory.storage",
        hosted: "Nothing — the memory tree and Obsidian mirror were always on-device",
        routes: &[],
        kind: LocalReplacementKind::Replaced,
        local_alternative:
            "Unchanged. SQLite under the workspace directory, mirrored to the Obsidian vault. \
             Local mode does not move or reshape any of it.",
        setup: "",
    },
    LocalServiceEntry {
        id: "voice.speech",
        hosted: "Hosted realtime voice and the managed transcription proxy",
        routes: &["/voice-agent", "/openai/v1/audio"],
        kind: LocalReplacementKind::RequiresSetup,
        local_alternative:
            "In-process Whisper for speech-to-text and Piper for text-to-speech, both \
             already bundled. Hosted realtime conversational voice needs a provider \
             websocket and stays off.",
        setup: "Download a Whisper model from Settings → Voice; set `PIPER_BIN` for speech \
                output.",
    },
    LocalServiceEntry {
        id: "integrations.composio",
        hosted: "100+ OAuth integrations brokered through the backend's Composio account",
        routes: &["/agent-integrations/composio"],
        kind: LocalReplacementKind::Unavailable,
        local_alternative:
            "MCP servers, which cover the same ground and run on your machine with your own \
             credentials. Composio is a third-party SaaS: its catalogue and OAuth broker \
             cannot be run locally. If you have your own Composio account, switch the \
             integration mode to `direct` and it will call Composio with your key instead \
             of ours.",
        setup: "",
    },
    LocalServiceEntry {
        id: "integrations.research",
        hosted: "Backend-proxied research and scraping vendors (Parallel, TinyFish, Apify)",
        routes: &[
            "/agent-integrations/parallel",
            "/agent-integrations/tinyfish",
            "/agent-integrations/apify",
        ],
        kind: LocalReplacementKind::Unavailable,
        local_alternative:
            "The built-in browser and fetch tools, plus any MCP server you point at these \
             vendors with your own key. Each is a metered third-party API; proxying around \
             their billing is not something local mode can or should do.",
        setup: "",
    },
    LocalServiceEntry {
        id: "media.generation",
        hosted: "Image and video generation (Seedream/SeedEdit, Seedance, Veo)",
        routes: &["/agent-integrations/media-generation"],
        kind: LocalReplacementKind::Unavailable,
        local_alternative:
            "A local OpenAI-compatible image endpoint (ComfyUI, Automatic1111, or any server \
             exposing /v1/images/generations) configured under [media]. The hosted video \
             models are proprietary weights with no local equivalent.",
        setup: "",
    },
    LocalServiceEntry {
        id: "channels.messaging",
        hosted: "Backend relay for the messaging channels",
        routes: &["/channels"],
        kind: LocalReplacementKind::RequiresSetup,
        local_alternative:
            "Direct connections. Telegram, Discord, Slack, IRC, Matrix and email (IMAP/SMTP) \
             all speak to their own servers from your machine with your own bot token — the \
             relay was a convenience, never a requirement.",
        setup: "Paste each channel's bot token in Settings → Channels.",
    },
    LocalServiceEntry {
        id: "telemetry.tracing",
        hosted: "Langfuse trace ingestion proxied through the backend",
        routes: &["/telemetry"],
        kind: LocalReplacementKind::Replaced,
        local_alternative:
            "Off by default in local mode. Run counts and per-call costs are still recorded \
             in the local run store and replayable in the UI; point agent tracing at your \
             own Langfuse if you want traces off-device.",
        setup: "",
    },
    LocalServiceEntry {
        id: "sync.realtime",
        hosted: "Socket.IO push from the backend (cross-device events, hosted notifications)",
        routes: &["/socket.io"],
        kind: LocalReplacementKind::Replaced,
        local_alternative:
            "The core's own event bus over the loopback Socket.IO and SSE transports, which \
             is what the desktop UI already consumes. Cross-*device* sync has no local \
             equivalent — there is no server in the middle.",
        setup: "",
    },
    LocalServiceEntry {
        id: "account.usage",
        hosted: "Team membership and metered usage reporting",
        routes: &["/teams"],
        kind: LocalReplacementKind::Replaced,
        local_alternative:
            "A single-member local team backed by the on-device cost ledger, so per-model \
             spend still shows up for your own provider keys.",
        setup: "",
    },
    LocalServiceEntry {
        id: "account.billing",
        hosted: "Subscription, credits, top-ups and auto-recharge (Stripe/Coinbase)",
        routes: &["/payments"],
        kind: LocalReplacementKind::NotApplicable,
        local_alternative:
            "Nothing to bill. You pay your model provider directly, or nothing at all when \
             everything runs on local weights.",
        setup: "",
    },
    LocalServiceEntry {
        id: "account.growth",
        hosted: "Referrals, invites, rewards and product announcements",
        routes: &["/referral", "/invites", "/announcements"],
        kind: LocalReplacementKind::NotApplicable,
        local_alternative: "Absent — all four are properties of the hosted account system.",
        setup: "",
    },
    LocalServiceEntry {
        id: "agents.marketplace",
        hosted: "tiny.place handles, agent-to-agent orchestration and x402 bounties",
        routes: &["/tinyplace"],
        kind: LocalReplacementKind::NotApplicable,
        local_alternative:
            "Absent. A handle is an identity on someone else's network; there is no local \
             form of it. Local subagents and fleets are unaffected.",
        setup: "",
    },
];

/// The inventory with its counts computed.
pub fn service_inventory() -> LocalServiceInventory {
    let entries = ENTRIES.to_vec();
    let count = |kind: LocalReplacementKind| entries.iter().filter(|e| e.kind == kind).count();

    LocalServiceInventory {
        replaced: count(LocalReplacementKind::Replaced),
        requires_setup: count(LocalReplacementKind::RequiresSetup),
        unavailable: count(LocalReplacementKind::Unavailable),
        not_applicable: count(LocalReplacementKind::NotApplicable),
        entries,
    }
}

/// The entry owning `path`, matched on the longest declared route prefix.
///
/// Longest-prefix matching matters because the route families nest:
/// `/agent-integrations/composio/execute` must resolve to `integrations.composio`
/// and not to a shorter `/agent-integrations` entry that happens to be declared
/// first. Returns `None` for a path no entry claims — the caller then answers
/// with the generic unsupported error rather than a misleading alternative.
pub fn entry_for_path(path: &str) -> Option<&'static LocalServiceEntry> {
    let path = path.trim();
    ENTRIES
        .iter()
        .flat_map(|entry| {
            entry
                .routes
                .iter()
                .filter(|route| path_matches_route(path, route))
                .map(move |route| (route.len(), entry))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, entry)| entry)
}

/// Does `path` fall under `route`?
///
/// A match requires a real segment boundary: `/teams` matches `/teams` and
/// `/teams/me/usage` but not `/teamsomething`. Without the boundary check a
/// future `/search-history` route would inherit `/search`'s alternative and
/// tell the user to install SearXNG.
fn path_matches_route(path: &str, route: &str) -> bool {
    match path.strip_prefix(route) {
        Some(rest) => rest.is_empty() || rest.starts_with('/') || rest.starts_with('?'),
        None => false,
    }
}

#[cfg(test)]
#[path = "services_tests.rs"]
mod tests;
