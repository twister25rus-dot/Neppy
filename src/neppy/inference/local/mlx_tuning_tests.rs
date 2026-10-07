//! Tests for MLX request defaults, text-tool recovery in the model wrapper, and
//! the wire shape (MLX gets the defaults; cloud providers do not).

use std::sync::{Arc, Mutex};

use futures::StreamExt;
use serde_json::{json, Value};
use tinyagents::harness::message::Message;
use tinyagents::harness::tool::ToolSchema;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::neppy::config::schema::mlx::KIND_LM;

fn block(f: impl FnOnce(&mut MlxServerConfig)) -> MlxServerConfig {
    let mut server = MlxServerConfig::default();
    f(&mut server);
    server
}

// ── Defaults from the server block ───────────────────────────────────────

#[test]
fn unset_sentinels_become_the_qwen_defaults() {
    let d = MlxRequestDefaults::for_block(Some(&MlxServerConfig::default()));
    assert_eq!(d.temperature, DEFAULT_TEMPERATURE);
    assert_eq!(d.top_p, DEFAULT_TOP_P);
    assert_eq!(d.top_k, DEFAULT_TOP_K);
    assert_eq!(d.min_p, None, "min_p has no default to impose");
    assert_eq!(d.max_tokens, DEFAULT_MAX_TOKENS);
    assert_eq!(d.thinking_budget, Some(AUTO_THINKING_BUDGET));
}

#[test]
fn explicit_block_values_are_never_overridden() {
    let d = MlxRequestDefaults::for_block(Some(&block(|s| {
        s.temp = 0.2;
        s.top_p = 0.8;
        s.top_k = 0;
        s.min_p = 0.05;
        s.max_tokens = 300;
        s.thinking_budget = 1000;
    })));
    assert!((d.temperature - 0.2).abs() < 1e-6);
    assert!((d.top_p - 0.8).abs() < 1e-6);
    assert_eq!(
        d.top_k, 0,
        "an explicit 0 (top-k off) is a value, not a sentinel"
    );
    assert!((d.min_p.unwrap() - 0.05).abs() < 1e-6);
    assert_eq!(d.max_tokens, 300);
    assert_eq!(d.thinking_budget, Some(1000));
}

#[test]
fn a_large_thinking_budget_raises_the_default_output_cap() {
    let d = MlxRequestDefaults::for_block(Some(&block(|s| s.thinking_budget = 12_000)));
    assert_eq!(d.max_tokens, 12_000 + ANSWER_HEADROOM_TOKENS);
}

#[test]
fn the_budget_is_withheld_where_the_server_cannot_take_it() {
    let lm = block(|s| s.kind = KIND_LM.to_string());
    assert_eq!(
        MlxRequestDefaults::for_block(Some(&lm)).thinking_budget,
        None
    );
    let speculative = block(|s| s.draft_model = "some/drafter".to_string());
    assert_eq!(
        MlxRequestDefaults::for_block(Some(&speculative)).thinking_budget,
        None,
        "mlx_vlm.server rejects thinking_budget with speculative decoding"
    );
}

#[test]
fn a_missing_block_behaves_like_a_default_vlm_block() {
    assert_eq!(
        MlxRequestDefaults::for_block(None),
        MlxRequestDefaults::for_block(Some(&MlxServerConfig::default()))
    );
}

#[test]
fn the_block_is_chosen_by_model_slot_then_port() {
    let mut config = Config::default();
    config.mlx.servers = vec![
        block(|s| {
            s.id = "a".into();
            s.model = "org/a".into();
            s.temp = 0.1;
        }),
        block(|s| {
            s.id = "b".into();
            s.model = "org/b".into();
            s.port = 9001;
            s.temp = 0.9;
        }),
    ];
    assert_eq!(server_for_model(&config, "org/a").unwrap().id, "a");
    assert_eq!(server_for_model(&config, "org/b").unwrap().id, "b");
    assert_eq!(
        server_for_model(&config, "org/other").unwrap().id,
        "b",
        "falls back to the block the endpoint resolver would dial"
    );
    assert!((turn_default_temperature(&config, "org/a") - 0.1).abs() < 1e-6);
    assert_eq!(
        turn_default_temperature(&Config::default(), "x"),
        DEFAULT_TEMPERATURE
    );
}

// ── apply(): fill, never override ────────────────────────────────────────

#[test]
fn apply_fills_an_empty_request() {
    let mut request = ModelRequest::default();
    MlxRequestDefaults::for_block(None).apply(&mut request);
    assert_eq!(request.temperature, Some(0.6));
    assert_eq!(request.top_p, Some(0.95));
    assert_eq!(request.max_tokens, Some(8192));
    assert_eq!(request.provider_options["top_k"], json!(20));
    assert_eq!(request.provider_options["thinking_budget"], json!(4096));
    assert!(request.provider_options.get("min_p").is_none());
}

#[test]
fn apply_never_overrides_what_the_request_already_carries() {
    let mut request = ModelRequest {
        temperature: Some(0.0),
        top_p: Some(0.5),
        max_tokens: Some(16_384),
        provider_options: json!({"top_k": 3, "thinking_budget": 64, "other": true}),
        ..ModelRequest::default()
    };
    MlxRequestDefaults::for_block(None).apply(&mut request);
    assert_eq!(request.temperature, Some(0.0));
    assert_eq!(request.top_p, Some(0.5));
    assert_eq!(request.max_tokens, Some(16_384));
    assert_eq!(
        request.provider_options,
        json!({"top_k": 3, "thinking_budget": 64, "other": true})
    );
}

#[test]
fn a_turn_that_turned_thinking_off_or_asked_for_high_gets_no_budget() {
    for options in [
        json!({"enable_thinking": false}),
        json!({"enable_thinking": true, "reasoning_effort": "high"}),
    ] {
        let mut request = ModelRequest {
            provider_options: options.clone(),
            ..ModelRequest::default()
        };
        MlxRequestDefaults::for_block(None).apply(&mut request);
        assert!(
            request.provider_options.get("thinking_budget").is_none(),
            "{options}: a ceiling would contradict the ask"
        );
        assert_eq!(request.provider_options["top_k"], json!(20));
    }
}

// ── The wrapper: text-form tool recovery ─────────────────────────────────

struct Scripted {
    seen: Mutex<Vec<ModelRequest>>,
    response: ModelResponse,
    stream: Vec<ModelStreamItem>,
}

impl Scripted {
    fn replying(text: &str) -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            response: ModelResponse::assistant(text),
            stream: Vec::new(),
        })
    }

    fn streaming(fragments: &[&str]) -> Arc<Self> {
        let full: String = fragments.concat();
        let mut items = vec![ModelStreamItem::Started];
        items.extend(
            fragments
                .iter()
                .map(|f| ModelStreamItem::MessageDelta(MessageDelta::text(*f))),
        );
        items.push(ModelStreamItem::Completed(ModelResponse::assistant(
            full.clone(),
        )));
        Arc::new(Self {
            seen: Mutex::new(Vec::new()),
            response: ModelResponse::assistant(full),
            stream: items,
        })
    }
}

#[async_trait]
impl ChatModel<()> for Scripted {
    async fn invoke(&self, _s: &(), request: ModelRequest) -> tinyagents::Result<ModelResponse> {
        self.seen.lock().unwrap().push(request);
        Ok(self.response.clone())
    }

    async fn stream(&self, _s: &(), request: ModelRequest) -> tinyagents::Result<ModelStream> {
        self.seen.lock().unwrap().push(request);
        Ok(Box::pin(futures::stream::iter(self.stream.clone())))
    }
}

fn shell_tool() -> ToolSchema {
    ToolSchema::new(
        "shell",
        "run a command",
        json!({"type":"object","properties":{"command":{"type":"string"}}}),
    )
}

fn request_with_tools() -> ModelRequest {
    ModelRequest {
        messages: vec![Message::user("hi")],
        tools: vec![shell_tool()],
        ..ModelRequest::default()
    }
}

fn tuned(inner: Arc<Scripted>) -> MlxTunedModel {
    MlxTunedModel {
        inner,
        defaults: MlxRequestDefaults::for_block(None),
    }
}

const QWEN_CALL: &str = "<function=shell>\n<parameter=command>\nls -la\n</parameter>\n</function>";

#[tokio::test]
async fn invoke_applies_the_defaults_to_the_inner_request() {
    let inner = Scripted::replying("hello");
    let model = tuned(inner.clone());
    model.invoke(&(), request_with_tools()).await.unwrap();
    let seen = inner.seen.lock().unwrap();
    assert_eq!(seen[0].temperature, Some(0.6));
    assert_eq!(seen[0].provider_options["top_k"], json!(20));
}

#[tokio::test]
async fn invoke_turns_a_text_form_call_into_a_tool_call() {
    let inner = Scripted::replying(&format!("Running it.\n{QWEN_CALL}"));
    let response = tuned(inner)
        .invoke(&(), request_with_tools())
        .await
        .unwrap();
    assert_eq!(response.message.tool_calls.len(), 1);
    let call = &response.message.tool_calls[0];
    assert_eq!(call.name, "shell");
    assert_eq!(call.arguments, json!({"command": "ls -la"}));
    assert!(call.id.starts_with("ptc_"), "synthetic id, got {}", call.id);
    assert_eq!(response.text(), "Running it.");
    assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
}

#[tokio::test]
async fn invoke_leaves_prose_about_the_tag_alone() {
    let prose = "To call a tool, write <tool_call> then a JSON body, or <function=NAME>.";
    let response = tuned(Scripted::replying(prose))
        .invoke(&(), request_with_tools())
        .await
        .unwrap();
    assert!(response.message.tool_calls.is_empty());
    assert_eq!(response.text(), prose);
}

#[tokio::test]
async fn invoke_ignores_a_call_to_a_tool_that_was_not_advertised() {
    let text = "<function=rm_rf><parameter=path>\n/\n</parameter></function>";
    let response = tuned(Scripted::replying(text))
        .invoke(&(), request_with_tools())
        .await
        .unwrap();
    assert!(response.message.tool_calls.is_empty());
    assert_eq!(response.text(), text);
}

#[tokio::test]
async fn invoke_does_not_touch_a_response_that_already_has_tool_calls() {
    let mut scripted = ModelResponse::assistant(QWEN_CALL);
    scripted.message.tool_calls.push(ToolCall {
        id: "native_1".into(),
        name: "shell".into(),
        arguments: json!({"command": "pwd"}),
        invalid: None,
    });
    let inner = Arc::new(Scripted {
        seen: Mutex::new(Vec::new()),
        response: scripted,
        stream: Vec::new(),
    });
    let response = tuned(inner)
        .invoke(&(), request_with_tools())
        .await
        .unwrap();
    assert_eq!(response.message.tool_calls.len(), 1);
    assert_eq!(response.message.tool_calls[0].id, "native_1");
}

#[tokio::test]
async fn a_tool_less_request_keeps_literal_call_examples() {
    let response = tuned(Scripted::replying(QWEN_CALL))
        .invoke(
            &(),
            ModelRequest {
                messages: vec![Message::user("show me the format")],
                ..ModelRequest::default()
            },
        )
        .await
        .unwrap();
    assert!(response.message.tool_calls.is_empty());
    assert_eq!(response.text(), QWEN_CALL);
}

async fn collect(stream: ModelStream) -> (String, Option<ModelResponse>) {
    let mut text = String::new();
    let mut done = None;
    let mut stream = stream;
    while let Some(item) = stream.next().await {
        match item {
            ModelStreamItem::MessageDelta(d) => text.push_str(&d.text),
            ModelStreamItem::Completed(r) => done = Some(r),
            _ => {}
        }
    }
    (text, done)
}

#[tokio::test]
async fn a_streamed_text_call_is_neither_shown_live_nor_left_in_the_message() {
    let inner = Scripted::streaming(&[
        "Running it.\n",
        "<func",
        "tion=shell>\n<parameter=command>\n",
        "ls -la\n</parameter>\n</function>",
    ]);
    let stream = tuned(inner)
        .stream(&(), request_with_tools())
        .await
        .unwrap();
    let (live, done) = collect(stream).await;
    assert_eq!(live, "Running it.\n", "markup must not stream: {live:?}");
    let done = done.expect("completed");
    assert_eq!(done.message.tool_calls.len(), 1);
    assert_eq!(done.message.tool_calls[0].name, "shell");
    assert_eq!(done.text(), "Running it.");
}

#[tokio::test]
async fn streamed_prose_with_an_angle_bracket_arrives_intact() {
    let inner = Scripted::streaming(&["if a ", "< b then ", "<fun", "ny> yes"]);
    let stream = tuned(inner)
        .stream(&(), request_with_tools())
        .await
        .unwrap();
    let (live, done) = collect(stream).await;
    assert_eq!(live, "if a < b then <funny> yes");
    assert!(done.unwrap().message.tool_calls.is_empty());
}

#[tokio::test]
async fn a_held_function_mention_that_is_not_a_call_is_released_at_the_end() {
    let inner = Scripted::streaming(&["Use <function=NAME> to ", "name the tool."]);
    let stream = tuned(inner)
        .stream(&(), request_with_tools())
        .await
        .unwrap();
    let (live, done) = collect(stream).await;
    assert_eq!(live, "Use <function=NAME> to name the tool.");
    assert!(done.unwrap().message.tool_calls.is_empty());
}

// ── The wire: MLX gets the defaults, cloud providers are untouched ───────

fn completion_body() -> Value {
    json!({
        "id": "c1",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": "ok"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 3, "completion_tokens": 1, "total_tokens": 4}
    })
}

async fn mock_completions() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(completion_body()))
        .mount(&server)
        .await;
    server
}

async fn sent_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.expect("recording on");
    assert_eq!(requests.len(), 1, "exactly one request reached the server");
    serde_json::from_slice(&requests[0].body).expect("json body")
}

fn mlx_config(server: &MockServer) -> Config {
    let mut config = Config::default();
    config.local_ai.base_url = Some(format!("{}/v1", server.uri()));
    config
}

#[tokio::test]
async fn an_mlx_request_carries_native_tools_and_the_sampling_defaults() {
    let server = mock_completions().await;
    let config = mlx_config(&server);
    let (chat, _) =
        crate::neppy::inference::provider::factory::create_local_chat_model_from_string(
            "mlx:ornith-ai/Ornith-1.5-9B-MLX-8bit",
            &config,
        )
        .expect("builds");
    assert!(
        chat.profile().is_some_and(|p| p.tool_calling),
        "mlx advertises native tool calling"
    );
    chat.invoke(&(), request_with_tools()).await.expect("ok");

    let body = sent_body(&server).await;
    assert_eq!(
        body["tools"][0]["function"]["name"], "shell",
        "native tools on the wire"
    );
    assert_eq!(body["temperature"], json!(0.6));
    assert_eq!(body["top_p"], json!(0.95));
    assert_eq!(body["top_k"], json!(20));
    assert_eq!(body["max_tokens"], json!(8192));
    assert_eq!(body["thinking_budget"], json!(4096));
    assert!(body.get("min_p").is_none());
}

#[tokio::test]
async fn an_mlx_request_honours_explicit_block_values() {
    let server = mock_completions().await;
    let mut config = mlx_config(&server);
    config.mlx.servers[0].temp = 0.25;
    config.mlx.servers[0].top_k = 7;
    config.mlx.servers[0].thinking_budget = 900;
    config.mlx.servers[0].max_tokens = 1500;
    let (chat, _) =
        crate::neppy::inference::provider::factory::create_local_chat_model_from_string(
            "mlx:org/model",
            &config,
        )
        .expect("builds");
    chat.invoke(&(), request_with_tools()).await.expect("ok");

    let body = sent_body(&server).await;
    assert_eq!(body["temperature"], json!(0.25));
    assert_eq!(body["top_k"], json!(7));
    assert_eq!(body["thinking_budget"], json!(900));
    assert_eq!(body["max_tokens"], json!(1500));
}

#[tokio::test]
async fn ollama_stays_prompt_guided_and_gets_no_mlx_fields() {
    let server = mock_completions().await;
    let mut config = Config::default();
    config.local_ai.base_url = Some(server.uri());
    let (chat, _) =
        crate::neppy::inference::provider::factory::create_local_chat_model_from_string(
            "ollama:qwen3:14b",
            &config,
        )
        .expect("builds");
    assert!(chat.profile().is_some_and(|p| !p.tool_calling));
    chat.invoke(
        &(),
        ModelRequest {
            messages: vec![Message::user("hi")],
            ..ModelRequest::default()
        },
    )
    .await
    .expect("ok");
    let body = sent_body(&server).await;
    for key in [
        "top_k",
        "thinking_budget",
        "top_p",
        "max_tokens",
        "temperature",
    ] {
        assert!(
            body.get(key).is_none(),
            "ollama request must not carry `{key}`"
        );
    }
}

/// Cloud providers never pass through the MLX layer: the body for a request
/// they send is exactly what the transport built before any of this existed.
#[tokio::test]
async fn cloud_provider_requests_are_byte_for_byte_unchanged() {
    use crate::neppy::inference::provider::auth::AuthStyle as CompatAuthStyle;
    use crate::neppy::inference::provider::crate_openai::make_crate_openai_chat_model;

    for (provider, auth, model) in [
        ("openai", CompatAuthStyle::Bearer, "gpt-4o"),
        ("anthropic", CompatAuthStyle::Anthropic, "claude-sonnet-4-5"),
    ] {
        let server = mock_completions().await;
        let chat = make_crate_openai_chat_model(
            provider,
            &format!("{}/v1", server.uri()),
            "test-key",
            auth,
            model,
            &[],
            None,
            false,
        );
        chat.invoke(
            &(),
            ModelRequest {
                messages: vec![Message::user("hi")],
                temperature: Some(0.35),
                ..ModelRequest::default()
            },
        )
        .await
        .expect("ok");

        let body = sent_body(&server).await;
        let mut keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(
            keys,
            ["messages", "model", "temperature"],
            "{provider}: no MLX fields may appear on a cloud request"
        );
        assert_eq!(body["temperature"], json!(0.35));
    }
}

#[test]
fn only_mlx_gets_a_profile_derived_turn_temperature() {
    // The turn path swaps the default temperature for `mlx` alone; this pins
    // the helper to the block rather than to a global.
    let mut config = Config::default();
    config.default_temperature = 0.35;
    config.mlx.servers[0].temp = 0.4;
    assert!((turn_default_temperature(&config, "m") - 0.4).abs() < 1e-6);
}

#[test]
fn a_config_block_written_before_any_of_this_still_loads_with_unset_sentinels() {
    let server: MlxServerConfig = toml::from_str("id = \"old\"\nmodel = \"org/m\"\n").unwrap();
    assert!(server.temp < 0.0 && server.top_p < 0.0 && server.min_p < 0.0);
    assert!(server.top_k < 0);
    assert_eq!(server.thinking_budget, 0);
    assert_eq!(server.max_tokens, 0);
    // ...and resolves to the defaults rather than to a degenerate request.
    let d = MlxRequestDefaults::for_block(Some(&server));
    assert_eq!(d.temperature, DEFAULT_TEMPERATURE);
    assert_eq!(d.thinking_budget, Some(AUTO_THINKING_BUDGET));
}

#[test]
fn a_block_kind_is_matched_case_insensitively_like_the_rest_of_the_config() {
    let lm = block(|s| s.kind = "LM".to_string());
    assert_eq!(
        MlxRequestDefaults::for_block(Some(&lm)).thinking_budget,
        None
    );
}

/// End to end through the real factory model: the server answers with the call
/// as `content` in the Qwen-coder grammar wrapped in `<tool_call>` (the server's
/// own tool parser missed it), and the turn still sees a tool call.
#[tokio::test]
async fn a_wrapped_qwen_call_in_native_response_text_is_recovered_end_to_end() {
    let server = MockServer::start().await;
    let content = "Checking.\n<tool_call>\n<function=shell>\n<parameter=command>\nls -la\n</parameter>\n</function>\n</tool_call>";
    let mut body = completion_body();
    body["choices"][0]["message"]["content"] = json!(content);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let config = mlx_config(&server);
    let (chat, _) =
        crate::neppy::inference::provider::factory::create_local_chat_model_from_string(
            "mlx:ornith-ai/Ornith-1.5-9B-MLX-8bit",
            &config,
        )
        .expect("builds");

    let response = chat.invoke(&(), request_with_tools()).await.expect("ok");
    assert_eq!(
        response.message.tool_calls.len(),
        1,
        "{:?}",
        response.message
    );
    assert_eq!(response.message.tool_calls[0].name, "shell");
    assert_eq!(
        response.message.tool_calls[0].arguments,
        json!({"command": "ls -la"})
    );
    assert_eq!(
        response.text(),
        "Checking.",
        "markup is stripped from the text"
    );
}
