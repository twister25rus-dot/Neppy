use super::*;
use serde_json::json;

fn shell_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "command": {"type": "string"},
            "timeout_secs": {"type": "integer"},
            "background": {"type": "boolean"},
            "env": {"type": "object"},
        }
    })
}

fn recover(text: &str) -> Option<Recovery> {
    let schema = shell_schema();
    let null = Value::Null;
    let mut known: KnownTools<'_> = HashMap::new();
    known.insert("shell", &schema);
    known.insert("ping", &null);
    recover_text_tool_calls(text, Some(&known))
}

#[test]
fn recovers_the_json_spelling() {
    let got = recover(
        "Let me check.\n<tool_call>{\"name\": \"shell\", \"arguments\": {\"command\": \"ls -la\"}}</tool_call>",
    )
    .expect("a known tool in JSON form is recovered");
    assert_eq!(got.text, "Let me check.");
    assert_eq!(
        got.calls,
        vec![RecoveredToolCall {
            name: "shell".into(),
            arguments: json!({"command": "ls -la"}),
        }]
    );
}

#[test]
fn recovers_the_qwen_coder_spelling_with_typed_arguments() {
    let text = "<tool_call>\n<function=shell>\n<parameter=command>\nls -la /tmp\n</parameter>\n\
                <parameter=timeout_secs>\n30\n</parameter>\n<parameter=background>\nfalse\n</parameter>\n\
                </function>\n</tool_call>";
    let got = recover(text).expect("qwen-coder form is recovered");
    assert_eq!(got.text, "");
    assert_eq!(
        got.calls[0].arguments,
        json!({"command": "ls -la /tmp", "timeout_secs": 30, "background": false})
    );
}

#[test]
fn recovers_the_bare_function_form_a_special_token_stripper_leaves_behind() {
    let text =
        "Running it now.\n<function=shell>\n<parameter=command>\ndate\n</parameter>\n</function>";
    let got = recover(text).expect("bare <function=...> is recovered");
    assert_eq!(got.text, "Running it now.");
    assert_eq!(got.calls[0].name, "shell");
    assert_eq!(got.calls[0].arguments, json!({"command": "date"}));
}

#[test]
fn a_string_parameter_stays_a_string_even_when_it_looks_numeric() {
    let text = "<function=shell><parameter=command>\n123\n</parameter></function>";
    let got = recover(text).unwrap();
    assert_eq!(got.calls[0].arguments, json!({"command": "123"}));
}

#[test]
fn without_a_schema_only_unambiguous_json_is_typed() {
    let text = "<function=ping><parameter=n>\n7\n</parameter>\
                <parameter=flag>\ntrue\n</parameter>\
                <parameter=label>\nhello world\n</parameter>\
                <parameter=zip>\n007\n</parameter></function>";
    let got = recover(text).unwrap();
    assert_eq!(
        got.calls[0].arguments,
        json!({"n": 7, "flag": true, "label": "hello world", "zip": "007"})
    );
}

#[test]
fn multiple_calls_are_all_recovered_in_order() {
    let text = "<tool_call>{\"name\":\"ping\",\"arguments\":{}}</tool_call>\n\
                <tool_call><function=shell><parameter=command>\nuptime\n</parameter></function></tool_call>";
    let got = recover(text).unwrap();
    let names: Vec<_> = got.calls.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["ping", "shell"]);
}

#[test]
fn a_call_to_an_unknown_tool_is_left_in_the_text() {
    let text = "<tool_call>{\"name\": \"rm_everything\", \"arguments\": {}}</tool_call>";
    assert!(recover(text).is_none());
    let qwen = "<function=rm_everything><parameter=x>\n1\n</parameter></function>";
    assert!(recover(qwen).is_none());
}

#[test]
fn only_the_known_block_is_removed_when_a_known_and_unknown_call_mix() {
    let text = "<tool_call>{\"name\":\"nope\",\"arguments\":{}}</tool_call> then \
                <tool_call>{\"name\":\"ping\",\"arguments\":{}}</tool_call>";
    let got = recover(text).unwrap();
    assert_eq!(got.calls.len(), 1);
    assert!(
        got.text.contains("\"nope\""),
        "unknown block stays: {}",
        got.text
    );
    assert!(!got.text.contains("\"ping\""));
}

#[test]
fn prose_that_merely_mentions_the_tag_is_untouched() {
    for prose in [
        "Wrap the call in a <tool_call> tag and I will run it.",
        "The format is <tool_call>...</tool_call> with a JSON body.",
        "Use `<function=NAME>` followed by <parameter=KEY> lines.",
        "<tool_calls> is a different thing entirely",
        "no markup at all",
    ] {
        assert!(recover(prose).is_none(), "must not recover from: {prose}");
    }
}

#[test]
fn an_unterminated_block_is_not_a_call() {
    assert!(recover("<tool_call>{\"name\":\"ping\",\"arguments\":{}}").is_none());
    assert!(recover("<function=shell><parameter=command>\nls\n</parameter>").is_none());
}

#[test]
fn a_parameter_cut_off_before_its_closing_tag_still_yields_the_call() {
    let text = "<tool_call><function=shell><parameter=command>\nls -la\n</function></tool_call>";
    let got = recover(text).unwrap();
    assert_eq!(got.calls[0].arguments, json!({"command": "ls -la"}));
}

#[test]
fn json_arguments_may_use_the_alternate_keys_or_be_double_encoded() {
    let alt = "<tool_call>{\"name\":\"shell\",\"parameters\":{\"command\":\"ls\"}}</tool_call>";
    assert_eq!(
        recover(alt).unwrap().calls[0].arguments,
        json!({"command": "ls"})
    );
    let encoded =
        "<tool_call>{\"name\":\"shell\",\"arguments\":\"{\\\"command\\\":\\\"ls\\\"}\"}</tool_call>";
    assert_eq!(
        recover(encoded).unwrap().calls[0].arguments,
        json!({"command": "ls"})
    );
    let nested =
        "<tool_call>{\"function\":{\"name\":\"shell\",\"arguments\":{\"command\":\"ls\"}}}</tool_call>";
    assert_eq!(recover(nested).unwrap().calls[0].name, "shell");
}

#[test]
fn no_known_set_accepts_any_name() {
    let got = recover_text_tool_calls(
        "<tool_call>{\"name\":\"anything\",\"arguments\":{}}</tool_call>",
        None,
    )
    .unwrap();
    assert_eq!(got.calls[0].name, "anything");
}

#[test]
fn the_stream_hold_back_stops_at_an_opener_or_a_partial_one() {
    assert_eq!(hold_from_for_bare_function("hello <function=sh"), 6);
    assert_eq!(hold_from_for_bare_function("hello <func"), 6);
    assert_eq!(hold_from_for_bare_function("hello <"), 6);
    assert_eq!(hold_from_for_bare_function("hello"), 5);
    assert_eq!(hold_from_for_bare_function("a < b"), 5);
}
