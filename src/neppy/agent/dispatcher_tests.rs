use super::*;
use crate::neppy::agent::pformat::PFormatToolParams;

#[test]
fn xml_dispatcher_parses_tool_calls() {
    let response = ChatResponse {
        text: Some(
            "Checking\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    };
    let dispatcher = XmlToolDispatcher;
    let (_, calls) = dispatcher.parse_response(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
}

#[test]
fn native_dispatcher_roundtrip() {
    let response = ChatResponse {
        text: Some("ok".into()),
        tool_calls: vec![crate::neppy::inference::provider::ToolCall {
            id: "tc1".into(),
            name: "file_read".into(),
            arguments: "{\"path\":\"a.txt\"}".into(),
            extra_content: None,
        }],
        usage: None,
        reasoning_content: None,
    };
    let dispatcher = NativeToolDispatcher;
    let (_, calls) = dispatcher.parse_response(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].tool_call_id.as_deref(), Some("tc1"));

    let msg = dispatcher.format_results(&[ToolExecutionResult {
        name: "file_read".into(),
        output: "hello".into(),
        success: true,
        tool_call_id: Some("tc1".into()),
    }]);
    match msg {
        ConversationMessage::ToolResults(results) => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].tool_call_id, "tc1");
        }
        _ => panic!("expected tool results"),
    }
}

#[test]
fn native_dispatcher_falls_back_to_xml_tool_calls() {
    let response = ChatResponse {
        text: Some(
            "Checking files...\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    };
    let dispatcher = NativeToolDispatcher;
    let (text, calls) = dispatcher.parse_response(&response);
    assert_eq!(text, "Checking files...");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].tool_call_id, None);
}

#[test]
fn native_dispatcher_falls_back_to_invoke_tag() {
    let response = ChatResponse {
        text: Some(
            "Let me run this.\n<invoke>{\"name\":\"shell\",\"arguments\":{\"command\":\"pwd\"}}</invoke>".into(),
        ),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    };
    let dispatcher = NativeToolDispatcher;
    let (text, calls) = dispatcher.parse_response(&response);
    assert_eq!(text, "Let me run this.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
}

#[test]
fn xml_format_results_contains_tool_result_tags() {
    let dispatcher = XmlToolDispatcher;
    let msg = dispatcher.format_results(&[ToolExecutionResult {
        name: "shell".into(),
        output: "ok".into(),
        success: true,
        tool_call_id: None,
    }]);
    let rendered = match msg {
        ConversationMessage::Chat(chat) => chat.content,
        _ => String::new(),
    };
    assert!(rendered.contains("<tool_result"));
    assert!(rendered.contains("shell"));
}

fn pformat_registry_for(name: &str, props: serde_json::Value) -> PFormatRegistry {
    let schema = serde_json::json!({
        "type": "object",
        "properties": props
    });
    let mut reg = PFormatRegistry::new();
    reg.insert(name.to_string(), PFormatToolParams::from_schema(&schema));
    reg
}

#[test]
fn pformat_dispatcher_parses_tool_call_tag() {
    // The model emits a p-format call inside a `<tool_call>` tag.
    // The dispatcher should pull it out, look up the tool's
    // parameter ordering, and produce named JSON args.
    let registry = pformat_registry_for(
        "get_weather",
        serde_json::json!({
            "location": { "type": "string" },
            "unit": { "type": "string" }
        }),
    );
    let dispatcher = PFormatToolDispatcher::new(registry);
    let response = ChatResponse {
        text: Some(
            "Let me check the weather.\n<tool_call>get_weather[London|metric]</tool_call>".into(),
        ),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    };
    let (text, calls) = dispatcher.parse_response(&response);
    assert_eq!(text, "Let me check the weather.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_weather");
    assert_eq!(
        calls[0].arguments,
        serde_json::json!({"location": "London", "unit": "metric"})
    );
}

#[test]
fn pformat_dispatcher_falls_back_to_json_in_tag() {
    // A model that ignored the p-format protocol and emitted a
    // JSON tool call should still be parsed correctly — the
    // dispatcher's whole point is to be a strict superset of the
    // legacy XML behaviour.
    let registry = pformat_registry_for(
        "shell",
        serde_json::json!({ "command": { "type": "string" } }),
    );
    let dispatcher = PFormatToolDispatcher::new(registry);
    let response = ChatResponse {
        text: Some(
            "Running it now.\n<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    };
    let (text, calls) = dispatcher.parse_response(&response);
    assert_eq!(text, "Running it now.");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
}

#[test]
fn pformat_dispatcher_handles_multiple_tags() {
    let registry = pformat_registry_for(
        "shell",
        serde_json::json!({ "command": { "type": "string" } }),
    );
    let dispatcher = PFormatToolDispatcher::new(registry);
    let response = ChatResponse {
        text: Some(
            "Step 1.\n<tool_call>shell[ls]</tool_call>\nStep 2.\n<tool_call>shell[pwd]</tool_call>"
                .into(),
        ),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    };
    let (_text, calls) = dispatcher.parse_response(&response);
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
    assert_eq!(calls[1].arguments, serde_json::json!({"command": "pwd"}));
}

#[test]
fn pformat_dispatcher_reports_pformat_tool_call_format() {
    let dispatcher = PFormatToolDispatcher::new(PFormatRegistry::new());
    assert_eq!(dispatcher.tool_call_format(), ToolCallFormat::PFormat);
}

#[test]
fn pformat_dispatcher_instructions_are_protocol_only() {
    // The dispatcher's prompt_instructions should NOT re-render
    // the tool catalogue — that's `ToolsSection`'s job. Otherwise
    // every tool gets emitted twice and the prompt double-pays.
    let dispatcher = PFormatToolDispatcher::new(PFormatRegistry::new());
    // Pass in a tool to make sure the dispatcher ignores it.
    struct DummyTool;
    #[async_trait::async_trait]
    impl Tool for DummyTool {
        fn name(&self) -> &str {
            "should_not_appear"
        }
        fn description(&self) -> &str {
            "this string must not show up in the dispatcher instructions"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _args: serde_json::Value,
        ) -> anyhow::Result<crate::neppy::tools::ToolResult> {
            Ok(crate::neppy::tools::ToolResult::success("ok"))
        }
    }
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(DummyTool)];
    let instructions = dispatcher.prompt_instructions(&tools);
    assert!(instructions.contains("Tool Use Protocol"));
    assert!(
        !instructions.contains("should_not_appear"),
        "dispatcher instructions must not duplicate the tool catalogue, got:\n{instructions}"
    );
}

#[test]
fn native_format_results_keeps_tool_call_id() {
    let dispatcher = NativeToolDispatcher;
    let msg = dispatcher.format_results(&[ToolExecutionResult {
        name: "shell".into(),
        output: "ok".into(),
        success: true,
        tool_call_id: Some("tc-1".into()),
    }]);

    match msg {
        ConversationMessage::ToolResults(results) => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].tool_call_id, "tc-1");
        }
        _ => panic!("expected ToolResults variant"),
    }
}

// ── TAURI-RUST-7 regression: tool_calls / ToolResults pairing ──────────
//
// Providers reject any assistant `tool_calls` message that isn't immediately
// followed by `tool` messages responding to every `tool_call_id`. Cached
// transcript restores and mid-turn aborts can produce bisected pairs. The
// fix in `to_provider_messages` drops unpaired AssistantToolCalls and orphan
// ToolResults so the wire payload is always well-formed.

fn assistant_tool_calls(id: &str) -> ConversationMessage {
    ConversationMessage::AssistantToolCalls {
        text: Some("calling tool".into()),
        tool_calls: vec![crate::neppy::inference::provider::ToolCall {
            id: id.into(),
            name: "shell".into(),
            arguments: "{}".into(),
            extra_content: None,
        }],
        reasoning_content: None,
        extra_metadata: None,
    }
}

fn tool_results(id: &str) -> ConversationMessage {
    use crate::neppy::agent::messages::ToolResultMessage;
    ConversationMessage::ToolResults(vec![ToolResultMessage {
        tool_call_id: id.into(),
        content: "ok".into(),
    }])
}

fn user_chat(text: &str) -> ConversationMessage {
    ConversationMessage::Chat(crate::neppy::agent::messages::ChatMessage::user(text))
}

fn assistant_chat(text: &str) -> ConversationMessage {
    ConversationMessage::Chat(crate::neppy::agent::messages::ChatMessage::assistant(text))
}

#[test]
fn to_provider_messages_keeps_paired_tool_cycle() {
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls("tc-1"),
        tool_results("tc-1"),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant", "tool", "assistant"]);
}

#[test]
fn to_provider_messages_drops_trailing_unpaired_tool_calls() {
    // The assistant emitted tool_calls but the run was aborted before the
    // ToolResults were persisted. The trailing tool_calls must not reach
    // the wire.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_chat("ok"),
        user_chat("again"),
        assistant_tool_calls("tc-2"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "user"],
        "trailing unpaired AssistantToolCalls must be stripped"
    );
}

#[test]
fn to_provider_messages_drops_mid_history_unpaired_tool_calls() {
    // History with a bisected pair in the middle: tool_calls followed
    // directly by a Chat (not ToolResults). Drop the tool_calls; keep
    // everything else.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls("tc-3"), // bisected — no following ToolResults
        user_chat("nevermind"),
        assistant_chat("ok"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "user", "assistant"]);
}

#[test]
fn to_provider_messages_drops_orphan_tool_results() {
    // Symmetric drop: ToolResults whose preceding AssistantToolCalls was
    // never emitted (either never persisted or already dropped above)
    // must not appear in the wire payload.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        tool_results("tc-4"), // orphan — no preceding AssistantToolCalls
        assistant_chat("ok"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, vec!["user", "assistant"]);
}

#[test]
fn to_provider_messages_handles_multiple_tool_cycles() {
    // Two paired cycles in a row — both must survive.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("a"),
        assistant_tool_calls("tc-5"),
        tool_results("tc-5"),
        assistant_tool_calls("tc-6"),
        tool_results("tc-6"),
        assistant_chat("final"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec![
            "user",
            "assistant",
            "tool",
            "assistant",
            "tool",
            "assistant"
        ]
    );
}

// ── tool_call_id set-pairing (CodeRabbit follow-up) ─────────────────────

fn assistant_tool_calls_multi(ids: &[&str]) -> ConversationMessage {
    ConversationMessage::AssistantToolCalls {
        text: Some("calling tools".into()),
        tool_calls: ids
            .iter()
            .map(|id| crate::neppy::inference::provider::ToolCall {
                id: (*id).into(),
                name: "shell".into(),
                arguments: "{}".into(),
                extra_content: None,
            })
            .collect(),
        reasoning_content: None,
        extra_metadata: None,
    }
}

#[test]
fn native_dispatcher_serializes_reasoning_content_for_tool_call_turns() {
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        ConversationMessage::AssistantToolCalls {
            text: Some("calling tools".into()),
            tool_calls: vec![crate::neppy::inference::provider::ToolCall {
                id: "tc-1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
                extra_content: None,
            }],
            reasoning_content: Some("chain-of-thought replay blob".into()),
            extra_metadata: None,
        },
        tool_results("tc-1"),
    ];

    let out = dispatcher.to_provider_messages(&history);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].role, "assistant");

    let payload: serde_json::Value =
        serde_json::from_str(&out[0].content).expect("assistant payload should be valid JSON");
    assert_eq!(
        payload
            .get("reasoning_content")
            .and_then(serde_json::Value::as_str),
        Some("chain-of-thought replay blob")
    );
}

#[test]
fn native_dispatcher_omits_reasoning_content_when_absent() {
    let dispatcher = NativeToolDispatcher;
    let history = vec![assistant_tool_calls("tc-1"), tool_results("tc-1")];

    let out = dispatcher.to_provider_messages(&history);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].role, "assistant");

    let payload: serde_json::Value =
        serde_json::from_str(&out[0].content).expect("assistant payload should be valid JSON");
    assert!(
        payload.get("reasoning_content").is_none(),
        "reasoning_content should be omitted when absent"
    );
}

fn tool_results_multi(ids: &[&str]) -> ConversationMessage {
    use crate::neppy::agent::messages::ToolResultMessage;
    ConversationMessage::ToolResults(
        ids.iter()
            .map(|id| ToolResultMessage {
                tool_call_id: (*id).into(),
                content: "ok".into(),
            })
            .collect(),
    )
}

#[test]
fn to_provider_messages_drops_pair_when_tool_call_ids_mismatch() {
    // Opener requests `tc-1`, but the only ToolResults entry answers `tc-x`.
    // Backend would 400 with "insufficient tool messages following tool_calls"
    // — drop both.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls_multi(&["tc-1"]),
        tool_results_multi(&["tc-x"]),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant"],
        "id-set mismatch must drop the bisected pair entirely, kept: {roles:?}"
    );
}

#[test]
fn to_provider_messages_drops_pair_when_results_are_partial() {
    // Opener requests two tool_call_ids, results answer only one. Backend
    // rejects with "insufficient tool messages". Strict set equality drops
    // the pair.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls_multi(&["tc-1", "tc-2"]),
        tool_results_multi(&["tc-1"]),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant"],
        "partial tool-result coverage must drop the pair, kept: {roles:?}"
    );
}

#[test]
fn to_provider_messages_keeps_pair_with_full_id_coverage() {
    // Strict set equality: opener has {tc-1, tc-2}, results cover both,
    // even if listed in a different order. Both messages must be emitted.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls_multi(&["tc-1", "tc-2"]),
        tool_results_multi(&["tc-2", "tc-1"]),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant", "tool", "tool", "assistant"]
    );
}

#[test]
fn to_provider_messages_drops_pair_with_extra_unsolicited_results() {
    // Opener requests `tc-1`; results answer `tc-1` *and* an unsolicited
    // `tc-extra`. The id sets differ, so the pair is dropped.
    let dispatcher = NativeToolDispatcher;
    let history = vec![
        user_chat("hi"),
        assistant_tool_calls_multi(&["tc-1"]),
        tool_results_multi(&["tc-1", "tc-extra"]),
        assistant_chat("done"),
    ];
    let out = dispatcher.to_provider_messages(&history);
    let roles: Vec<&str> = out.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(
        roles,
        vec!["user", "assistant"],
        "extra unsolicited tool_call_ids must invalidate the pair, kept: {roles:?}"
    );
}

// `ChatMessage.role` is a free-form `String` in the durable transcript record,
// but `to_dialect_message`/`from_dialect_message` route it through the
// crate's closed `DialectRole` (system/user/assistant/tool) — the same four
// roles every dispatcher already spoke before this seam existed. A role
// outside that vocabulary (nothing in this codebase produces one today; see
// the doc comment on `to_dialect_message`) is treated as a `user` turn on the
// way out, matching how every provider already treats an unrecognized role.
// This pins that intentional behaviour so it does not silently drift.
#[test]
fn to_provider_messages_treats_unrecognized_role_as_user() {
    let dispatcher = XmlToolDispatcher;
    let mut developer_message = crate::neppy::agent::messages::ChatMessage::user("hi");
    developer_message.role = "developer".to_string();
    let history = vec![ConversationMessage::Chat(developer_message)];

    let out = dispatcher.to_provider_messages(&history);

    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].role, "user",
        "an unrecognized role must fall back to `user`, not be dropped or panic"
    );
    assert_eq!(out[0].content, "hi");
}

// ── One tool catalogue, never two ────────────────────────────────────────

#[test]
fn xml_protocol_instructions_carry_no_catalogue() {
    let protocol = XmlToolDispatcher::protocol_instructions();
    assert!(protocol.contains("## Tool Use Protocol"));
    assert!(protocol.contains("<tool_call>"));
    assert!(
        !protocol.contains("### Available Tools"),
        "a heading over nothing reads as a missing list: {protocol}"
    );
    assert!(!protocol.contains("Parameters: `"), "no schema catalogue");
    assert!(protocol.contains("`## Tools` section"));
}

#[test]
fn xml_session_instructions_are_protocol_only_whatever_the_specs() {
    let specs = vec![ToolSpec {
        name: "shell".into(),
        description: "run a command".into(),
        parameters: serde_json::json!({"type":"object","properties":{"command":{"type":"string"}}}),
    }];
    let from_specs = XmlToolDispatcher
        .prompt_instructions_for_specs(&specs)
        .expect("xml renders instructions from specs");
    assert_eq!(from_specs, XmlToolDispatcher::protocol_instructions());
    assert!(!from_specs.contains("run a command"));
    // The explicit "full catalogue" renderer still exists for callers that want it.
    assert!(XmlToolDispatcher::prompt_instructions_from_specs(&specs).contains("run a command"));
}

#[test]
fn xml_prompt_instructions_with_no_tools_is_protocol_only_and_with_tools_is_unchanged() {
    assert_eq!(
        XmlToolDispatcher.prompt_instructions(&[]),
        XmlToolDispatcher::protocol_instructions()
    );
}

// ── Native dispatcher: Qwen-coder text fallback ──────────────────────────

fn text_response(text: &str) -> ChatResponse {
    ChatResponse {
        text: Some(text.into()),
        tool_calls: vec![],
        usage: None,
        reasoning_content: None,
    }
}

#[test]
fn native_dispatcher_recovers_a_qwen_coder_call_written_as_text() {
    let text = "Looking.\n<tool_call>\n<function=shell>\n<parameter=command>\nls\n</parameter>\n</function>\n</tool_call>";
    let (visible, calls) = NativeToolDispatcher.parse_response(&text_response(text));
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].arguments, serde_json::json!({"command": "ls"}));
    assert_eq!(visible, "Looking.");
}

#[test]
fn native_dispatcher_still_recovers_the_json_text_form() {
    let text = "<tool_call>{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}</tool_call>";
    let (_, calls) = NativeToolDispatcher.parse_response(&text_response(text));
    assert_eq!(calls.len(), 1);
}

#[test]
fn native_dispatcher_ignores_prose_that_mentions_the_tags() {
    let text = "Wrap a call in <tool_call> tags, or write <function=NAME> yourself.";
    let (visible, calls) = NativeToolDispatcher.parse_response(&text_response(text));
    assert!(calls.is_empty());
    assert_eq!(visible, text);
}

#[test]
fn native_dispatcher_prefers_structured_calls_over_text() {
    let response = ChatResponse {
        text: Some("<function=shell><parameter=command>\nrm\n</parameter></function>".into()),
        tool_calls: vec![crate::neppy::inference::provider::ToolCall {
            id: "tc1".into(),
            name: "file_read".into(),
            arguments: "{\"path\":\"a.txt\"}".into(),
            extra_content: None,
        }],
        usage: None,
        reasoning_content: None,
    };
    let (_, calls) = NativeToolDispatcher.parse_response(&response);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "file_read");
}

// ── Measured: the debug_agent system prompt on an MLX route ──────────────

/// Builds the real `debug_agent` session on a given chat route and returns the
/// rendered system prompt, its tool count, and the visible tool specs.
async fn debug_agent_prompt(chat_provider: &str) -> (String, Vec<ToolSpec>) {
    use crate::neppy::agent::context::prompt::LearnedContextData;
    use crate::neppy::agent::debug_mode::turn;
    use crate::neppy::agent::harness::AgentDefinitionRegistry;

    AgentDefinitionRegistry::init_global_builtins().expect("builtins");
    let workspace = tempfile::TempDir::new().expect("tempdir");
    let mut config = crate::neppy::config::Config {
        workspace_dir: workspace.path().to_path_buf(),
        action_dir: workspace.path().to_path_buf(),
        ..crate::neppy::config::Config::default()
    };
    config.chat_provider = Some(chat_provider.to_string());

    // `debug_agent` only builds inside a Debug-mode turn, which needs a git repo.
    let repo = tempfile::tempdir().expect("repo");
    for args in [
        &["init", "-q"][..],
        &["commit", "--allow-empty", "-qm", "i"][..],
    ] {
        assert!(std::process::Command::new("git")
            .arg("-C")
            .arg(repo.path())
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .status()
            .expect("git runs")
            .success());
    }
    let ws = tempfile::tempdir().expect("ws");
    let root = repo.path().canonicalize().expect("canonical root");
    turn::run_in_root(ws.path(), root, "measure", async {
        let agent = crate::neppy::agent::Agent::from_config_for_agent(&config, "debug_agent")
            .map_err(|e| e.to_string())?;
        let prompt = agent
            .build_system_prompt(LearnedContextData::default())
            .map_err(|e| e.to_string())?;
        Ok::<_, String>((prompt, agent.tool_specs().to_vec()))
    })
    .await
    .expect("debug_agent builds inside a Debug turn")
}

/// The scout measured 141,785 characters for `debug_agent` on MLX against
/// 15,545 on a hosted model: a `## Tools` catalogue (41.8k) plus a second,
/// full-schema one under `## Tool Use Protocol` (84.7k) because the runtime was
/// advertised as prompt-guided. Native calling removes both from the prompt (the
/// schemas ride the request), and a runtime that stays prompt-guided now gets the
/// protocol once.
#[tokio::test(flavor = "multi_thread")]
async fn debug_agent_prompt_carries_at_most_one_tool_catalogue() {
    let (native, specs) = debug_agent_prompt("mlx:org/model").await;
    let (guided, _) = debug_agent_prompt("ollama:org/model").await;

    // Reconstruct what the prompt-guided path used to send: the same prompt with
    // the dialect's full-schema catalogue in place of the protocol-only block.
    let protocol = XmlToolDispatcher::protocol_instructions();
    let old_full = XmlToolDispatcher::prompt_instructions_from_specs(&specs);
    assert!(
        guided.contains(&protocol),
        "prompt-guided agents get the protocol block"
    );
    let before = guided.len() - protocol.len() + old_full.len();

    eprintln!(
        "[measure] debug_agent system prompt: tools={} \
         before(mlx, prompt-guided, two catalogues)={} chars \
         after(mlx, native)={} chars after(prompt-guided, one catalogue)={} chars",
        specs.len(),
        before,
        native.len(),
        guided.len()
    );

    // One catalogue at most: a tool's name appears as a catalogue entry once.
    let entry = "- **shell**:";
    assert!(
        guided.matches(entry).count() <= 1,
        "duplicate catalogue entry"
    );
    assert!(
        !guided.contains("### Available Tools"),
        "the second, full-schema catalogue is gone"
    );
    assert!(guided.len() < before, "{} !< {before}", guided.len());
    // Native: no catalogue in the prompt at all (schemas are sent natively).
    assert!(!native.contains(entry));
    assert!(native.len() < guided.len());
}

// ── Compat: a thread written by the XML dispatcher, continued natively ───

/// Existing MLX users have threads whose tool rounds were persisted by the
/// prompt-guided dispatcher (calls as `<tool_call>` text inside an assistant
/// message, results folded into a user turn). Local MLX turns now run native, so
/// the same history is replayed onto a native request and then continued with a
/// native tool round. The replay must stay a valid request: old rounds are plain
/// text, never orphaned `tool` messages, and the new native round pairs ids.
#[tokio::test]
async fn an_xml_written_history_replays_validly_onto_a_native_request() {
    use crate::neppy::agent::message_convert::history_to_messages;
    use crate::neppy::agent::messages::ChatMessage;
    use crate::neppy::config::Config;
    use tinyagents::harness::model::ModelRequest;

    // Persist two old tool rounds exactly as the XML dispatcher does.
    let xml_round = |id: &str, cmd: &str, out: &str| -> Vec<ConversationMessage> {
        vec![
            ConversationMessage::AssistantToolCalls {
                // Prompt-guided models write the call into the visible text.
                text: Some(format!(
                    "Checking.\n<tool_call>{{\"name\":\"shell\",\"arguments\":{{\"command\":\"{cmd}\"}}}}</tool_call>"
                )),
                tool_calls: vec![crate::neppy::inference::provider::ToolCall {
                    id: id.into(),
                    name: "shell".into(),
                    arguments: format!("{{\"command\":\"{cmd}\"}}"),
                    extra_content: None,
                }],
                reasoning_content: None,
                extra_metadata: None,
            },
            XmlToolDispatcher.format_results(&[ToolExecutionResult {
                name: "shell".into(),
                output: out.into(),
                success: true,
                tool_call_id: Some(id.into()),
            }]),
        ]
    };
    let mut conversation = xml_round("c1", "ls", "a.txt");
    conversation.extend(xml_round("c2", "pwd", "/tmp"));
    let mut history = vec![ChatMessage::system("sys"), ChatMessage::user("list files")];
    history.extend(XmlToolDispatcher.to_provider_messages(&conversation));
    history.push(ChatMessage::assistant("There is a.txt in /tmp."));
    // Sanity: the old form really is text, not envelopes.
    assert!(history.iter().any(|m| m.content.contains("<tool_call>")));

    // Continue natively: new user turn, then a native assistant call + result.
    history.push(ChatMessage::user("now delete it"));
    let native_round = vec![
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![crate::neppy::inference::provider::ToolCall {
                id: "n1".into(),
                name: "shell".into(),
                arguments: "{\"command\":\"rm a.txt\"}".into(),
                extra_content: None,
            }],
            reasoning_content: None,
            extra_metadata: None,
        },
        NativeToolDispatcher.format_results(&[ToolExecutionResult {
            name: "shell".into(),
            output: "removed".into(),
            success: true,
            tool_call_id: Some("n1".into()),
        }]),
    ];
    history.extend(NativeToolDispatcher.to_provider_messages(&native_round));

    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "c", "object": "chat.completion",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "done"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        })))
        .mount(&server)
        .await;
    let mut config = Config::default();
    config.local_ai.base_url = Some(format!("{}/v1", server.uri()));
    let (chat, _) =
        crate::neppy::inference::provider::factory::create_local_chat_model_from_string(
            "mlx:org/model",
            &config,
        )
        .expect("builds");
    assert!(
        chat.profile().is_some_and(|p| p.tool_calling),
        "native path"
    );
    let request = ModelRequest {
        messages: history_to_messages(&history),
        tools: vec![tinyagents::harness::tool::ToolSchema::new(
            "shell",
            "run",
            serde_json::json!({"type":"object","properties":{"command":{"type":"string"}}}),
        )],
        ..ModelRequest::default()
    };
    chat.invoke(&(), request)
        .await
        .expect("request is accepted");

    let received = server.received_requests().await.expect("recorded");
    let body: serde_json::Value = serde_json::from_slice(&received[0].body).unwrap();
    let messages = body["messages"].as_array().expect("messages");

    // Every `tool` message answers a tool_call declared by the assistant message
    // directly before it, and every declared call is answered.
    let mut open: Vec<String> = Vec::new();
    for message in messages {
        match message["role"].as_str().unwrap() {
            "assistant" => {
                assert!(open.is_empty(), "unanswered tool_calls before {message}");
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    let id = call["id"].as_str().expect("tool_call id present");
                    assert!(!id.is_empty());
                    open.push(id.to_string());
                }
            }
            "tool" => {
                let id = message["tool_call_id"].as_str().expect("tool_call_id");
                let pos = open.iter().position(|o| o == id);
                assert!(pos.is_some(), "orphaned tool result {id}");
                open.remove(pos.unwrap());
            }
            _ => assert!(open.is_empty(), "user/system turn inside a tool round"),
        }
    }
    assert!(open.is_empty(), "every tool_call is answered");

    // The old rounds stay readable: calls and results survive as text.
    let wire = serde_json::to_string(messages).unwrap();
    assert!(wire.contains("ls") && wire.contains("a.txt") && wire.contains("/tmp"));
    assert!(wire.contains("tool_result") || wire.contains("Tool results"));
    // And exactly one native round (n1) exists.
    assert_eq!(wire.matches("\"tool_call_id\"").count(), 1);
}
