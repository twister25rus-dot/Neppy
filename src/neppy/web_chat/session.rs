use crate::neppy::agent::profiles::{AgentProfile, DEFAULT_PROFILE_ID};
use crate::neppy::agent::Agent;
use crate::neppy::config::Config;
use crate::neppy::inference::turn_controls::TurnModelControls;
use serde_json::json;

use super::types::SessionCacheFingerprint;

pub(super) fn autonomy_signature(config: &Config) -> String {
    serde_json::to_string(&config.autonomy).unwrap_or_default()
}

/// Signature of `config.model_registry` for the session-cache fingerprint.
/// Captures every per-model `vision` flag so toggling one in Settings forces a
/// rebuild (picking up the new build-time `model_vision`). Mirrors
/// [`autonomy_signature`].
pub(super) fn model_registry_signature(config: &Config) -> String {
    serde_json::to_string(&config.model_registry).unwrap_or_default()
}

pub(super) fn pick_target_agent_id(_config: &Config, profile: &AgentProfile) -> String {
    if profile.id == DEFAULT_PROFILE_ID {
        "orchestrator".to_string()
    } else {
        profile.agent_id.clone()
    }
}

pub(crate) fn normalize_model_override(model_override: Option<String>) -> Option<String> {
    model_override
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

pub(crate) fn provider_role_for_model_override(model_override: Option<&str>) -> &'static str {
    match model_override.map(str::trim) {
        Some("hint:agentic") | Some("agentic-v1") => "agentic",
        Some("hint:coding") | Some("coding-v1") => "coding",
        Some("hint:summarization") | Some("summarization-v1") => "summarization",
        Some("hint:reasoning") | Some("reasoning-v1") => "reasoning",
        _ => "chat",
    }
}

/// The provider route a composer model pick names, if it names one.
///
/// The picker encodes a choice as `<slug>:<model>`, and for anything that is
/// not the managed backend the slug is the only statement of *where* the turn
/// should run. `None` means "leave the role's route alone": a managed tier
/// (`chat-v1`), a `hint:`, a bare model id, or a slug this install has never
/// heard of — the last of those matters because a model id may itself contain a
/// colon (`qwen3:8b`), and reading `qwen3` as a provider would route the turn
/// at a runtime that does not exist.
pub(super) fn route_for_model_pick(pick: &str, config: &Config) -> Option<String> {
    use crate::neppy::inference::local::profile::LocalProviderKind;

    let pick = pick.trim();
    if pick.is_empty()
        || pick.starts_with("hint:")
        || crate::neppy::inference::provider::factory::is_known_neppy_tier(pick)
    {
        return None;
    }
    let (slug, model) = pick.split_once(':')?;
    if model.trim().is_empty() {
        return None;
    }
    let known = LocalProviderKind::from_str_loose(slug).is_some()
        || slug == "claude-code"
        || config
            .cloud_providers
            .iter()
            .any(|entry| entry.slug == slug);
    known.then(|| pick.to_string())
}

/// The config a turn runs under once the composer's model pick is applied.
///
/// A pick like `mlx:<id>` names a provider as well as a model, and on a local
/// runtime the *route* decides the model — the provider string carries its own
/// and `default_model` is never consulted. Setting the model alone therefore
/// left the turn on whatever the role already pointed at, so changing models in
/// the composer changed nothing at all. The pick sets both.
///
/// A managed tier or a `hint:` sets only `default_model`, exactly as before:
/// there the model *is* the tier and the backend resolves it.
pub(super) fn config_with_model_pick(config: &Config, pick: Option<String>) -> Config {
    let mut effective = config.clone();
    let Some(model) = pick else {
        return effective;
    };
    if let Some(route) = route_for_model_pick(&model, &effective) {
        log::debug!(
            "[web-chat] model pick '{}' names provider '{}' — routing the chat turn there",
            model,
            route.split(':').next().unwrap_or("<unknown>")
        );
        effective.chat_provider = Some(route);
    }
    effective.default_model = Some(model);
    effective
}

pub(super) fn build_session_agent(
    config: &Config,
    client_id: &str,
    thread_id: &str,
    target_agent_id: &str,
    profile: &AgentProfile,
    model_override: Option<String>,
    temperature: Option<f64>,
    controls: TurnModelControls,
    locale: Option<&str>,
) -> Result<Agent, String> {
    let mut effective = config_with_model_pick(config, model_override);
    let provider_role = provider_role_for_model_override(effective.default_model.as_deref());
    effective.turn_controls = (!controls.is_empty()).then_some(controls);
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    log::info!(
        "[web-channel] routing chat turn to '{}' via profile '{}' provider_role='{}' (client_id={}, thread_id={})",
        target_agent_id,
        profile.id,
        provider_role,
        client_id,
        thread_id
    );

    let locale_directive = locale.and_then(locale_reply_directive);
    let composed_suffix = compose_system_prompt_suffix(
        locale_directive.as_deref(),
        profile.system_prompt_suffix.as_deref(),
    );
    if let Some(s) = locale_directive.as_deref() {
        log::info!(
            "[web-channel] injecting locale directive client={} thread={} locale={} directive={:?}",
            client_id,
            thread_id,
            locale.unwrap_or(""),
            s
        );
    }

    let agent_result = Agent::from_config_for_agent_with_profile(
        &effective,
        target_agent_id,
        composed_suffix,
        Some(profile),
    );

    agent_result
        .map(|mut agent| {
            agent.set_event_context(
                json!({"client_id": client_id, "thread_id": thread_id}).to_string(),
                "web_channel",
            );
            let short_thread = if thread_id.len() > 12 {
                &thread_id[..12]
            } else {
                thread_id
            };
            agent.set_agent_definition_name(format!("{target_agent_id}_{short_thread}"));
            agent
        })
        .map_err(|e| e.to_string())
}

pub(crate) fn locale_reply_directive(locale: &str) -> Option<String> {
    let language = match locale.trim() {
        "ar" => "Arabic",
        "bn" => "Bengali",
        "es" => "Spanish",
        "fr" => "French",
        "hi" => "Hindi",
        "id" => "Indonesian",
        "it" => "Italian",
        "pt" => "Portuguese",
        "ru" => "Russian",
        "zh-CN" | "zh" => "Simplified Chinese",
        _ => return None,
    };
    Some(format!(
        "User language: the user's interface is set to {language}. \
         Respond in {language} unless the user explicitly asks for a different language. \
         Keep proper nouns, code, and command names untranslated."
    ))
}

pub(crate) fn compose_system_prompt_suffix(
    locale_directive: Option<&str>,
    profile_suffix: Option<&str>,
) -> Option<String> {
    match (locale_directive, profile_suffix) {
        (None, None) => None,
        (Some(d), None) => Some(d.to_string()),
        (None, Some(p)) => Some(p.to_string()),
        (Some(d), Some(p)) => Some(format!("{d}\n\n{p}")),
    }
}

pub(super) fn build_session_fingerprint(
    config: &Config,
    model_override: Option<String>,
    temperature: Option<f64>,
    controls: TurnModelControls,
    target_agent_id: String,
    provider_role: &str,
    profile: &AgentProfile,
) -> SessionCacheFingerprint {
    SessionCacheFingerprint {
        controls,
        model_override,
        temperature,
        provider_binding: crate::neppy::inference::provider::provider_for_role(
            provider_role,
            config,
        ),
        target_agent_id,
        autonomy_signature: autonomy_signature(config),
        model_registry_signature: model_registry_signature(config),
        // Any change to the resolved profile record or its canonical on-disk
        // SOUL/MEMORY files forces a session-agent rebuild — see the field doc.
        profile_signature: crate::neppy::agent::profiles::profile_session_signature(
            &config.workspace_dir,
            profile,
        ),
    }
}
