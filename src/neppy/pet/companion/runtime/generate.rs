//! Suggestion text generation: the only place the companion reaches a model.
//!
//! The runtime decides WHETHER to call (a rule trigger that passed usefulness
//! and the rate limiter, or an ask / capture / click), builds the prompt from
//! scrubbed text only ([`build_prompt`], which also runs the prompt-injection
//! gate), and picks the provider with [`choose_provider`]. The [`Generator`]
//! seam then runs one zero-tool `pet_companion` turn under the `PetCompanion`
//! origin (production: [`BusGenerator`]; tests: a fake).
//!
//! Provider choice (user decision D3): the cloud chat model by default
//! (`allow_cloud_model = true`) receiving scrubbed text only; when the user
//! switches that off, a configured local model; with neither, no body (the
//! suggestion offers "open in chat" so the user sends it themselves). A chat
//! provider that is itself local is always fine.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::neppy::config::Config;
use crate::neppy::pet::companion::sensitive::{scrub, ScrubCtx};
use crate::neppy::pet::companion::types::{ActionCategory, TriggerKind};
use crate::neppy::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};

/// Generated body cap (chars), after scrubbing.
pub const MAX_BODY_CHARS: usize = 1500;
/// Excerpt cap handed to the model.
pub const MAX_PROMPT_EXCERPT: usize = 1500;
pub const GENERATION_TIMEOUT: Duration = Duration::from_secs(60);
/// The agent's "nothing to add" sentinel; treated as no body.
pub const NOTHING_TO_ADD: &str = "Nothing useful to add.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenProvider {
    /// The user's chat model (cloud unless their chat provider is local).
    Chat,
    /// A configured local model (MLX / Ollama / LM Studio).
    Local,
    /// No model may be used: no body.
    None,
}

impl GenProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Local => "local",
            Self::None => "none",
        }
    }
}

/// What the configured providers are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCaps {
    /// The chat provider runs on this machine.
    pub chat_is_local: bool,
    /// A local model is configured and enabled.
    pub local_available: bool,
}

/// Pure provider choice (test-pinned, PC11 / D3).
pub fn choose_provider(allow_cloud_model: bool, caps: ProviderCaps) -> GenProvider {
    if caps.chat_is_local || allow_cloud_model {
        GenProvider::Chat
    } else if caps.local_available {
        GenProvider::Local
    } else {
        GenProvider::None
    }
}

/// What to write.
#[derive(Debug, Clone)]
pub struct GenRequest {
    pub kind: TriggerKind,
    pub category: ActionCategory,
    pub app_name: String,
    /// Scrubbed title excerpt.
    pub title_excerpt: String,
    /// Scrubbed context excerpt.
    pub excerpt: String,
}

fn task_line(req: &GenRequest) -> &'static str {
    match (req.category, req.kind) {
        (ActionCategory::DraftText, _) | (_, TriggerKind::EmailDraft) => {
            "Write a short draft reply or a polished version of the observed text."
        }
        (_, TriggerKind::BuildError) => {
            "Explain the likely cause of this error and give up to three fix steps."
        }
        (_, TriggerKind::Term) => "Define the selected term briefly and say why it matters here.",
        _ => "Offer one short, useful suggestion about what the user is looking at.",
    }
}

/// Build the user message. Every field is re-scrubbed (idempotent) and the
/// observed part is wrapped as untrusted data. Returns `None` when the
/// prompt-injection gate refuses it: the caller then keeps the template
/// suggestion and makes no model call.
pub fn build_prompt(req: &GenRequest) -> Option<String> {
    let clean = |s: &str, cap: usize| {
        scrub(s, &ScrubCtx::generated().capped(cap))
            .into_text()
            .map(|t| t.into_string())
            .unwrap_or_default()
    };
    let excerpt = clean(&req.excerpt, MAX_PROMPT_EXCERPT);
    let title = clean(&req.title_excerpt, 200);
    let app = clean(&req.app_name, 80);
    let observed = format!("App: {app}\nWindow: {title}\n{excerpt}");
    let decision = enforce_prompt_input(
        &observed,
        PromptEnforcementContext {
            source: "pet.companion",
            request_id: None,
            user_id: None,
            session_id: None,
        },
    );
    if decision.action != PromptEnforcementAction::Allow {
        log::info!("[pet::companion] prompt refused by injection gate; template only");
        return None;
    }
    Some(format!(
        "{}\n\n<observed untrusted=\"true\">\n{}\n</observed>",
        task_line(req),
        observed.replace("</observed>", "")
    ))
}

/// Clean model output for storage and display: scrubbed, capped, plain text.
/// `None` for an empty reply or the "nothing to add" sentinel.
pub fn sanitize_output(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() || t.starts_with(NOTHING_TO_ADD) {
        return None;
    }
    let plain: String = t.replace(['<', '>'], "");
    scrub(&plain, &ScrubCtx::generated().capped(MAX_BODY_CHARS))
        .into_text()
        .map(|s| s.into_string())
        .filter(|s| !s.trim().is_empty())
}

/// The model seam.
#[async_trait]
pub trait Generator: Send + Sync {
    fn caps(&self, config: &Config) -> ProviderCaps;
    /// Run one zero-tool turn. `job_id` is system-generated, never content.
    async fn run(
        &self,
        config: &Config,
        provider: GenProvider,
        job_id: &str,
        prompt: &str,
    ) -> Result<String, String>;
}

/// Production generator: one `pet_companion` turn over the `agent.run_turn` bus.
pub struct BusGenerator;

fn chat_provider_string(config: &Config) -> String {
    crate::neppy::inference::provider::factory::provider_for_role("chat", config)
}

struct Resolved {
    source: crate::neppy::agent::tinyagents::TurnModelSource,
    provider_name: String,
    model: String,
}

fn resolve(config: &Config, provider: GenProvider) -> Result<Resolved, String> {
    match provider {
        GenProvider::Chat => {
            let s = chat_provider_string(config);
            let (_m, model) =
                crate::neppy::inference::provider::factory::create_chat_model_from_string_with_model_id(
                    "chat",
                    &s,
                    config,
                    config.default_temperature,
                )
                .map_err(|e| format!("chat provider unavailable: {e:#}"))?;
            Ok(Resolved {
                source:
                    crate::neppy::agent::tinyagents::TurnModelSource::new_crate_native_from_string(
                        "chat",
                        &s,
                        Arc::new(config.clone()),
                    ),
                provider_name: s.split(':').next().unwrap_or("chat").to_string(),
                model,
            })
        }
        GenProvider::Local => {
            let r = crate::neppy::agent::triage::routing::build_local_provider_with_config(config)
                .ok_or("local model unavailable")?;
            Ok(Resolved {
                source: r.turn_model_source,
                provider_name: r.provider_name,
                model: r.model,
            })
        }
        GenProvider::None => Err("no model allowed".into()),
    }
}

fn system_prompt(config: &Config, model: &str) -> Result<String, String> {
    use crate::neppy::agent::context::prompt::{LearnedContextData, PromptContext, ToolCallFormat};
    let visible = std::collections::HashSet::new();
    let ctx = PromptContext {
        workspace_dir: &config.workspace_dir,
        model_name: model,
        agent_id: crate::neppy::pet::lane::PET_COMPANION_AGENT_ID,
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names: &visible,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
    };
    crate::neppy::agent::registry::agents::pet_companion::prompt::build(&ctx)
        .map_err(|e| format!("companion prompt: {e:#}"))
}

#[async_trait]
impl Generator for BusGenerator {
    fn caps(&self, config: &Config) -> ProviderCaps {
        let chat = chat_provider_string(config);
        ProviderCaps {
            chat_is_local: crate::neppy::inference::local::profile::is_local_provider_string(&chat),
            local_available: config.local_ai.runtime_enabled
                && !config.local_ai.chat_model_id.trim().is_empty(),
        }
    }

    async fn run(
        &self,
        config: &Config,
        provider: GenProvider,
        job_id: &str,
        prompt: &str,
    ) -> Result<String, String> {
        use crate::core::bus::BUS;
        use crate::neppy::agent::bus::{
            AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD,
        };
        use crate::neppy::agent::messages::ChatMessage;

        let r = resolve(config, provider)?;
        let system = system_prompt(config, &r.model)?;
        log::debug!(
            "[pet::companion] generation start provider={} model={}",
            provider.as_str(),
            r.model
        );
        let request = AgentTurnRequest {
            turn_model_source: r.source,
            history: vec![ChatMessage::system(&system), ChatMessage::user(prompt)],
            tools_registry: Arc::new(Vec::new()),
            provider_name: r.provider_name,
            model: r.model,
            temperature: 0.3,
            silent: true,
            channel_name: "pet_companion".to_string(),
            multimodal: crate::neppy::config::MultimodalConfig::default(),
            // Observed text is untrusted: no file-marker resolution at all.
            multimodal_files:
                crate::neppy::config::MultimodalFileConfig::for_untrusted_channel_input(),
            max_tool_iterations: 1,
            on_delta: None,
            target_agent_id: Some(crate::neppy::pet::lane::PET_COMPANION_AGENT_ID.to_string()),
            visible_tool_names: None,
            extra_tools: Vec::new(),
            on_progress: None,
            origin: crate::neppy::agent::turn_origin::pet_companion_origin(job_id, None),
        };
        let fut = BUS
            .native()
            .request::<AgentTurnRequest, AgentTurnResponse>(AGENT_RUN_TURN_METHOD, request);
        match tokio::time::timeout(GENERATION_TIMEOUT, fut).await {
            Ok(Ok(resp)) => Ok(resp.text),
            Ok(Err(e)) => Err(format!("agent turn failed: {e}")),
            Err(_) => Err("generation timed out".into()),
        }
    }
}
