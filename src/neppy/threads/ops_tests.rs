//! Shape + validation tests for the pure, pre-IO helpers used by the
//! threads RPC surface. Every test here avoids disk, network, and
//! provider calls — they pin the behaviour of the branches that all of
//! the async `ops::*` entry points rely on.
use super::*;
// Re-imported here rather than through `ops`: `ops` itself no longer names
// these, so importing them there would be an unused import in a non-test build.
use crate::neppy::threads::title::{build_title_prompt, THREAD_TITLE_SYSTEM_PROMPT};
use crate::neppy::threads::turn_state::{
    self, ClearTurnStateRequest, GetTurnStateRequest, TurnState,
};
use crate::neppy::threads::ThreadsError;
use serde_json::{json, Value};
use std::ffi::OsString;
use std::path::Path;
use tinycortex::memory::conversations as conversations_store;

struct EnvVarGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, value: &Path) -> Self {
        let old = crate::neppy::util::env::var_os(key);
        std::env::set_var(key, value.as_os_str());
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => crate::neppy::util::env::remove_var(self.key),
        }
    }
}

// ── request_id ────────────────────────────────────────────────

#[test]
fn request_id_is_a_non_empty_uuid_and_fresh_per_call() {
    let a = request_id();
    let b = request_id();
    assert!(!a.is_empty());
    // v4 UUID canonical form: 36 chars with 4 hyphens.
    assert_eq!(a.len(), 36);
    assert_eq!(a.chars().filter(|c| *c == '-').count(), 4);
    // Two calls must not collide — catches accidental caching.
    assert_ne!(a, b);
}

// ── counts ────────────────────────────────────────────────────

#[test]
fn counts_materialises_entries_as_owned_string_keys() {
    let map = counts([("num_threads", 3), ("num_messages", 7)]);
    assert_eq!(map.get("num_threads"), Some(&3));
    assert_eq!(map.get("num_messages"), Some(&7));
    assert_eq!(map.len(), 2);
}

#[test]
fn counts_empty_iter_yields_empty_map() {
    let map = counts([]);
    assert!(map.is_empty());
}

// NOTE: the title_log_fingerprint / collapse_whitespace copies were removed
// here (plan.md §2.1) — threads/title.rs (the owning module) already covers
// these functions with equivalent cases; the lowercase-hex assertion was
// folded into title.rs so no coverage was lost.

// ── build_title_prompt ────────────────────────────────────────

// ── build_title_request: the outgoing wire model (#5637) ──────

#[test]
fn build_title_request_sends_no_per_request_model_override() {
    // The regression guard for #5637. `ModelRequest::model` is a per-request
    // override that the managed backend resolves verbatim, so anything set
    // here replaces the model the `summarization` provider already resolved.
    // It used to be `"hint:summarize"` — a string no hint-alias table in the
    // tree defines — which reached the backend as a literal model id and was
    // answered `400 Model 'hint:summarize' is not available` on every call,
    // for four months.
    //
    // Asserted as `None` rather than as some expected id on purpose: leaving
    // it unset is the only form that is correct for every branch of
    // `create_chat_model` (managed backend, Claude Agent SDK, Claude Code,
    // local runtime, BYOK cloud slug). Pinning a concrete managed tier here
    // would fix the managed route and break the other four.
    let request = crate::neppy::threads::title::build_title_request("hi there", "hello back");

    assert!(
        request.model.is_none(),
        "the title request must carry no model override so the resolved \
         `summarization` provider decides; got {:?}",
        request.model,
    );
}

#[test]
fn build_title_request_carries_the_system_prompt_and_rendered_user_prompt() {
    // Pins the rest of the request so the `model` assertion above cannot be
    // satisfied by an empty or malformed request.
    use tinyagents::harness::message::Message;

    let request = crate::neppy::threads::title::build_title_request("hi there", "hello back");

    assert_eq!(request.messages.len(), 2, "system + user");
    assert!(
        matches!(request.messages[0], Message::System(_)),
        "first message must be the system prompt, got {:?}",
        request.messages[0],
    );
    assert_eq!(request.messages[0].text(), THREAD_TITLE_SYSTEM_PROMPT,);
    assert!(
        matches!(request.messages[1], Message::User(_)),
        "second message must be the user prompt, got {:?}",
        request.messages[1],
    );
    assert_eq!(
        request.messages[1].text(),
        build_title_prompt("hi there", "hello back"),
    );
}

#[test]
fn build_title_prompt_renders_user_and_assistant_sections_in_order() {
    let prompt = build_title_prompt("hi there", "hello back");
    assert_eq!(
        prompt,
        "First user message:\nhi there\n\nAssistant reply:\nhello back\n\nReturn the best thread name."
    );
}

// NOTE: the sanitize_generated_title / title_from_user_message copies were
// removed here (plan.md §2.1) — threads/title.rs (the owning module) already
// covers these functions with equivalent cases (title shaping, filler removal,
// first non-empty line, empty→None, and the char-safe length ceiling incl.
// multibyte input).

// ── is_auto_generated_thread_title ────────────────────────────

#[test]
fn is_auto_generated_thread_title_accepts_canonical_new_chat_format() {
    // Parser locks the format produced by `thread_create_new`:
    // "Chat <Mon> <day> <H:MM> AM|PM".
    assert!(is_auto_generated_thread_title("Chat Jan 1 1:00 AM"));
    assert!(is_auto_generated_thread_title("Chat Dec 31 12:59 PM"));
}

#[test]
fn is_auto_generated_thread_title_tolerates_surrounding_whitespace() {
    // Input is trimmed before parsing — storage layers may round-trip
    // titles with stray whitespace.
    assert!(is_auto_generated_thread_title("  Chat Jan 1 1:00 AM  "));
}

#[test]
fn is_auto_generated_thread_title_rejects_user_edited_titles() {
    // Any freeform user title must fall through to the "not a
    // placeholder" branch so we never overwrite user-authored names.
    assert!(!is_auto_generated_thread_title("My custom title"));
    assert!(!is_auto_generated_thread_title("Trip planning"));
}

#[test]
fn is_auto_generated_thread_title_rejects_short_strings() {
    // Hard `bytes.len() < 16` guard — locks in the minimum shape so
    // we never enter the parser with too-small input.
    assert!(!is_auto_generated_thread_title(""));
    assert!(!is_auto_generated_thread_title("Chat"));
    assert!(!is_auto_generated_thread_title("Chat Jan 1"));
}

#[test]
fn is_auto_generated_thread_title_rejects_non_alpha_month() {
    // Month abbreviation must be 3 ASCII alpha chars.
    assert!(!is_auto_generated_thread_title("Chat 123 1 1:00 AM"));
}

#[test]
fn is_auto_generated_thread_title_rejects_long_month_name() {
    // "January 1 1:00 AM" — after "Chat ", bytes[8] is 'u' not ' '.
    assert!(!is_auto_generated_thread_title("Chat January 1 1:00 AM"));
}

#[test]
fn is_auto_generated_thread_title_rejects_three_digit_day() {
    // day: 1–2 ASCII digits; idx-day_start>2 rejects.
    assert!(!is_auto_generated_thread_title("Chat Jan 100 1:00 AM"));
}

#[test]
fn is_auto_generated_thread_title_rejects_missing_colon() {
    // 3-digit hour consumes through the position the `:` must occupy.
    assert!(!is_auto_generated_thread_title("Chat Jan 1 100 AM"));
}

#[test]
fn is_auto_generated_thread_title_rejects_lowercase_meridiem() {
    // Parser only accepts "AM" | "PM" (not "am"/"pm") so pattern stays
    // tied to the producer in `thread_create_new`.
    assert!(!is_auto_generated_thread_title("Chat Jan 1 1:00 am"));
}

#[test]
fn is_auto_generated_thread_title_rejects_missing_space_before_meridiem() {
    // The `bytes[idx + 2] != b' '` guard must reject "1:00AM" (no space).
    assert!(!is_auto_generated_thread_title("Chat Jan 1 1:00AM"));
}

// ── envelope ──────────────────────────────────────────────────

#[test]
fn envelope_sets_data_and_propagates_counts_and_pagination() {
    let pagination = PaginationMeta {
        limit: 10,
        offset: 0,
        count: 7,
    };
    let counts_map = counts([("num_messages", 7)]);
    let out = envelope(
        json!({"v": 42}),
        Some(counts_map.clone()),
        Some(pagination.clone()),
    );
    let env = &out.value;
    assert_eq!(env.data.as_ref().unwrap()["v"], json!(42));
    assert!(env.error.is_none());
    assert!(!env.meta.request_id.is_empty());
    assert_eq!(env.meta.counts.as_ref().unwrap(), &counts_map);
    let pag = env.meta.pagination.as_ref().unwrap();
    assert_eq!(pag.limit, pagination.limit);
    assert_eq!(pag.count, pagination.count);
    assert_eq!(pag.offset, pagination.offset);
    // No implicit latency/cached info — the envelope helper keeps
    // optional fields unset so callers opt in explicitly.
    assert!(env.meta.latency_seconds.is_none());
    assert!(env.meta.cached.is_none());
    // No logs are attached by default.
    assert!(out.logs.is_empty());
}

#[test]
fn envelope_omits_counts_and_pagination_when_not_provided() {
    let out = envelope(json!(null), None, None);
    assert!(out.value.meta.counts.is_none());
    assert!(out.value.meta.pagination.is_none());
}

#[test]
fn envelope_generates_unique_request_ids_per_call() {
    // request_id uniqueness matters for client-side correlation of
    // overlapping threads-API calls. Lock it in.
    let a = envelope(json!({}), None, None);
    let b = envelope(json!({}), None, None);
    assert_ne!(a.value.meta.request_id, b.value.meta.request_id);
}

// ── thread_to_summary / message_to_record / record_to_message ─

fn sample_thread() -> ConversationThread {
    ConversationThread {
        id: "t-1".into(),
        title: "My thread".into(),
        chat_id: Some(42),
        is_active: true,
        message_count: 5,
        last_message_at: "2026-01-01T00:00:00Z".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        parent_thread_id: None,
        labels: vec!["general".to_string()],
        personality_id: None,
    }
}

fn sample_message() -> ConversationMessage {
    ConversationMessage {
        id: "m-1".into(),
        content: "hi".into(),
        message_type: "text".into(),
        extra_metadata: json!({"k": "v"}),
        sender: "user".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

#[test]
fn thread_to_summary_preserves_all_fields() {
    let summary = thread_to_summary(sample_thread());
    assert_eq!(summary.id, "t-1");
    assert_eq!(summary.title, "My thread");
    assert_eq!(summary.chat_id, Some(42));
    assert!(summary.is_active);
    assert_eq!(summary.message_count, 5);
    assert_eq!(summary.last_message_at, "2026-01-01T00:00:00Z");
    assert_eq!(summary.created_at, "2026-01-01T00:00:00Z");
    assert_eq!(summary.labels, vec!["general".to_string()]);
}

#[test]
fn message_to_record_and_back_is_lossless() {
    let msg = sample_message();
    let record = message_to_record(msg.clone());
    assert_eq!(record.id, msg.id);
    assert_eq!(record.content, msg.content);
    assert_eq!(record.message_type, msg.message_type);
    assert_eq!(record.extra_metadata, msg.extra_metadata);
    assert_eq!(record.sender, msg.sender);
    assert_eq!(record.created_at, msg.created_at);

    let round_tripped = record_to_message(record);
    assert_eq!(round_tripped, msg);
}

#[test]
fn record_to_message_preserves_null_extra_metadata() {
    // Default Value::Null must pass through untouched so the downstream
    // storage layer sees the same "no metadata" signal it produced.
    let rec = ConversationMessageRecord {
        id: "m-2".into(),
        content: "x".into(),
        message_type: "text".into(),
        extra_metadata: Value::Null,
        sender: "agent".into(),
        created_at: "2026-01-02T00:00:00Z".into(),
    };
    let msg = record_to_message(rec);
    assert_eq!(msg.extra_metadata, Value::Null);
    assert_eq!(msg.sender, "agent");
}

// ── Title constants ───────────────────────────────────────────

#[test]
fn title_system_prompt_constrains_model_output_shape() {
    // The system prompt is shipped verbatim to the provider. Locking in the
    // length clause catches accidental edits that would let the model emit a
    // sentence-shaped title that `sanitize_generated_title` would then have to
    // rewrite silently.
    assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("at most 3 words"));
    assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("No quotes"));
    assert!(THREAD_TITLE_SYSTEM_PROMPT.contains("No markdown"));
}

#[test]
fn title_log_prefix_is_grep_friendly_and_stable() {
    // The `[threads:title]` prefix is what CLAUDE.md's "debug logging"
    // rule asks contributors to grep for when debugging. It is part
    // of the observable contract — lock it down.
    assert_eq!(THREAD_TITLE_LOG_PREFIX, "[threads:title]");
}

#[tokio::test]
async fn message_append_returns_typed_not_found_for_stale_thread() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "thread-missing";

    let err = message_append(AppendConversationMessageRequest {
        thread_id: thread_id.to_string(),
        message: ConversationMessageRecord {
            id: "msg-1".to_string(),
            content: "hello".to_string(),
            message_type: "text".to_string(),
            extra_metadata: Value::Null,
            sender: "user".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        },
    })
    .await
    .expect_err("missing thread must return a typed not-found error");

    assert_eq!(
        err,
        ThreadsError::NotFound {
            thread_id: thread_id.to_string()
        }
    );
    assert_eq!(err.to_string(), "thread thread-missing not found");
}

#[tokio::test]
async fn generate_title_returns_typed_not_found_for_stale_thread() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "thread-missing";

    let err = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        assistant_message: None,
    })
    .await
    .expect_err("missing thread must return a typed not-found error");

    assert_eq!(
        err,
        ThreadsError::NotFound {
            thread_id: thread_id.to_string()
        }
    );
    assert_eq!(err.to_string(), "thread thread-missing not found");
}

async fn create_thread_with_title(_workspace: &tempfile::TempDir, thread_id: &str, title: &str) {
    let dir = crate::neppy::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;
    conversations_store::ensure_thread(
        dir,
        CreateConversationThread {
            id: thread_id.to_string(),
            title: title.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .expect("ensure thread");
}

#[tokio::test]
async fn generate_title_leaves_custom_title_unchanged() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "thread-custom";
    create_thread_with_title(&workspace, thread_id, "Already named").await;
    let dir = crate::neppy::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;
    conversations_store::append_message(
        dir,
        thread_id,
        ConversationMessage {
            id: "msg-1".into(),
            content: "Please summarize my notes".into(),
            message_type: "text".into(),
            extra_metadata: Value::Null,
            sender: "user".into(),
            created_at: "2026-01-01T00:01:00Z".into(),
        },
    )
    .unwrap();

    let outcome = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        assistant_message: None,
    })
    .await
    .expect("generate title");

    assert_eq!(
        outcome.value.data.as_ref().unwrap().title,
        "Already named",
        "non-placeholder titles must not be replaced"
    );
}

#[tokio::test]
async fn generate_title_returns_existing_title_when_no_user_message_exists() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "thread-no-user";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;

    let outcome = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        assistant_message: Some("assistant reply".into()),
    })
    .await
    .expect("generate title");

    assert_eq!(
        outcome.value.data.as_ref().unwrap().title,
        "Chat Jan 1 1:00 AM"
    );
}

#[tokio::test]
async fn generate_title_falls_back_to_first_user_message_when_assistant_missing() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "thread-fallback";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;
    let dir = crate::neppy::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;
    let user_message = "Please summarize the latest five email threads for me.";
    conversations_store::append_message(
        dir,
        thread_id,
        ConversationMessage {
            id: "msg-1".into(),
            content: user_message.into(),
            message_type: "text".into(),
            extra_metadata: Value::Null,
            sender: "user".into(),
            created_at: "2026-01-01T00:01:00Z".into(),
        },
    )
    .unwrap();

    let outcome = thread_generate_title(GenerateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        assistant_message: None,
    })
    .await
    .expect("generate title");

    assert_eq!(
        outcome.value.data.as_ref().unwrap().title,
        title_from_user_message(user_message).unwrap()
    );
}

#[tokio::test]
async fn thread_delete_removes_persisted_turn_state_snapshot() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "thread-delete";
    create_thread_with_title(&workspace, thread_id, "Chat Jan 1 1:00 AM").await;
    let dir = crate::neppy::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;

    let snapshot = TurnState::started(thread_id, "req-1", 4, "2026-01-01T00:00:00Z");
    turn_state::store::put(dir.clone(), &snapshot).expect("put snapshot");
    assert!(turn_state::store::get(dir, thread_id).unwrap().is_some());

    // Queue a finished background sub-agent result for this thread; deleting the
    // thread must discard it so it's never delivered into a dead thread.
    use crate::neppy::agent::orchestration::background_completions as bg;
    bg::record_completion(
        "sess-del",
        "sub-del-1",
        "researcher",
        "result",
        Some(thread_id.to_string()),
    );
    assert_eq!(bg::pending_count("sess-del"), 1);

    thread_delete(DeleteConversationThreadRequest {
        thread_id: thread_id.to_string(),
        deleted_at: "2026-01-01T00:02:00Z".into(),
    })
    .await
    .expect("delete thread");

    assert_eq!(
        bg::pending_count("sess-del"),
        0,
        "queued completion for the deleted thread should be discarded"
    );

    let turn_state = turn_state_get(GetTurnStateRequest {
        thread_id: thread_id.to_string(),
    })
    .await
    .expect("turn_state_get");
    assert!(turn_state.value.data.unwrap().turn_state.is_none());
}

#[tokio::test]
async fn threads_purge_removes_valid_and_corrupted_turn_state_files() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    create_thread_with_title(&workspace, "thread-a", "Chat Jan 1 1:00 AM").await;
    create_thread_with_title(&workspace, "thread-b", "Chat Jan 1 1:01 AM").await;
    let dir = crate::neppy::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;

    let snapshot = TurnState::started("thread-a", "req-1", 4, "2026-01-01T00:00:00Z");
    turn_state::store::put(dir.clone(), &snapshot).expect("put snapshot");

    let turn_state_dir = dir.join("memory").join("conversations").join("turn_states");
    std::fs::create_dir_all(&turn_state_dir).unwrap();
    std::fs::write(
        turn_state_dir.join("corrupted.json"),
        "{ definitely not json",
    )
    .unwrap();

    // Queue background sub-agent results across sessions; a full purge must wipe
    // them all since no parent thread survives.
    use crate::neppy::agent::orchestration::background_completions as bg;
    bg::record_completion(
        "sess-p1",
        "sub-p1",
        "researcher",
        "x",
        Some("thread-a".into()),
    );
    bg::record_completion(
        "sess-p2",
        "sub-p2",
        "researcher",
        "y",
        Some("thread-b".into()),
    );

    threads_purge(EmptyRequest {})
        .await
        .expect("purge threads should also clear snapshots");

    assert!(!bg::has_pending("sess-p1"));
    assert!(!bg::has_pending("sess-p2"));

    if turn_state_dir.exists() {
        let remaining_json: Vec<_> = std::fs::read_dir(&turn_state_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .collect();
        assert!(remaining_json.is_empty(), "expected no snapshot json files");
    }
}

#[tokio::test]
async fn turn_state_clear_reports_false_when_snapshot_is_absent() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());

    let outcome = turn_state_clear(ClearTurnStateRequest {
        thread_id: "missing-thread".into(),
    })
    .await
    .expect("turn_state_clear");

    assert!(!outcome.value.data.unwrap().cleared);
}

// ── thread_update_title ───────────────────────────────────────

#[tokio::test]
async fn thread_update_title_rejects_empty_title() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());

    let err = thread_update_title(crate::neppy::memory::UpdateConversationThreadTitleRequest {
        thread_id: "t-1".to_string(),
        title: "".to_string(),
    })
    .await
    .expect_err("empty title must be rejected");

    assert!(
        err.contains("must not be empty"),
        "expected empty-title error, got: {err}"
    );
}

#[tokio::test]
async fn thread_update_title_rejects_whitespace_only_title() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());

    let err = thread_update_title(crate::neppy::memory::UpdateConversationThreadTitleRequest {
        thread_id: "t-1".to_string(),
        title: "   ".to_string(),
    })
    .await
    .expect_err("whitespace-only title must be rejected");

    assert!(
        err.contains("must not be empty"),
        "expected empty-title error, got: {err}"
    );
}

#[tokio::test]
async fn thread_update_title_persists_new_title() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());

    let thread_id = "t-title";
    create_thread_with_title(&workspace, thread_id, "Original title").await;

    let outcome = thread_update_title(crate::neppy::memory::UpdateConversationThreadTitleRequest {
        thread_id: thread_id.to_string(),
        title: "  Invoice follow-up  ".to_string(),
    })
    .await
    .expect("thread_update_title");

    let summary = outcome.value.data.expect("data envelope");
    assert_eq!(
        summary.title, "Invoice follow-up",
        "title must be trimmed and persisted"
    );
    assert_eq!(summary.id, thread_id);
}

#[tokio::test]
async fn thread_update_title_returns_error_for_missing_thread() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());

    let err = thread_update_title(crate::neppy::memory::UpdateConversationThreadTitleRequest {
        thread_id: "nonexistent-thread".to_string(),
        title: "New title".to_string(),
    })
    .await
    .expect_err("missing thread must return an error");

    assert!(
        err.contains("update title"),
        "error must describe the update-title failure, got: {err}"
    );
}

/// Review follow-up on #5282: moving the conversation store onto the blocking
/// pool put an `.await` between a destructive store mutation and the cleanup
/// that has to follow it (web-session invalidation, sub-agent cancellation,
/// turn-snapshot deletion).
///
/// `spawn_blocking` work is never cancelled by dropping its `JoinHandle`, so a
/// caller that goes away in that window — client disconnect, RPC timeout —
/// used to leave the thread deleted from the store while every one of those
/// cleanup steps was skipped. `run_to_completion` owns the mutation *and* the
/// cleanup in one spawned task, so the tail runs regardless of the caller.
#[tokio::test]
async fn run_to_completion_runs_the_tail_after_the_caller_is_dropped() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let tail_ran = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&tail_ran);
    let (release, parked) = tokio::sync::oneshot::channel::<()>();

    let mut call = Box::pin(run_to_completion("test_destructive_op", async move {
        // Stands in for the blocking store mutation still in progress.
        let _ = parked.await;
        // Stands in for the cleanup that must never be skipped.
        flag.store(true, Ordering::SeqCst);
        Ok::<(), String>(())
    }));

    // Poll once so the inner task is spawned, then abandon the caller — exactly
    // what dropping the RPC future does.
    tokio::select! {
        biased;
        _ = &mut call => panic!("the inner task should still be parked"),
        _ = tokio::task::yield_now() => {}
    }
    drop(call);

    assert!(
        !tail_ran.load(Ordering::SeqCst),
        "the tail must not have run before the mutation completed"
    );

    // The store mutation finishes after the caller is already gone.
    release.send(()).expect("release the parked task");

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !tail_ran.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cleanup must still run after the caller's future was dropped");
}

// ── begin answer variant (regenerate) ─────────────────────────
use crate::neppy::memory::conversations::variants;

/// Appends a message through the real op so the store sees what the app writes.
async fn append(thread_id: &str, id: &str, sender: &str, content: &str, metadata: Value) {
    message_append(AppendConversationMessageRequest {
        thread_id: thread_id.to_string(),
        message: ConversationMessageRecord {
            id: id.to_string(),
            content: content.to_string(),
            message_type: "text".to_string(),
            extra_metadata: metadata,
            sender: sender.to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        },
    })
    .await
    .expect("append");
}

async fn stored(thread_id: &str) -> Vec<tinycortex::memory::conversations::ConversationMessage> {
    let dir = crate::neppy::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir;
    crate::neppy::memory::conversations::blocking::get_messages(dir, thread_id.to_string())
        .await
        .expect("messages")
}

/// The answer a question already has becomes variant one, so the reply about to
/// arrive replaces it on screen instead of stacking under it.
#[tokio::test]
async fn beginning_a_variant_tags_the_existing_answer() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "t-regen";
    create_thread_with_title(&workspace, thread_id, "Regenerate").await;
    append(thread_id, "u1", "user", "why?", Value::Null).await;
    // A segmented answer: two messages, one answer.
    append(
        thread_id,
        "a1",
        "agent",
        "because",
        json!({ "requestId": "req-1" }),
    )
    .await;
    append(thread_id, "a1b", "agent", "and also", Value::Null).await;

    let outcome = message_begin_answer_variant(BeginAnswerVariantRequest {
        thread_id: thread_id.to_string(),
        message_id: "u1".to_string(),
    })
    .await
    .expect("begin variant");

    let data = outcome.value.data.expect("envelope carries the response");
    assert_eq!(data.variant_turn_id.as_deref(), Some("a1"));
    assert_eq!(data.tagged, 2);
    // One answer on disk plus the one about to be produced.
    assert_eq!(data.variant_count, 2);

    let messages = stored(thread_id).await;
    for id in ["a1", "a1b"] {
        let message = messages.iter().find(|m| m.id == id).expect("answer kept");
        assert_eq!(variants::variant_of(message), Some("u1"));
        // Both segments group under the first, so switching moves the whole
        // answer rather than its last paragraph.
        assert_eq!(variants::variant_turn(message), "a1");
    }
    // The merge kept what the reply already carried.
    assert_eq!(messages[1].extra_metadata["requestId"], "req-1");
}

/// Running twice must not split one answer across two turns — a retried send
/// would otherwise invent a variant nobody asked for.
#[tokio::test]
async fn beginning_a_variant_twice_changes_nothing_the_second_time() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "t-regen-twice";
    create_thread_with_title(&workspace, thread_id, "Regenerate").await;
    append(thread_id, "u1", "user", "why?", Value::Null).await;
    append(thread_id, "a1", "agent", "because", Value::Null).await;

    let request = || BeginAnswerVariantRequest {
        thread_id: thread_id.to_string(),
        message_id: "u1".to_string(),
    };
    message_begin_answer_variant(request())
        .await
        .expect("first");
    let second = message_begin_answer_variant(request())
        .await
        .expect("second");

    let data = second.value.data.expect("envelope carries the response");
    assert_eq!(data.tagged, 0);
    assert_eq!(data.variant_turn_id, None);
    assert_eq!(data.variant_count, 2);
    let messages = stored(thread_id).await;
    assert_eq!(variants::variant_turns(&messages, "u1"), ["a1"]);
}

/// Regenerating after switching back must clear the stored choice, or the new
/// answer lands already superseded and invisible.
#[tokio::test]
async fn beginning_a_variant_clears_an_earlier_choice() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "t-regen-choice";
    create_thread_with_title(&workspace, thread_id, "Regenerate").await;
    append(thread_id, "u1", "user", "why?", Value::Null).await;
    append(
        thread_id,
        "a1",
        "agent",
        "first",
        json!({ "variantOf": "u1", "variantTurn": "a1" }),
    )
    .await;
    append(
        thread_id,
        "a2",
        "agent",
        "second",
        json!({ "variantOf": "u1", "variantTurn": "a2" }),
    )
    .await;
    // The user switched back to the first answer.
    message_set_active_variant(SetActiveVariantRequest {
        thread_id: thread_id.to_string(),
        message_id: "u1".to_string(),
        variant_id: "a1".to_string(),
    })
    .await
    .expect("switch back");

    message_begin_answer_variant(BeginAnswerVariantRequest {
        thread_id: thread_id.to_string(),
        message_id: "u1".to_string(),
    })
    .await
    .expect("begin variant");

    let messages = stored(thread_id).await;
    let question = messages.iter().find(|m| m.id == "u1").expect("question");
    assert_eq!(variants::active_variant_choice(question), None);
    // Newest wins again, so the answer about to arrive will be the visible one.
    assert_eq!(variants::active_variant_id(&messages, "u1"), Some("a2"));
}

/// A message id that is not a question fails the call rather than writing tags
/// onto whatever happened to follow it.
#[tokio::test]
async fn beginning_a_variant_on_something_that_is_not_a_question_fails() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _workspace_guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    let thread_id = "t-regen-bad";
    create_thread_with_title(&workspace, thread_id, "Regenerate").await;
    append(thread_id, "u1", "user", "why?", Value::Null).await;
    append(thread_id, "a1", "agent", "because", Value::Null).await;

    let err = message_begin_answer_variant(BeginAnswerVariantRequest {
        thread_id: thread_id.to_string(),
        message_id: "a1".to_string(),
    })
    .await
    .expect_err("an assistant message is not a question");

    assert!(err.contains("is not a question"), "unexpected error: {err}");
    let messages = stored(thread_id).await;
    assert!(variants::variant_of(&messages[1]).is_none());
}

// ── thread operating mode (chat | orchestration) ──────────────────────────

async fn mode_test_workspace_dir() -> std::path::PathBuf {
    crate::neppy::config::Config::load_or_init()
        .await
        .expect("load config")
        .workspace_dir
}

fn thread_from(
    outcome: RpcOutcome<ApiEnvelope<ConversationThreadSummary>>,
) -> ConversationThreadSummary {
    outcome.value.data.expect("envelope data")
}

#[tokio::test]
async fn new_and_legacy_threads_default_to_chat_mode() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());

    let created = thread_from(
        thread_create_new(CreateConversationThreadRequest {
            labels: None,
            personality_id: None,
        })
        .await
        .expect("create"),
    );
    assert_eq!(created.mode, "chat");

    // A thread stored before the field existed has no mode label at all.
    create_thread_with_title(&workspace, "legacy-thread", "Old chat").await;
    let listed = threads_list(EmptyRequest {}).await.expect("list");
    let all = listed.value.data.expect("data").threads;
    let legacy = all
        .iter()
        .find(|t| t.id == "legacy-thread")
        .expect("legacy");
    assert_eq!(legacy.mode, "chat");
    assert!(all.iter().all(|t| t.mode == "chat"));

    let dir = mode_test_workspace_dir().await;
    assert_eq!(
        thread_mode_for(&dir, "legacy-thread").await,
        ThreadMode::Chat
    );
    // An unknown thread reads as the default rather than failing a turn.
    assert_eq!(thread_mode_for(&dir, "nope").await, ThreadMode::Chat);
}

#[tokio::test]
async fn set_mode_persists_is_readable_on_list_and_keeps_labels_clean() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    create_thread_with_title(&workspace, "t1", "Work").await;

    let result = thread_set_mode(SetThreadModeRequest {
        thread_id: "t1".into(),
        mode: "orchestration".into(),
        source: None,
    })
    .await
    .expect("set mode")
    .value
    .data
    .expect("data");
    assert!(result.changed);
    assert_eq!(result.previous_mode, "chat");
    assert_eq!(result.thread.mode, "orchestration");
    // The reserved label never reaches the client.
    assert!(result.thread.labels.iter().all(|l| !l.starts_with("mode:")));

    // Persisted: a fresh read sees it, and the on-disk label is the reserved one.
    let listed = threads_list(EmptyRequest {}).await.expect("list");
    let t1 = listed
        .value
        .data
        .expect("data")
        .threads
        .into_iter()
        .find(|t| t.id == "t1")
        .expect("t1");
    assert_eq!(t1.mode, "orchestration");
    let dir = mode_test_workspace_dir().await;
    assert_eq!(thread_mode_for(&dir, "t1").await, ThreadMode::Orchestration);
    let raw = conversations_store::list_threads(dir.clone()).expect("raw list");
    let raw_t1 = raw.iter().find(|t| t.id == "t1").expect("raw t1");
    assert!(raw_t1.labels.iter().any(|l| l == "mode:orchestration"));

    // Setting the same mode again is a no-op.
    let again = thread_set_mode(SetThreadModeRequest {
        thread_id: "t1".into(),
        mode: " Orchestration ".into(),
        source: Some("test".into()),
    })
    .await
    .expect("set again")
    .value
    .data
    .expect("data");
    assert!(!again.changed);

    // Switching back restores chat with no leftover label, same thread.
    let back = thread_set_mode(SetThreadModeRequest {
        thread_id: "t1".into(),
        mode: "chat".into(),
        source: None,
    })
    .await
    .expect("back")
    .value
    .data
    .expect("data");
    assert!(back.changed);
    assert_eq!(back.previous_mode, "orchestration");
    assert_eq!(back.thread.mode, "chat");
    assert_eq!(back.thread.id, "t1");
    let raw = conversations_store::list_threads(dir).expect("raw list");
    assert!(raw
        .iter()
        .find(|t| t.id == "t1")
        .expect("t1")
        .labels
        .iter()
        .all(|l| !l.starts_with("mode:")));
}

#[tokio::test]
async fn set_mode_rejects_unknown_modes_and_unknown_threads() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    create_thread_with_title(&workspace, "t1", "Work").await;

    let bad = thread_set_mode(SetThreadModeRequest {
        thread_id: "t1".into(),
        mode: "pet".into(),
        source: None,
    })
    .await
    .expect_err("unknown mode");
    assert!(bad.to_string().contains("unknown thread mode"), "{bad}");

    let missing = thread_set_mode(SetThreadModeRequest {
        thread_id: "ghost".into(),
        mode: "chat".into(),
        source: None,
    })
    .await
    .expect_err("unknown thread");
    assert_eq!(missing, ThreadsError::not_found("ghost"));
}

#[tokio::test]
async fn update_labels_cannot_flip_the_mode() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    create_thread_with_title(&workspace, "t1", "Work").await;
    thread_set_mode(SetThreadModeRequest {
        thread_id: "t1".into(),
        mode: "orchestration".into(),
        source: None,
    })
    .await
    .expect("set mode");

    // A client rewriting the visible labels (which no longer include the mode)
    // keeps the orchestration mode...
    let relabeled = thread_from(
        thread_update_labels(UpdateConversationThreadLabelsRequest {
            thread_id: "t1".into(),
            labels: vec!["general".into(), "tasks".into()],
        })
        .await
        .expect("relabel"),
    );
    assert_eq!(relabeled.mode, "orchestration");
    assert_eq!(relabeled.labels, vec!["general", "tasks"]);

    // ...and cannot smuggle a mode in through the label list either.
    let back = thread_from(
        thread_update_labels(UpdateConversationThreadLabelsRequest {
            thread_id: "t1".into(),
            labels: vec![
                "general".into(),
                "mode:orchestration".into(),
                "mode:x".into(),
            ],
        })
        .await
        .expect("relabel"),
    );
    assert_eq!(back.mode, "orchestration");
    assert_eq!(back.labels, vec!["general"]);
}

/// Release re-review C2: the Pet hand-off origin label is reserved — hidden
/// from clients, kept across a client relabel, and impossible to forge.
#[tokio::test]
async fn origin_label_is_hidden_kept_and_unforgeable() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    create_thread_with_title(&workspace, "h1", "Pet").await;
    create_thread_with_title(&workspace, "u1", "User").await;
    let dir = mode_test_workspace_dir().await;
    assert!(crate::neppy::agent::task_dispatcher::mark_pet_companion_thread(&dir, "h1"));

    let relabeled = thread_from(
        thread_update_labels(UpdateConversationThreadLabelsRequest {
            thread_id: "h1".into(),
            labels: vec!["general".into()],
        })
        .await
        .expect("relabel"),
    );
    assert_eq!(relabeled.labels, vec!["general"], "never shown to a client");
    let stored = thread_labels_for(&dir, "h1").await.expect("labels");
    assert!(crate::neppy::threads::mode::is_pet_companion_thread(
        &stored
    ));

    let forged = thread_from(
        thread_update_labels(UpdateConversationThreadLabelsRequest {
            thread_id: "u1".into(),
            labels: vec!["general".into(), "origin:pet_companion".into()],
        })
        .await
        .expect("relabel"),
    );
    assert_eq!(forged.labels, vec!["general"]);
    let stored = thread_labels_for(&dir, "u1").await.expect("labels");
    assert!(!crate::neppy::threads::mode::is_pet_companion_thread(
        &stored
    ));
}

#[tokio::test]
async fn mode_change_publishes_a_web_channel_event_without_content() {
    let _env_lock = crate::neppy::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let workspace = tempfile::tempdir().expect("workspace");
    let _guard = EnvVarGuard::set_to_path("NEPPY_WORKSPACE", workspace.path());
    create_thread_with_title(&workspace, "t-evt", "Work").await;

    let mut rx = crate::neppy::web_chat::subscribe_web_channel_events();
    thread_set_mode(SetThreadModeRequest {
        thread_id: "t-evt".into(),
        mode: "orchestration".into(),
        source: Some("rpc".into()),
    })
    .await
    .expect("set mode");

    let event = loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("event within timeout")
            .expect("recv");
        if event.event == "thread_mode_changed" && event.thread_id == "t-evt" {
            break event;
        }
    };
    assert_eq!(event.client_id, "system");
    let args = event.args.expect("args");
    assert_eq!(args["from"], "chat");
    assert_eq!(args["to"], "orchestration");
    assert_eq!(args["source"], "rpc");
    assert!(event.message.is_none() && event.full_response.is_none());

    // No event for a no-op switch.
    thread_set_mode(SetThreadModeRequest {
        thread_id: "t-evt".into(),
        mode: "orchestration".into(),
        source: None,
    })
    .await
    .expect("no-op");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), async {
            loop {
                let e = rx.recv().await.expect("recv");
                if e.event == "thread_mode_changed" && e.thread_id == "t-evt" {
                    return e;
                }
            }
        })
        .await
        .is_err(),
        "a no-op switch must not publish"
    );
}
