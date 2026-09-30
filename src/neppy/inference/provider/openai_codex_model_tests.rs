use std::sync::{Arc, Mutex};

use futures::StreamExt;
use serde_json::{json, Value};
use tinyagents::harness::message::{AssistantMessage, ContentBlock, Message, ToolMessage};
use tinyagents::harness::model::{ModelRequest, ModelStreamItem};
use tinyagents::harness::tool::{ToolCall, ToolSchema};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::super::openai_codex_sse::{RecoverPolicy, ResponseFolder, SseDecoder};
use super::*;
use crate::neppy::inference::provider::auth::AuthStyle;
use crate::neppy::inference::provider::crate_openai::{
    build_crate_openai_model, CrateOpenAiConfig,
};

fn request_with_tool() -> ModelRequest {
    let mut request = ModelRequest::new(vec![
        Message::system("be terse"),
        Message::user("list files"),
        Message::Assistant(AssistantMessage {
            id: None,
            content: vec![ContentBlock::Text("checking".into())],
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "list".into(),
                arguments: json!({"path": "."}),
                invalid: None,
            }],
            usage: None,
        }),
        Message::Tool(ToolMessage {
            tool_call_id: "call_1".into(),
            content: vec![ContentBlock::Text("a.txt".into())],
            trusted_verbatim: false,
            artifact: None,
        }),
    ]);
    request.tools = vec![ToolSchema {
        name: "list".into(),
        description: "list dir".into(),
        parameters: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        format: Default::default(),
    }];
    // Fields the Codex backend must never see.
    request.max_tokens = Some(1024);
    request.temperature = Some(0.3);
    request.top_p = Some(0.9);
    request.seed = Some(7);
    request.stop_sequences = vec!["END".into()];
    request
}

#[test]
fn codex_body_always_streams_and_omits_rejected_fields() {
    let body = build_request_body("gpt-5.5", &request_with_tool(), true);
    assert_eq!(body["stream"], json!(true), "codex rejects non-streaming");
    assert_eq!(body["store"], json!(false));
    assert_eq!(body["model"], "gpt-5.5");
    assert_eq!(body["instructions"], "be terse");
    for key in [
        "max_output_tokens",
        "max_tokens",
        "temperature",
        "top_p",
        "seed",
        "stop",
    ] {
        assert!(body.get(key).is_none(), "{key} must not be sent to codex");
    }
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["tools"][0]["name"], "list");
    assert_eq!(body["tools"][0]["strict"], json!(false));
}

#[test]
fn codex_body_sends_tool_calls_natively() {
    let body = build_request_body("gpt-5.5", &request_with_tool(), true);
    let input = body["input"].as_array().unwrap();
    let kinds: Vec<&str> = input.iter().map(|i| i["type"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "message",
            "message",
            "function_call",
            "function_call_output"
        ]
    );
    assert_eq!(input[2]["call_id"], "call_1");
    assert_eq!(input[2]["arguments"], r#"{"path":"."}"#);
    assert_eq!(input[3]["call_id"], "call_1");
    assert_eq!(input[3]["output"], "a.txt");
}

#[test]
fn codex_body_prompt_guided_mode_sends_no_tools_and_renders_results_as_text() {
    let body = build_request_body("gpt-5.5", &request_with_tool(), false);
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
    let input = body["input"].as_array().unwrap();
    assert!(input.iter().all(|i| i["type"] == "message"));
    assert!(input.last().unwrap()["content"][0]["text"]
        .as_str()
        .unwrap()
        .starts_with("[tool_result id=call_1]"));
}

#[test]
fn codex_body_supplies_instructions_when_no_system_message() {
    let request = ModelRequest::new(vec![Message::user("hi")]);
    let body = build_request_body("gpt-5.5", &request, true);
    assert_eq!(body["instructions"], FALLBACK_INSTRUCTIONS);
}

#[test]
fn sse_decoder_handles_split_chunks_and_crlf() {
    let mut d = SseDecoder::default();
    assert!(d.push(b"event: a\ndata: {\"x\"").is_empty());
    assert_eq!(
        d.push(b":1}\n\ndata: two\r\n\r\n"),
        vec![r#"{"x":1}"#, "two"]
    );
    assert!(d.push(b"data: tail").is_empty());
    assert_eq!(d.finish().as_deref(), Some("tail"));
}

fn fold(
    events: &[Value],
    native: bool,
    has_tools: bool,
) -> Result<(Vec<ModelStreamItem>, ModelResponse), ProviderError> {
    let mut folder = ResponseFolder::default();
    let mut items = Vec::new();
    for event in events {
        items.extend(folder.apply(event, "openai", "gpt-5.5")?);
    }
    assert!(folder.terminal, "fixture must end in a terminal event");
    Ok((items, folder.finish(RecoverPolicy { native, has_tools })))
}

#[test]
fn folder_builds_text_response_with_usage() {
    let (items, response) = fold(
        &[
            json!({"type": "response.output_text.delta", "delta": "Hel"}),
            json!({"type": "response.output_text.delta", "delta": "lo"}),
            json!({"type": "response.output_item.done", "item": {
                "type": "message", "content": [{"type": "output_text", "text": "Hello"}]}}),
            json!({"type": "response.completed", "response": {"usage": {
                "input_tokens": 10, "output_tokens": 4,
                "input_tokens_details": {"cached_tokens": 3},
                "output_tokens_details": {"reasoning_tokens": 1}}}}),
        ],
        true,
        false,
    )
    .unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(response.text(), "Hello");
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    let usage = response.usage.unwrap();
    assert_eq!(
        (usage.input_tokens, usage.output_tokens, usage.total_tokens),
        (10, 4, 14)
    );
    assert_eq!((usage.cache_read_tokens, usage.reasoning_tokens), (3, 1));
}

#[test]
fn folder_decodes_function_calls_and_streams_argument_deltas() {
    let (items, response) = fold(
        &[
            json!({"type": "response.output_item.added", "item": {
                "type": "function_call", "id": "fc_1", "call_id": "call_9", "name": "shell"}}),
            json!({"type": "response.function_call_arguments.delta", "item_id": "fc_1", "delta": "{\"cmd\""}),
            json!({"type": "response.output_item.done", "item": {
                "type": "function_call", "id": "fc_1", "call_id": "call_9",
                "name": "shell", "arguments": "{\"cmd\":\"ls\"}"}}),
            json!({"type": "response.completed", "response": {}}),
        ],
        true,
        true,
    )
    .unwrap();
    let deltas: Vec<_> = items
        .iter()
        .filter_map(|i| match i {
            ModelStreamItem::ToolCallDelta(d) => Some(d),
            _ => None,
        })
        .collect();
    assert_eq!(deltas[0].tool_name.as_deref(), Some("shell"));
    assert_eq!(
        deltas[1].call_id, "call_9",
        "argument deltas route by item id"
    );
    let call = &response.message.tool_calls[0];
    assert_eq!((call.id.as_str(), call.name.as_str()), ("call_9", "shell"));
    assert_eq!(call.arguments, json!({"cmd": "ls"}));
    assert_eq!(response.finish_reason.as_deref(), Some("tool_calls"));
}

#[test]
fn folder_flags_unparseable_arguments_instead_of_failing_the_turn() {
    let (_, response) = fold(
        &[
            json!({"type": "response.output_item.done", "item": {
                "type": "function_call", "call_id": "c", "name": "t", "arguments": "{oops"}}),
            json!({"type": "response.completed", "response": {}}),
        ],
        true,
        true,
    )
    .unwrap();
    assert!(response.message.tool_calls[0].invalid.is_some());
}

#[test]
fn folder_falls_back_to_completed_output_when_no_item_events_arrive() {
    let (_, response) = fold(
        &[
            json!({"type": "response.completed", "response": {"output": [
            {"type": "message", "content": [{"type": "output_text", "text": "ok"}]}]}}),
        ],
        true,
        false,
    )
    .unwrap();
    assert_eq!(response.text(), "ok");
}

#[test]
fn folder_reports_failed_and_error_events_as_provider_errors() {
    let mut folder = ResponseFolder::default();
    let err = folder
        .apply(
            &json!({"type": "response.failed", "response": {"error": {"message": "boom", "code": "server_error"}}}),
            "openai",
            "m",
        )
        .unwrap_err();
    assert_eq!(
        (err.message.as_str(), err.code.as_deref()),
        ("boom", Some("server_error"))
    );
    let err = folder
        .apply(
            &json!({"type": "error", "message": "bad", "code": "x"}),
            "openai",
            "m",
        )
        .unwrap_err();
    assert_eq!(err.message, "bad");
}

#[test]
fn folder_marks_incomplete_responses_as_length() {
    let (_, response) = fold(
        &[json!({"type": "response.incomplete", "response": {
            "incomplete_details": {"reason": "max_output_tokens"}}})],
        true,
        false,
    )
    .unwrap();
    assert_eq!(response.finish_reason.as_deref(), Some("length"));
}

#[test]
fn error_body_reads_the_codex_detail_shape() {
    let err = parse_error_body(
        "openai",
        "m",
        400,
        r#"{"detail":"Stream must be set to true"}"#,
    );
    assert_eq!(err.message, "Stream must be set to true");
    assert_eq!(err.status, Some(400));
}

// ---- end-to-end against a loopback server ---------------------------------

#[derive(Debug, Clone)]
struct Captured {
    path: String,
    headers: String,
    body: Value,
}

/// A one-connection-per-request HTTP/1.1 server. `respond` maps the captured
/// request to `(status, content_type, body)`.
async fn spawn_server(
    respond: impl Fn(&Captured) -> (u16, &'static str, String) + Send + Sync + 'static,
) -> (String, Arc<Mutex<Vec<Captured>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let respond = Arc::new(respond);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let (log, respond) = (log.clone(), respond.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (head_end, content_length) = loop {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (pos + 4, len);
                    }
                };
                while buf.len() < head_end + content_length {
                    let n = socket.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let path = head
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split(' ')
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                let body = serde_json::from_slice(&buf[head_end..]).unwrap_or(Value::Null);
                let captured = Captured {
                    path,
                    headers: head.to_lowercase(),
                    body,
                };
                let (status, ctype, payload) = respond(&captured);
                log.lock().unwrap().push(captured);
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    (format!("http://{addr}"), seen)
}

fn sse(events: &[Value]) -> String {
    events
        .iter()
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("")))
        .collect()
}

fn codex_config<'a>(
    endpoint: &'a str,
    headers: &'a [(String, String)],
    query: &'a [(String, String)],
) -> CrateOpenAiConfig<'a> {
    CrateOpenAiConfig {
        provider_name: "openai",
        endpoint,
        api_key: "test-oauth-token",
        auth_style: AuthStyle::Bearer,
        model: "gpt-5.5",
        temperature_unsupported_models: &[],
        temperature_override: None,
        merge_system_into_user: false,
        extra_headers: headers,
        native_tool_calling: Some(true),
        vision: None,
        default_provider_options: None,
        responses_api_primary: true,
        responses_omit_max_output_tokens: true,
        extra_query_params: query,
        user_agent: Some("codex_cli_rs/test"),
    }
}

/// A server that behaves like the Codex backend: anything but `stream: true` is
/// the exact 400 from the field reports.
fn codex_like(captured: &Captured) -> (u16, &'static str, String) {
    if captured.body["stream"] != json!(true) {
        return (
            400,
            "application/json",
            r#"{"detail":"Stream must be set to true"}"#.into(),
        );
    }
    (
        200,
        "text/event-stream",
        sse(&[
            json!({"type": "response.output_text.delta", "delta": "po"}),
            json!({"type": "response.output_text.delta", "delta": "ng"}),
            json!({"type": "response.output_item.done", "item": {
                "type": "message", "content": [{"type": "output_text", "text": "pong"}]}}),
            json!({"type": "response.completed", "response": {"usage": {"input_tokens": 2, "output_tokens": 1}}}),
        ]),
    )
}

#[tokio::test]
async fn oauth_openai_invoke_sends_stream_true_and_folds_the_sse_reply() {
    let (base, seen) = spawn_server(codex_like).await;
    let headers = vec![("ChatGPT-Account-ID".to_string(), "acct_1".to_string())];
    let query = vec![("client_version".to_string(), "0.130.0".to_string())];
    let model = build_crate_openai_model(codex_config(&base, &headers, &query));

    let response = model
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user("ping")]).with_max_tokens(64),
        )
        .await
        .expect("codex-style server must accept the request");
    assert_eq!(response.text(), "pong");
    assert_eq!(response.usage.unwrap().total_tokens, 3);

    let requests = seen.lock().unwrap();
    assert_eq!(
        requests.len(),
        1,
        "no rejected non-streaming round trip first"
    );
    let request = &requests[0];
    assert!(
        request.path.starts_with("/responses"),
        "path was {}",
        request.path
    );
    assert!(request.path.contains("client_version=0.130.0"));
    assert_eq!(request.body["stream"], json!(true));
    assert_eq!(request.body["store"], json!(false));
    assert!(request.body.get("max_output_tokens").is_none());
    assert!(request
        .headers
        .contains("authorization: bearer test-oauth-token"));
    assert!(request.headers.contains("chatgpt-account-id: acct_1"));
    assert!(request.headers.contains("text/event-stream"));
}

#[tokio::test]
async fn oauth_openai_stream_emits_started_deltas_then_completed() {
    let (base, _) = spawn_server(codex_like).await;
    let model = build_crate_openai_model(codex_config(&base, &[], &[]));
    let stream = model
        .stream(&(), ModelRequest::new(vec![Message::user("ping")]))
        .await
        .unwrap();
    let items: Vec<_> = stream.collect().await;
    assert!(matches!(items.first(), Some(ModelStreamItem::Started)));
    let text: String = items
        .iter()
        .filter_map(|i| match i {
            ModelStreamItem::MessageDelta(d) => Some(d.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "pong");
    match items.last() {
        Some(ModelStreamItem::Completed(r)) => assert_eq!(r.text(), "pong"),
        other => panic!("expected Completed, got {other:?}"),
    }
}

#[tokio::test]
async fn oauth_openai_surfaces_a_backend_rejection_as_a_provider_error() {
    let (base, _) = spawn_server(|_| {
        (
            401,
            "application/json",
            r#"{"detail":"token expired"}"#.into(),
        )
    })
    .await;
    let model = build_crate_openai_model(codex_config(&base, &[], &[]));
    let err = model
        .invoke(&(), ModelRequest::new(vec![Message::user("x")]))
        .await
        .unwrap_err();
    match err {
        TinyAgentsError::Provider(p) => {
            assert_eq!(p.status, Some(401));
            assert_eq!(p.message, "token expired");
        }
        other => panic!("expected provider error, got {other:?}"),
    }
}

#[tokio::test]
async fn truncated_stream_is_a_retryable_error_not_an_empty_answer() {
    let (base, _) = spawn_server(|_| {
        (
            200,
            "text/event-stream",
            sse(&[json!({"type": "response.output_text.delta", "delta": "par"})]),
        )
    })
    .await;
    let model = build_crate_openai_model(codex_config(&base, &[], &[]));
    let err = model
        .invoke(&(), ModelRequest::new(vec![Message::user("x")]))
        .await
        .unwrap_err();
    assert!(
        matches!(err, TinyAgentsError::Provider(p) if p.code.as_deref() == Some("stream_truncated"))
    );
}

#[tokio::test]
async fn api_key_openai_path_is_unchanged_chat_completions() {
    let (base, seen) = spawn_server(|_| {
        (
            200,
            "application/json",
            json!({
                "id": "x", "model": "gpt-4.1",
                "choices": [{"index": 0, "finish_reason": "stop",
                    "message": {"role": "assistant", "content": "hi"}}],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            })
            .to_string(),
        )
    })
    .await;
    let mut config = codex_config(&base, &[], &[]);
    config.api_key = "sk-test";
    config.model = "gpt-4.1";
    config.responses_api_primary = false;
    config.responses_omit_max_output_tokens = false;
    config.user_agent = None;
    let model = build_crate_openai_model(config);

    let response = model
        .invoke(&(), ModelRequest::new(vec![Message::user("hello")]))
        .await
        .unwrap();
    assert_eq!(response.text(), "hi");
    let requests = seen.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0].path.starts_with("/chat/completions"),
        "path was {}",
        requests[0].path
    );
    assert_ne!(
        requests[0].body["stream"],
        json!(true),
        "API-key path stays a unary call"
    );
    assert!(
        requests[0].body.get("input").is_none(),
        "not a Responses body"
    );
}
