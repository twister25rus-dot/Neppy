//! Native turn-model source coverage.

use std::sync::Arc;

use super::*;

#[test]
fn crate_native_turn_source_retains_only_role_and_config() {
    let source = TurnModelSource::new_crate_native(
        "chat",
        Arc::new(crate::neppy::config::Config::default()),
    );

    assert!(source.direct_model.is_none());
    assert!(source.crate_native.is_some());
}

#[test]
fn crate_native_text_mode_is_recorded_without_resolving_a_model() {
    let source = TurnModelSource::new_crate_native(
        "chat",
        Arc::new(crate::neppy::config::Config::default()),
    )
    .with_text_mode();

    assert!(source
        .crate_native
        .as_ref()
        .is_some_and(|native| native.force_text_mode));
}

#[test]
fn crate_native_text_mode_disables_native_tools_on_workload_fallbacks() {
    use crate::neppy::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let _guard = crate::neppy::inference::inference_test_guard();
    let provider = "deepseek:deepseek-chat".to_string();
    let mut config = crate::neppy::config::Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_deepseek".to_string(),
        slug: "deepseek".to_string(),
        label: "DeepSeek".to_string(),
        endpoint: "https://api.deepseek.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("deepseek-chat".to_string()),
        ..Default::default()
    });
    config.chat_provider = Some(provider.clone());
    config.reasoning_provider = Some(provider.clone());
    config.agentic_provider = Some(provider.clone());
    config.coding_provider = Some(provider.clone());
    config.vision_provider = Some(provider.clone());
    config.memory_provider = Some(provider);

    let models = TurnModelSource::new_crate_native("chat", Arc::new(config))
        .with_text_mode()
        .build("chat-v1", 0.0, Some(32_000))
        .expect("text-mode turn models build");

    assert!(
        !models.routes.is_empty(),
        "expected workload fallback models"
    );
    assert!(
        models
            .routes
            .iter()
            .all(|(_, model)| { model.profile().is_some_and(|profile| !profile.tool_calling) }),
        "every workload fallback must preserve prompt-guided text mode"
    );
}

#[test]
fn direct_model_turn_source_builds_without_provider_adapter() {
    let model: Arc<dyn tinyagents::harness::model::ChatModel<()>> =
        Arc::new(tinyagents::harness::testkit::ScriptedModel::replies(vec![
            "done",
        ]));
    let source = TurnModelSource::from_model(model);

    assert!(source.crate_native.is_none());
    assert!(source.direct_model.is_some());

    let models = source
        .build("mock-model", 0.0, Some(32_000))
        .expect("direct model source builds");
    assert_eq!(models.provider_id(), "injected");
    assert_eq!(models.context_window(), Some(32_000));
    assert!(!models.native_tools());
}

#[test]
fn run_policy_for_makes_invalid_tool_arguments_recoverable() {
    let policy = run_policy_for(10, false);
    assert_eq!(
        policy.invalid_args,
        InvalidArgsPolicy::ReturnToolError,
        "schema-invalid calls must return a corrective tool result instead of aborting the turn"
    );
}

/// N9: `reliability.model_fallbacks` for a BYOK primary is built into the
/// turn's routes and recorded as the configured chain — it used to be ignored
/// on the chat path entirely.
#[test]
fn byok_turn_builds_user_configured_fallback_models() {
    use crate::neppy::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let _guard = crate::neppy::inference::inference_test_guard();
    let mut config = crate::neppy::config::Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_deepseek".to_string(),
        slug: "deepseek".to_string(),
        label: "DeepSeek".to_string(),
        endpoint: "https://api.deepseek.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("deepseek-chat".to_string()),
        ..Default::default()
    });
    config.chat_provider = Some("deepseek:deepseek-chat".to_string());
    config.reliability.model_fallbacks.insert(
        "deepseek-chat".to_string(),
        vec!["deepseek-reasoner".to_string()],
    );

    let models = TurnModelSource::new_crate_native("chat", Arc::new(config))
        .build("deepseek-chat", 0.0, Some(32_000))
        .expect("turn models build");

    assert_eq!(
        models.configured_fallbacks,
        vec!["deepseek-reasoner".to_string()]
    );
    assert!(
        models
            .routes
            .iter()
            .any(|(name, _)| name == "deepseek-reasoner"),
        "the configured fallback must be registered as a route"
    );
    let chain = routes::turn_fallback_policy("deepseek-chat", &models.configured_fallbacks)
        .expect("a configured fallback yields a chain")
        .models;
    assert_eq!(chain, vec!["deepseek-chat", "deepseek-reasoner"]);
}

/// A primary that always fails with a permanent (non-retryable) error, standing
/// in for a BYOK model the provider rejects.
struct AlwaysFailingModel;

#[async_trait::async_trait]
impl tinyagents::harness::model::ChatModel<()> for AlwaysFailingModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: tinyagents::harness::model::ModelRequest,
    ) -> tinyagents::Result<tinyagents::harness::model::ModelResponse> {
        Err(tinyagents::TinyAgentsError::Validation(
            "byok-primary: model not found".to_string(),
        ))
    }
}

/// N9 end to end: a failing primary falls back to the user-configured model and
/// the turn answers from it instead of erroring.
#[tokio::test]
async fn failing_primary_falls_back_to_the_configured_model() {
    let primary: TurnChatModel = Arc::new(AlwaysFailingModel);
    let fallback: TurnChatModel =
        Arc::new(tinyagents::harness::testkit::ScriptedModel::replies(vec![
            "answered by the fallback",
        ]));
    let turn_models = TurnModels {
        primary: primary.clone(),
        routes: vec![("byok-fallback".to_string(), fallback)],
        configured_fallbacks: vec!["byok-fallback".to_string()],
        summarizer: primary,
        error_slot: Arc::new(std::sync::Mutex::new(None)),
        provider_id: "injected".to_string(),
        context_window: Some(32_000),
        native_tools: false,
        supports_vision: false,
    };

    let outcome = run_turn_via_tinyagents_shared(
        turn_models,
        "injected".to_string(),
        "byok-primary",
        vec![ChatMessage::user("hello")],
        vec![],
        None,
        4,
        None,
        None,
        Some(32_000),
        None,
        &[],
        false,
        None,
        TurnContextMiddleware::defaults(),
        None,
        None,
        false,
        false,
    )
    .await
    .expect("the configured fallback must answer the turn");

    assert_eq!(outcome.text, "answered by the fallback");
}

/// Without a configured fallback the same failing primary still fails — the
/// chain is only what the user configured (no invented fallback).
#[tokio::test]
async fn failing_primary_without_configured_fallback_still_errors() {
    let primary: TurnChatModel = Arc::new(AlwaysFailingModel);
    let turn_models = TurnModels {
        primary: primary.clone(),
        routes: Vec::new(),
        configured_fallbacks: Vec::new(),
        summarizer: primary,
        error_slot: Arc::new(std::sync::Mutex::new(None)),
        provider_id: "injected".to_string(),
        context_window: Some(32_000),
        native_tools: false,
        supports_vision: false,
    };
    let result = run_turn_via_tinyagents_shared(
        turn_models,
        "injected".to_string(),
        "byok-primary",
        vec![ChatMessage::user("hello")],
        vec![],
        None,
        4,
        None,
        None,
        Some(32_000),
        None,
        &[],
        false,
        None,
        TurnContextMiddleware::defaults(),
        None,
        None,
        false,
        false,
    )
    .await;
    assert!(
        result.is_err(),
        "no fallback configured → the failure surfaces"
    );
}
