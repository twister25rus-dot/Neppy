//! Recover a tool call a small local model wrote as *text*.
//!
//! With native tool calling on, `mlx_vlm.server` runs the model's own tool
//! parser (`qwen3_coder` for the Qwen family) and answers with OpenAI
//! `tool_calls`. A 9B model still sometimes drifts from its template, and the
//! server's parser then misses the call: it comes back as plain `content`, the
//! turn sees no tool call, and the model's final answer is a half-formed tag.
//!
//! Two spellings matter in practice:
//!
//! ```text
//! <tool_call>{"name": "shell", "arguments": {"command": "ls"}}</tool_call>
//!
//! <tool_call>
//! <function=shell>
//! <parameter=command>
//! ls
//! </parameter>
//! </function>
//! </tool_call>
//! ```
//!
//! The second is the Qwen-coder chat template's own grammar. The `<tool_call>`
//! tags are special tokens, and a server that decodes with
//! `skip_special_tokens` leaves the bare `<function=...>` form behind, so the
//! parser accepts the wrapper as optional.
//!
//! **A call is only recovered when it names a tool that exists.** Prose that
//! merely mentions `<tool_call>`, or shows an example for a tool the agent does
//! not have, is left exactly as written: turning every such mention into a call
//! would execute things the model never meant to run.
//!
//! The parser is pure (text in, text and calls out) so every rule is a unit
//! test. Arguments are typed against the advertised JSON schema when one is
//! known, because `<parameter=...>` values are all text on the wire and `"123"`
//! is a string for one parameter and a number for the next.

use std::collections::HashMap;

use serde_json::{Map, Value};

const LOG: &str = "[mlx:tool-text]";

const TOOL_CALL_OPEN: &str = "<tool_call>";
const TOOL_CALL_CLOSE: &str = "</tool_call>";
const FUNCTION_OPEN: &str = "<function=";
const FUNCTION_CLOSE: &str = "</function>";
const PARAMETER_OPEN: &str = "<parameter=";
const PARAMETER_CLOSE: &str = "</parameter>";

/// Keys a model may nest its arguments under in the JSON spelling.
const ARGUMENT_KEYS: [&str; 4] = ["arguments", "parameters", "args", "input"];

/// A tool call recovered from text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecoveredToolCall {
    pub name: String,
    /// Always a JSON object.
    pub arguments: Value,
}

/// What a recovery pass produced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Recovery {
    /// The input with every recovered call removed, trimmed.
    pub text: String,
    pub calls: Vec<RecoveredToolCall>,
}

/// The tools a response may legitimately call, keyed by name, with each one's
/// JSON-Schema `parameters` for argument typing (`Null` when unknown).
pub(crate) type KnownTools<'a> = HashMap<&'a str, &'a Value>;

/// Recover text-form tool calls from `text`.
///
/// `known` is `Some` to restrict recovery to those tool names (the normal
/// case: the names the request advertised) and `None` to accept any name —
/// only for callers that merely ask "did the model attempt a call".
///
/// Returns `None` when nothing was recovered, so the caller keeps the original
/// text byte for byte.
pub(crate) fn recover_text_tool_calls(
    text: &str,
    known: Option<&KnownTools<'_>>,
) -> Option<Recovery> {
    if !text.contains(FUNCTION_OPEN) && !text.contains(TOOL_CALL_OPEN) {
        return None;
    }
    let mut calls = Vec::new();
    let mut cleaned = String::new();
    let mut rest = text;

    while let Some(span) = next_span(rest) {
        let (before, after) = rest.split_at(span.start);
        cleaned.push_str(before);
        let block = &after[..span.len];
        match parse_block(span.inner(after), known) {
            Some(call) => {
                log::debug!("{LOG} recovered call tool={} form={}", call.name, span.kind);
                calls.push(call);
            }
            None => {
                // Not a call we can vouch for: leave the markup in the text.
                log::debug!(
                    "{LOG} left a `{}` block untouched (unknown tool or unparsable body)",
                    span.kind
                );
                cleaned.push_str(block);
            }
        }
        rest = &after[span.len..];
    }
    cleaned.push_str(rest);

    if calls.is_empty() {
        return None;
    }
    Some(Recovery {
        text: cleaned.trim().to_string(),
        calls,
    })
}

/// Whether `text` could be the start of a bare `<function=` call, i.e. it
/// contains the opener or ends in a proper prefix of it. Used by the streaming
/// filter to decide how much of the tail to hold back.
pub(crate) fn hold_from_for_bare_function(buf: &str) -> usize {
    if let Some(pos) = buf.find(FUNCTION_OPEN) {
        return pos;
    }
    let len = buf.len();
    let max = FUNCTION_OPEN.len().min(len);
    for k in (1..=max).rev() {
        if buf.is_char_boundary(len - k) && buf[len - k..] == FUNCTION_OPEN[..k] {
            return len - k;
        }
    }
    len
}

struct Span {
    start: usize,
    len: usize,
    /// Offset of the inner body relative to `start`, and its length.
    inner_start: usize,
    inner_len: usize,
    kind: &'static str,
}

impl Span {
    fn inner<'a>(&self, after: &'a str) -> &'a str {
        &after[self.inner_start..self.inner_start + self.inner_len]
    }
}

/// Find the earliest complete block: a `<tool_call>…</tool_call>` pair, or a
/// bare `<function=…>…</function>` outside one.
fn next_span(text: &str) -> Option<Span> {
    let wrapped = text.find(TOOL_CALL_OPEN);
    let bare = text.find(FUNCTION_OPEN);
    match (wrapped, bare) {
        (Some(w), b) if b.is_none_or(|b| w <= b) => {
            let body_start = w + TOOL_CALL_OPEN.len();
            let end = text[body_start..].find(TOOL_CALL_CLOSE)?;
            Some(Span {
                start: w,
                len: body_start + end + TOOL_CALL_CLOSE.len() - w,
                inner_start: TOOL_CALL_OPEN.len(),
                inner_len: end,
                kind: "tool_call",
            })
        }
        (_, Some(b)) => {
            let end = text[b..].find(FUNCTION_CLOSE)?;
            let len = end + FUNCTION_CLOSE.len();
            Some(Span {
                start: b,
                len,
                inner_start: 0,
                inner_len: len,
                kind: "function",
            })
        }
        _ => None,
    }
}

fn is_known(name: &str, known: Option<&KnownTools<'_>>) -> bool {
    known.is_none_or(|known| known.contains_key(name))
}

fn schema_for<'a>(name: &str, known: Option<&KnownTools<'a>>) -> Option<&'a Value> {
    known.and_then(|known| known.get(name).copied())
}

fn parse_block(inner: &str, known: Option<&KnownTools<'_>>) -> Option<RecoveredToolCall> {
    let inner = inner.trim();
    if inner.contains(FUNCTION_OPEN) {
        return parse_qwen_function(inner, known);
    }
    parse_json_call(inner, known)
}

/// `{"name": ..., "arguments": {...}}`
fn parse_json_call(inner: &str, known: Option<&KnownTools<'_>>) -> Option<RecoveredToolCall> {
    let value: Value = serde_json::from_str(inner).ok()?;
    let object = value.as_object()?;
    // OpenAI's own nesting: {"function": {"name": ..., "arguments": ...}}.
    let object = object
        .get("function")
        .and_then(Value::as_object)
        .unwrap_or(object);
    let name = object.get("name")?.as_str()?.trim();
    if name.is_empty() || !is_known(name, known) {
        return None;
    }
    let raw = ARGUMENT_KEYS.iter().find_map(|key| object.get(*key));
    let arguments = match raw {
        Some(Value::Object(map)) => Value::Object(map.clone()),
        // Some templates double-encode the arguments as a JSON string.
        Some(Value::String(encoded)) => match serde_json::from_str::<Value>(encoded) {
            Ok(Value::Object(map)) => Value::Object(map),
            _ => return None,
        },
        Some(_) => return None,
        None => Value::Object(Map::new()),
    };
    Some(RecoveredToolCall {
        name: name.to_string(),
        arguments,
    })
}

/// `<function=NAME><parameter=K>V</parameter>…</function>`
fn parse_qwen_function(inner: &str, known: Option<&KnownTools<'_>>) -> Option<RecoveredToolCall> {
    let start = inner.find(FUNCTION_OPEN)? + FUNCTION_OPEN.len();
    let name_end = inner[start..].find('>')?;
    let name = inner[start..start + name_end].trim().trim_matches('"');
    if name.is_empty() || !is_known(name, known) {
        return None;
    }
    let body = &inner[start + name_end + 1..];
    let body = body.find(FUNCTION_CLOSE).map_or(body, |end| &body[..end]);
    let schema = schema_for(name, known);

    let mut arguments = Map::new();
    let mut rest = body;
    while let Some(open) = rest.find(PARAMETER_OPEN) {
        let key_start = open + PARAMETER_OPEN.len();
        let key_end = rest[key_start..].find('>')?;
        let key = rest[key_start..key_start + key_end]
            .trim()
            .trim_matches('"');
        let value_start = key_start + key_end + 1;
        // A value cut off before its closing tag runs to the end of the body.
        let (raw_value, next) = match rest[value_start..].find(PARAMETER_CLOSE) {
            Some(end) => (
                &rest[value_start..value_start + end],
                value_start + end + PARAMETER_CLOSE.len(),
            ),
            None => (&rest[value_start..], rest.len()),
        };
        if !key.is_empty() {
            arguments.insert(key.to_string(), typed_value(raw_value, key, schema));
        }
        rest = &rest[next..];
    }
    Some(RecoveredToolCall {
        name: name.to_string(),
        arguments: Value::Object(arguments),
    })
}

/// The Qwen template writes each value on its own line; strip exactly that
/// framing (one leading and one trailing newline), not the value's own
/// whitespace.
fn strip_framing(raw: &str) -> &str {
    let raw = raw
        .strip_prefix("\r\n")
        .or_else(|| raw.strip_prefix('\n'))
        .unwrap_or(raw);
    raw.strip_suffix("\r\n")
        .or_else(|| raw.strip_suffix('\n'))
        .unwrap_or(raw)
}

fn declared_type<'a>(schema: Option<&'a Value>, key: &str) -> Option<&'a str> {
    let property = schema?.get("properties")?.get(key)?;
    match property.get("type")? {
        Value::String(kind) => Some(kind.as_str()),
        // `["string", "null"]` and friends: the first non-null type wins.
        Value::Array(kinds) => kinds
            .iter()
            .filter_map(Value::as_str)
            .find(|k| *k != "null"),
        _ => None,
    }
}

fn typed_value(raw: &str, key: &str, schema: Option<&Value>) -> Value {
    let text = strip_framing(raw);
    match declared_type(schema, key) {
        Some("string") => Value::String(text.to_string()),
        Some("integer" | "number" | "boolean" | "object" | "array" | "null") => {
            serde_json::from_str(text.trim()).unwrap_or_else(|_| Value::String(text.to_string()))
        }
        // No schema (or an untyped property): take structured JSON when it is
        // unambiguous, and keep everything else as the string it was written as.
        _ => match serde_json::from_str::<Value>(text.trim()) {
            Ok(value @ (Value::Object(_) | Value::Array(_) | Value::Bool(_))) => value,
            Ok(Value::Number(n)) if text.trim() == n.to_string() => Value::Number(n),
            _ => Value::String(text.to_string()),
        },
    }
}

#[cfg(test)]
#[path = "mlx_tool_text_tests.rs"]
mod tests;
