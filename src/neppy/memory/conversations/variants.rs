//! Answer variants: more than one assistant reply to the same user message.
//!
//! Regenerating keeps the previous answer instead of replacing it. The new one
//! is appended like any other message and tagged with the user message it
//! answers, so nothing is destroyed and the log stays append-only — the store
//! has no truncation, and giving it one to support a UI affordance would be a
//! poor trade.
//!
//! ## Why this is not only a display concern
//!
//! A turn resumes from the agent's own session transcript and falls back to
//! this log on cold boot (`web_chat::run_task`). Append a second answer and do
//! nothing else, and the model's history ends up holding both — every later
//! turn then reasons over a transcript where it answered the same question
//! twice, differently. [`active_messages`] is what the resume path seeds from,
//! so the model sees exactly one answer per question: the one on screen.
//!
//! Switching variants must also rebuild the session, or a cached one keeps
//! serving the context it was built with. That is what
//! [`selection_signature`] is for — it goes into the session fingerprint, the
//! same way a changed model or reasoning ask does.

use serde_json::Value;
use tinycortex::memory::conversations::ConversationMessage;

/// On an assistant message: the id of the user message it answers.
pub const VARIANT_OF: &str = "variantOf";
/// On an assistant message: the turn that produced it.
///
/// The unit of an answer is the turn, not the message: one turn can append
/// several assistant messages (a segmented reply), and all of them belong to
/// the same answer. Grouping by message instead would keep the last segment of
/// a regenerated answer and silently drop the rest.
pub const VARIANT_TURN: &str = "variantTurn";
/// On a user message: which of its answers is the chosen one, by turn id.
pub const ACTIVE_VARIANT: &str = "activeVariant";

fn metadata_str<'a>(metadata: &'a Value, key: &str) -> Option<&'a str> {
    metadata
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// The user message this assistant message answers, when it is a variant.
pub fn variant_of(message: &ConversationMessage) -> Option<&str> {
    metadata_str(&message.extra_metadata, VARIANT_OF)
}

/// The turn that produced this message, falling back to its own id.
///
/// The fallback makes a single-message answer its own turn, so a caller that
/// records no turn id still gets one coherent group per answer.
pub fn variant_turn(message: &ConversationMessage) -> &str {
    metadata_str(&message.extra_metadata, VARIANT_TURN).unwrap_or(message.id.as_str())
}

/// The variant this user message has selected, when one was chosen explicitly.
pub fn active_variant_choice(message: &ConversationMessage) -> Option<&str> {
    metadata_str(&message.extra_metadata, ACTIVE_VARIANT)
}

/// Every message answering `user_message_id`, in log order.
pub fn variant_messages<'a>(
    messages: &'a [ConversationMessage],
    user_message_id: &str,
) -> Vec<&'a ConversationMessage> {
    messages
        .iter()
        .filter(|message| variant_of(message) == Some(user_message_id))
        .collect()
}

/// The distinct answers to `user_message_id`, as turn ids in the order they
/// were produced. This is what the switcher counts.
pub fn variant_turns<'a>(
    messages: &'a [ConversationMessage],
    user_message_id: &str,
) -> Vec<&'a str> {
    let mut turns: Vec<&str> = Vec::new();
    for message in variant_messages(messages, user_message_id) {
        let turn = variant_turn(message);
        if !turns.contains(&turn) {
            turns.push(turn);
        }
    }
    turns
}

/// The id of the answer in effect for `user_message_id`.
///
/// An explicit choice wins, but only when it names a variant that is actually
/// there: a selection left behind by a deleted message must not blank the
/// answer. Otherwise the newest wins, which is what makes a fresh regenerate
/// the visible one without writing a selection first.
pub fn active_variant_id<'a>(
    messages: &'a [ConversationMessage],
    user_message_id: &str,
) -> Option<&'a str> {
    let turns = variant_turns(messages, user_message_id);
    if turns.is_empty() {
        return None;
    }
    let chosen = messages
        .iter()
        .find(|message| message.id == user_message_id)
        .and_then(active_variant_choice);
    if let Some(chosen) = chosen {
        if let Some(found) = turns.iter().find(|turn| **turn == chosen) {
            return Some(found);
        }
    }
    turns.last().copied()
}

/// The log with only the answer in effect kept for each question.
///
/// Messages that are not variants pass through untouched, so a thread from
/// before this existed reads exactly as it did.
pub fn active_messages(messages: &[ConversationMessage]) -> Vec<&ConversationMessage> {
    messages
        .iter()
        .filter(|message| match variant_of(message) {
            None => true,
            Some(group) => active_variant_id(messages, group) == Some(variant_turn(message)),
        })
        .collect()
}

/// A stable fingerprint of which answers are in effect.
///
/// Empty when a thread has no variants, so existing threads keep the
/// fingerprint they already had and nothing is needlessly rebuilt.
pub fn selection_signature(messages: &[ConversationMessage]) -> String {
    let mut groups: Vec<(&str, &str)> = messages
        .iter()
        .filter_map(|message| {
            let group = variant_of(message)?;
            Some((group, active_variant_id(messages, group)?))
        })
        .collect();
    groups.sort_unstable();
    groups.dedup();
    groups
        .into_iter()
        .map(|(group, active)| format!("{group}>{active}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// The user message's metadata with `activeVariant` set to `variant_id`.
///
/// Merges rather than replaces: the store's patch takes a whole metadata object,
/// so building a fresh one would drop whatever else the message carries.
pub fn metadata_with_selection(user_metadata: &Value, variant_id: &str) -> Value {
    let mut merged = match user_metadata {
        Value::Object(map) => map.clone(),
        // A message with no metadata object (or a malformed one) still gets a
        // selection rather than being left unswitchable.
        _ => serde_json::Map::new(),
    };
    merged.insert(
        ACTIVE_VARIANT.to_string(),
        Value::String(variant_id.to_string()),
    );
    Value::Object(merged)
}

/// Whether `variant_id` is really one of `user_message_id`'s answers.
///
/// Checked before a selection is written so a wrong id fails the call instead of
/// being stored and silently ignored at read time by [`active_variant_id`]'s
/// fallback.
pub fn is_variant_of(
    messages: &[ConversationMessage],
    user_message_id: &str,
    variant_id: &str,
) -> bool {
    variant_turns(messages, user_message_id).contains(&variant_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(id: &str, sender: &str, metadata: Value) -> ConversationMessage {
        ConversationMessage {
            id: id.to_string(),
            content: format!("content of {id}"),
            message_type: "text".to_string(),
            extra_metadata: metadata,
            sender: sender.to_string(),
            created_at: "2026-09-11T00:00:00Z".to_string(),
        }
    }

    /// One question, three answers, none chosen explicitly.
    fn three_answers() -> Vec<ConversationMessage> {
        vec![
            message("u1", "user", json!({})),
            message("a1", "assistant", json!({ VARIANT_OF: "u1" })),
            message("a2", "assistant", json!({ VARIANT_OF: "u1" })),
            message("a3", "assistant", json!({ VARIANT_OF: "u1" })),
        ]
    }

    #[test]
    fn a_thread_without_variants_is_unchanged() {
        let messages = vec![
            message("u1", "user", json!({})),
            message("a1", "assistant", json!({ "scope": "channel" })),
        ];

        let kept: Vec<&str> = active_messages(&messages)
            .into_iter()
            .map(|m| m.id.as_str())
            .collect();

        assert_eq!(kept, ["u1", "a1"]);
        assert_eq!(
            selection_signature(&messages),
            "",
            "no variants must not disturb an existing session fingerprint"
        );
    }

    #[test]
    fn the_newest_answer_is_in_effect_until_one_is_chosen() {
        let messages = three_answers();

        assert_eq!(active_variant_id(&messages, "u1"), Some("a3"));
        let kept: Vec<&str> = active_messages(&messages)
            .into_iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(
            kept,
            ["u1", "a3"],
            "the model must see one answer per question, not three"
        );
    }

    #[test]
    fn an_explicit_choice_wins() {
        let mut messages = three_answers();
        messages[0].extra_metadata = json!({ ACTIVE_VARIANT: "a1" });

        assert_eq!(active_variant_id(&messages, "u1"), Some("a1"));
        let kept: Vec<&str> = active_messages(&messages)
            .into_iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(kept, ["u1", "a1"]);
    }

    #[test]
    fn a_choice_naming_a_message_that_is_gone_falls_back_rather_than_blanking() {
        let mut messages = three_answers();
        messages[0].extra_metadata = json!({ ACTIVE_VARIANT: "a-deleted" });

        assert_eq!(
            active_variant_id(&messages, "u1"),
            Some("a3"),
            "a stale selection must not leave the question unanswered"
        );
    }

    #[test]
    fn variants_are_listed_in_the_order_they_were_produced() {
        let messages = three_answers();

        assert_eq!(
            variant_turns(&messages, "u1"),
            ["a1", "a2", "a3"],
            "the switcher counts in this order"
        );
    }

    #[test]
    fn a_segmented_answer_is_one_variant_and_survives_switching_whole() {
        // One turn, three assistant messages. Counting messages would show
        // "3 answers" and switching would keep only the last segment.
        let messages = vec![
            message("u1", "user", json!({})),
            message(
                "s1",
                "assistant",
                json!({ VARIANT_OF: "u1", VARIANT_TURN: "turn-1" }),
            ),
            message(
                "s2",
                "assistant",
                json!({ VARIANT_OF: "u1", VARIANT_TURN: "turn-1" }),
            ),
            message(
                "s3",
                "assistant",
                json!({ VARIANT_OF: "u1", VARIANT_TURN: "turn-1" }),
            ),
            message(
                "r1",
                "assistant",
                json!({ VARIANT_OF: "u1", VARIANT_TURN: "turn-2" }),
            ),
        ];

        assert_eq!(variant_turns(&messages, "u1"), ["turn-1", "turn-2"]);
        assert_eq!(active_variant_id(&messages, "u1"), Some("turn-2"));

        let mut chose_first = messages.clone();
        chose_first[0].extra_metadata = json!({ ACTIVE_VARIANT: "turn-1" });
        let kept: Vec<&str> = active_messages(&chose_first)
            .into_iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(
            kept,
            ["u1", "s1", "s2", "s3"],
            "every segment of the chosen answer must survive"
        );
    }

    #[test]
    fn switching_the_answer_changes_the_signature() {
        let messages = three_answers();
        let mut switched = three_answers();
        switched[0].extra_metadata = json!({ ACTIVE_VARIANT: "a1" });

        assert_ne!(
            selection_signature(&messages),
            selection_signature(&switched),
            "a cached session would otherwise keep serving the old answer's context"
        );
    }

    #[test]
    fn the_signature_is_stable_for_the_same_selection() {
        assert_eq!(
            selection_signature(&three_answers()),
            selection_signature(&three_answers()),
            "an unchanged selection must keep hitting the cached session"
        );
    }

    #[test]
    fn selecting_an_answer_keeps_the_rest_of_the_metadata() {
        let existing = json!({ "scope": "channel", "model": "ornith" });

        let merged = metadata_with_selection(&existing, "a2");

        assert_eq!(merged["scope"], "channel");
        assert_eq!(merged["model"], "ornith");
        assert_eq!(merged[ACTIVE_VARIANT], "a2");
    }

    #[test]
    fn a_message_with_no_metadata_object_is_still_switchable() {
        let merged = metadata_with_selection(&Value::Null, "a2");
        assert_eq!(merged[ACTIVE_VARIANT], "a2");
    }

    #[test]
    fn a_selection_is_validated_against_the_real_answers() {
        let messages = three_answers();

        assert!(is_variant_of(&messages, "u1", "a2"));
        assert!(
            !is_variant_of(&messages, "u1", "b1"),
            "a wrong id must fail the call, not be stored and ignored later"
        );
        assert!(!is_variant_of(&messages, "u2", "a2"));
    }

    #[test]
    fn several_questions_each_keep_their_own_answer() {
        let messages = vec![
            message("u1", "user", json!({ ACTIVE_VARIANT: "a1" })),
            message("a1", "assistant", json!({ VARIANT_OF: "u1" })),
            message("a2", "assistant", json!({ VARIANT_OF: "u1" })),
            message("u2", "user", json!({})),
            message("b1", "assistant", json!({ VARIANT_OF: "u2" })),
            message("b2", "assistant", json!({ VARIANT_OF: "u2" })),
        ];

        let kept: Vec<&str> = active_messages(&messages)
            .into_iter()
            .map(|m| m.id.as_str())
            .collect();

        assert_eq!(kept, ["u1", "a1", "u2", "b2"]);
    }
}
