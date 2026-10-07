//! RPC operations for conversation thread management.

use crate::core::runtime::context::CoreContext;
use crate::neppy::config::Config;
use crate::neppy::inference::provider;
use crate::neppy::memory::{
    ApiEnvelope, ApiMeta, AppendConversationMessageRequest, BeginAnswerVariantRequest,
    BeginAnswerVariantResponse, ConversationMessageRecord, ConversationMessagesRequest,
    ConversationMessagesResponse, ConversationThreadSummary, ConversationThreadsListResponse,
    CreateConversationThreadRequest, DeleteConversationThreadRequest,
    DeleteConversationThreadResponse, EmptyRequest, GenerateConversationThreadTitleRequest,
    PaginationMeta, PurgeConversationThreadsResponse, SetActiveVariantRequest,
    UpdateConversationMessageRequest, UpdateConversationThreadLabelsRequest,
    UpdateConversationThreadTitleRequest, UpsertConversationThreadRequest,
};
// Every conversation-store call in this module goes through
// `conversations::blocking::*`, which runs the store's synchronous,
// globally-locked, fsync'ing operations on tokio's blocking pool. Calling the
// sync entry points directly from these handlers parked async worker threads on
// the store's `parking_lot` mutex, which starved the runtime and made
// `threads_create_new` blow the frontend's 30 s RPC budget (#5156).
use crate::neppy::memory::conversations;
use crate::neppy::threads::mode::{
    labels_with_mode, strip_reserved_labels, SetThreadModeRequest, ThreadMode, ThreadModeResult,
    ORIGIN_LABEL_PREFIX,
};
use crate::neppy::threads::title::{
    build_title_request, is_auto_generated_thread_title, sanitize_generated_title,
    title_from_user_message, title_log_fingerprint, THREAD_TITLE_LOG_PREFIX,
};
use crate::neppy::threads::turn_state::{
    self, ClearTurnStateRequest, ClearTurnStateResponse, GetTurnStateForRequestRequest,
    GetTurnStateRequest, GetTurnStateResponse, ListTurnStatesResponse,
};
use crate::neppy::threads::ThreadsError;
use crate::neppy::web_chat as web_channel;
use crate::rpc::RpcOutcome;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tinycortex::memory::conversations::{
    ConversationMessage, ConversationMessagePatch, ConversationThread, CreateConversationThread,
    CrossThreadHit,
};

fn request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn counts(entries: impl IntoIterator<Item = (&'static str, usize)>) -> BTreeMap<String, usize> {
    entries
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn envelope<T: Serialize>(
    data: T,
    counts: Option<BTreeMap<String, usize>>,
    pagination: Option<PaginationMeta>,
) -> RpcOutcome<ApiEnvelope<T>> {
    RpcOutcome::new(
        ApiEnvelope {
            data: Some(data),
            error: None,
            meta: ApiMeta {
                request_id: request_id(),
                latency_seconds: None,
                cached: None,
                counts,
                pagination,
            },
        },
        vec![],
    )
}

async fn workspace_dir() -> Result<PathBuf, String> {
    Config::load_or_init()
        .await
        .map(|c| c.workspace_dir)
        .map_err(|e| format!("load config: {e}"))
}

/// Run a destructive sequence to completion even if the caller's future is
/// dropped (client disconnect, RPC timeout).
///
/// Moving the store onto the blocking pool (#5156) introduced a cancellation
/// point that did not exist before. `spawn_blocking` work is never cancelled
/// when its `JoinHandle` is dropped, so the store mutation lands regardless —
/// but the `.await` on that handle *is* a yield point, and previously the
/// synchronous store call had none. Dropping the handler there leaves the thread
/// deleted while the cleanup that follows it never runs: the web-channel session
/// stays live and can append to a thread index row that no longer exists,
/// detached sub-agents keep running and queueing completions, and the turn
/// snapshot survives to resurface as `Interrupted` for a thread that is gone.
/// Those are precisely the invariants `thread_delete`'s ordering comments exist
/// to hold.
///
/// Owning the mutation *and* its cleanup in one spawned task decouples the
/// sequence from the caller's lifetime. The ambient [`CoreContext`] is carried
/// across explicitly: a bare `tokio::spawn` drops the `task_local` scope, and
/// `CoreContext::current` then silently falls back to the process default —
/// which under multi-tenant scoped dispatch is the wrong workspace.
async fn run_to_completion<T, F>(operation: &'static str, fut: F) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>> + Send + 'static,
    T: Send + 'static,
{
    let ctx = CoreContext::current();
    tokio::spawn(async move {
        match ctx {
            Some(ctx) => CoreContext::scope(ctx, fut).await,
            None => fut.await,
        }
    })
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(
            operation,
            error = %error,
            "[threads] destructive task failed to join"
        );
        Err(format!("{operation} task failed: {error}"))
    })
}

fn thread_to_summary(thread: ConversationThread) -> ConversationThreadSummary {
    // The mode lives in a reserved label; report it as its own field and keep it
    // out of the user-visible label list.
    let mode = ThreadMode::from_labels(&thread.labels);
    ConversationThreadSummary {
        id: thread.id,
        title: thread.title,
        chat_id: thread.chat_id,
        is_active: thread.is_active,
        message_count: thread.message_count,
        last_message_at: thread.last_message_at,
        created_at: thread.created_at,
        parent_thread_id: thread.parent_thread_id,
        labels: strip_reserved_labels(thread.labels),
        personality_id: thread.personality_id,
        mode: mode.as_str().to_string(),
    }
}

fn message_to_record(message: ConversationMessage) -> ConversationMessageRecord {
    ConversationMessageRecord {
        id: message.id,
        content: message.content,
        message_type: message.message_type,
        extra_metadata: message.extra_metadata,
        sender: message.sender,
        created_at: message.created_at,
    }
}

fn record_to_message(record: ConversationMessageRecord) -> ConversationMessage {
    ConversationMessage {
        id: record.id,
        content: record.content,
        message_type: record.message_type,
        extra_metadata: record.extra_metadata,
        sender: record.sender,
        created_at: record.created_at,
    }
}

fn fallback_title_from_user_message(thread_id: &str, user_message: &str) -> Option<String> {
    let title = title_from_user_message(user_message);
    if let Some(title) = &title {
        tracing::debug!(
            thread_id = %thread_id,
            title_len = title.chars().count(),
            title_hash = %title_log_fingerprint(title),
            "{THREAD_TITLE_LOG_PREFIX} derived fallback title from user message"
        );
    } else {
        tracing::debug!(
            thread_id = %thread_id,
            "{THREAD_TITLE_LOG_PREFIX} user message did not yield fallback title"
        );
    }
    title
}

async fn update_thread_with_fallback_title(
    dir: PathBuf,
    thread: ConversationThread,
    user_message: &str,
) -> Result<ConversationThread, String> {
    let Some(title) = fallback_title_from_user_message(&thread.id, user_message) else {
        return Ok(thread);
    };
    if title == thread.title {
        return Ok(thread);
    }
    conversations::blocking::update_thread_title(
        dir,
        thread.id.clone(),
        title,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
}

/// Lists all conversation threads.
pub async fn threads_list(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadsListResponse>>, String> {
    let dir = workspace_dir().await?;
    let threads = conversations::blocking::list_threads(dir)
        .await?
        .into_iter()
        .map(thread_to_summary)
        .collect::<Vec<_>>();
    let count = threads.len();
    Ok(envelope(
        ConversationThreadsListResponse { threads, count },
        Some(counts([("num_threads", count)])),
        None,
    ))
}

/// Creates or refreshes a conversation thread.
pub async fn thread_upsert(
    request: UpsertConversationThreadRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let thread = conversations::blocking::ensure_thread(
        dir,
        CreateConversationThread {
            id: request.id,
            title: request.title,
            created_at: request.created_at,
            parent_thread_id: request.parent_thread_id,
            labels: request.labels,
            personality_id: request.personality_id,
        },
    )
    .await?;
    Ok(envelope(
        thread_to_summary(thread),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Creates a new conversation thread with auto-generated ID and title.
pub async fn thread_create_new(
    request: CreateConversationThreadRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let id = format!("thread-{}", uuid::Uuid::new_v4());
    let now = chrono::Local::now();
    let title = format!("Chat {} {}", now.format("%b %-d"), now.format("%-I:%M %p"));
    let created_at = chrono::Utc::now().to_rfc3339();
    let thread = conversations::blocking::ensure_thread(
        dir,
        CreateConversationThread {
            id,
            title,
            created_at,
            parent_thread_id: None,
            // Pass labels through as-is; the store's infer_labels() applies
            // the same default on index rebuild, so this is the single source
            // of truth for default labels.
            labels: request.labels,
            personality_id: request.personality_id,
        },
    )
    .await?;
    tracing::debug!(
        thread_id = %thread.id,
        labels = ?thread.labels,
        "[threads] created new thread"
    );
    Ok(envelope(
        thread_to_summary(thread),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Lists messages for a conversation thread.
pub async fn messages_list(
    request: ConversationMessagesRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessagesResponse>>, String> {
    let dir = workspace_dir().await?;
    let messages = conversations::blocking::get_messages(dir, request.thread_id.clone())
        .await?
        .into_iter()
        .map(message_to_record)
        .collect::<Vec<_>>();
    let count = messages.len();
    Ok(envelope(
        ConversationMessagesResponse { messages, count },
        Some(counts([("num_messages", count)])),
        None,
    ))
}

/// Search messages across **every** thread in the workspace for a query,
/// returning up to `limit` of the most-recent matches (newest first). Backed
/// by the trigram/CJK-bigram inverted index in `memory_conversations` — the
/// same cross-chat reader the durable-context pipeline uses (issue #1505).
///
/// Read-only and workspace-scoped. `exclude_thread_id` lets a caller drop the
/// active chat from the results when it already has that context in hand.
pub async fn transcript_search(
    query: &str,
    limit: usize,
    exclude_thread_id: Option<&str>,
) -> Result<Vec<CrossThreadHit>, String> {
    let dir = workspace_dir().await?;
    log::debug!(
        "[threads][transcript_search] query_chars={} limit={} exclude={:?}",
        query.chars().count(),
        limit,
        exclude_thread_id
    );
    let hits = conversations::blocking::search_cross_thread_messages(
        dir,
        query.to_string(),
        limit,
        exclude_thread_id.map(str::to_string),
    )
    .await?;
    log::debug!("[threads][transcript_search] hits={}", hits.len());
    Ok(hits)
}

/// Appends a message to a conversation thread.
pub async fn message_append(
    request: AppendConversationMessageRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessageRecord>>, ThreadsError> {
    let dir = workspace_dir().await?;
    let message = conversations::blocking::append_message(
        dir,
        request.thread_id.clone(),
        record_to_message(request.message),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(&request.thread_id, err))?;
    Ok(envelope(
        message_to_record(message),
        Some(counts([("num_messages", 1)])),
        None,
    ))
}

/// Generates a durable thread title from the first user message and assistant reply.
pub async fn thread_generate_title(
    request: GenerateConversationThreadTitleRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, ThreadsError> {
    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e}"))?;
    let dir = config.workspace_dir.clone();
    let Some(thread) = conversations::blocking::list_threads(dir.clone())
        .await?
        .into_iter()
        .find(|thread| thread.id == request.thread_id)
    else {
        return Err(ThreadsError::not_found(request.thread_id));
    };

    if !is_auto_generated_thread_title(&thread.title) {
        tracing::debug!(
            thread_id = %request.thread_id,
            title_len = thread.title.chars().count(),
            title_hash = %title_log_fingerprint(&thread.title),
            "{THREAD_TITLE_LOG_PREFIX} skipping non-placeholder title"
        );
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    }

    let messages =
        conversations::blocking::get_messages(dir.clone(), request.thread_id.clone()).await?;
    let Some(first_user_message) = messages
        .iter()
        .find(|message| message.sender == "user" && !message.content.trim().is_empty())
        .map(|message| message.content.trim().to_string())
    else {
        tracing::debug!(
            thread_id = %request.thread_id,
            "{THREAD_TITLE_LOG_PREFIX} no user message yet; skipping"
        );
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    let assistant_message = request
        .assistant_message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            messages
                .iter()
                .find(|message| message.sender == "agent" && !message.content.trim().is_empty())
                .map(|message| message.content.trim().to_string())
        });

    let Some(assistant_message) = assistant_message else {
        tracing::debug!(
            thread_id = %request.thread_id,
            "{THREAD_TITLE_LOG_PREFIX} no assistant message yet; applying fallback title"
        );
        let updated = update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
        return Ok(envelope(
            thread_to_summary(updated),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    // `_with_model_id` rather than the plain constructor: the debug line below
    // reports the model this call actually dispatches on, and only the factory
    // knows what the `summarization` role resolved to for this configuration.
    let (chat_model, resolved_model) =
        match provider::create_chat_model_with_model_id("summarization", &config, 0.2) {
            Ok(resolved) => resolved,
            Err(error) => {
                tracing::warn!(
                    thread_id = %request.thread_id,
                    error = %error,
                    "{THREAD_TITLE_LOG_PREFIX} provider init failed; applying fallback title"
                );
                let updated =
                    update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
                return Ok(envelope(
                    thread_to_summary(updated),
                    Some(counts([("num_threads", 1)])),
                    None,
                ));
            }
        };

    tracing::debug!(
        thread_id = %request.thread_id,
        user_len = first_user_message.len(),
        assistant_len = assistant_message.len(),
        model = %resolved_model,
        "{THREAD_TITLE_LOG_PREFIX} generating thread title"
    );

    let raw_title = match chat_model
        .invoke(
            &(),
            build_title_request(&first_user_message, &assistant_message),
        )
        .await
    {
        Ok(response) => response.text(),
        Err(error) => {
            tracing::warn!(
                thread_id = %request.thread_id,
                error = %error,
                "{THREAD_TITLE_LOG_PREFIX} title generation failed; applying fallback title"
            );
            let updated =
                update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
            return Ok(envelope(
                thread_to_summary(updated),
                Some(counts([("num_threads", 1)])),
                None,
            ));
        }
    };

    let Some(title) = sanitize_generated_title(&raw_title) else {
        tracing::warn!(
            thread_id = %request.thread_id,
            raw_title_len = raw_title.chars().count(),
            raw_title_hash = %title_log_fingerprint(&raw_title),
            "{THREAD_TITLE_LOG_PREFIX} generated empty title after sanitization; applying fallback title"
        );
        let updated = update_thread_with_fallback_title(dir, thread, &first_user_message).await?;
        return Ok(envelope(
            thread_to_summary(updated),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    };

    if title == thread.title {
        return Ok(envelope(
            thread_to_summary(thread),
            Some(counts([("num_threads", 1)])),
            None,
        ));
    }

    let updated = conversations::blocking::update_thread_title(
        dir,
        request.thread_id.clone(),
        title,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(&request.thread_id, err))?;

    tracing::debug!(
        thread_id = %request.thread_id,
        title_len = updated.title.chars().count(),
        title_hash = %title_log_fingerprint(&updated.title),
        "{THREAD_TITLE_LOG_PREFIX} updated thread title"
    );

    Ok(envelope(
        thread_to_summary(updated),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Updates labels for a conversation thread.
///
/// An empty `labels` vec is valid and clears all labels from the thread,
/// making it invisible in every non-"All" filter view. Callers should
/// ensure this is intentional.
pub async fn thread_update_labels(
    request: UpdateConversationThreadLabelsRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    // A client rewrites the user-visible labels; the reserved mode label is not
    // among them (it is stripped from every summary), so re-attach the thread's
    // current mode or this call would silently flip an orchestration thread
    // back to chat. A client-supplied mode label is dropped — mode changes only
    // through `thread_set_mode`.
    let current_mode = thread_mode_for(&dir, &request.thread_id).await;
    // Reserved origin labels (e.g. the Pet hand-off marker) are likewise kept
    // from the stored thread and never taken from the client: a client can
    // neither remove one (loosening follow-up turns) nor forge one.
    let kept_origin: Vec<String> = thread_labels_for(&dir, &request.thread_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|l| l.starts_with(ORIGIN_LABEL_PREFIX))
        .collect();
    let mut labels = labels_with_mode(strip_reserved_labels(request.labels.clone()), current_mode);
    labels.extend(kept_origin);
    let thread = conversations::blocking::update_thread_labels(
        dir,
        request.thread_id.clone(),
        labels,
        chrono::Utc::now().to_rfc3339(),
    )
    .await?;
    tracing::debug!(
        thread_id = %request.thread_id,
        labels = ?request.labels,
        "[threads] updated thread labels"
    );
    Ok(envelope(
        thread_to_summary(thread),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// The stored labels of `thread_id` (reserved labels included), or `None`
/// when the thread does not exist or the store cannot be read.
pub async fn thread_labels_for(dir: &std::path::Path, thread_id: &str) -> Option<Vec<String>> {
    match conversations::blocking::list_threads(dir.to_path_buf()).await {
        Ok(threads) => threads
            .into_iter()
            .find(|t| t.id == thread_id)
            .map(|t| t.labels),
        Err(err) => {
            tracing::warn!(
                thread_id = %thread_id,
                error = %err,
                "[threads] could not read thread store for labels"
            );
            None
        }
    }
}

/// The persisted operating mode of `thread_id`. A thread that does not exist
/// (or a store that cannot be read) reads as the default, `chat`: the mode is a
/// refinement of how a turn runs, and failing a turn over it would be worse than
/// running it in the default mode.
pub async fn thread_mode_for(dir: &std::path::Path, thread_id: &str) -> ThreadMode {
    match conversations::blocking::list_threads(dir.to_path_buf()).await {
        Ok(threads) => threads
            .iter()
            .find(|t| t.id == thread_id)
            .map(|t| ThreadMode::from_labels(&t.labels))
            .unwrap_or_default(),
        Err(err) => {
            tracing::warn!(
                thread_id = %thread_id,
                error = %err,
                "[mode] could not read thread store; defaulting to chat"
            );
            ThreadMode::default()
        }
    }
}

/// Switches a thread between `chat`, `orchestration` and `debug`.
///
/// Same thread, same history: only the reserved mode label changes. Publishes
/// [`DomainEvent::ThreadModeChanged`] and a `thread_mode_changed` web-channel
/// event when the mode actually changes, and evicts the thread's cached session
/// agent so the very next turn is built for the new mode (the conversation is
/// re-seeded from the thread's own history on rebuild).
pub async fn thread_set_mode(
    request: SetThreadModeRequest,
) -> Result<RpcOutcome<ApiEnvelope<ThreadModeResult>>, ThreadsError> {
    let mode = ThreadMode::parse(&request.mode).ok_or_else(|| {
        ThreadsError::Message(format!(
            "unknown thread mode '{}': expected 'chat', 'orchestration' or 'debug'",
            request.mode.trim()
        ))
    })?;
    let dir = workspace_dir().await?;
    let source = request
        .source
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("rpc")
        .to_string();
    let result = apply_thread_mode(&dir, &request.thread_id, mode, &source).await?;
    Ok(envelope(result, Some(counts([("num_threads", 1)])), None))
}

/// Shared by [`thread_set_mode`] and the chat-send path (`mode` param).
pub async fn apply_thread_mode(
    dir: &std::path::Path,
    thread_id: &str,
    mode: ThreadMode,
    source: &str,
) -> Result<ThreadModeResult, ThreadsError> {
    let threads = conversations::blocking::list_threads(dir.to_path_buf()).await?;
    let Some(existing) = threads.into_iter().find(|t| t.id == thread_id) else {
        return Err(ThreadsError::not_found(thread_id));
    };
    let previous = ThreadMode::from_labels(&existing.labels);
    if previous == mode {
        tracing::debug!(
            thread_id = %thread_id,
            mode = %mode,
            "[mode] set_mode no-op: thread already in requested mode"
        );
        return Ok(ThreadModeResult {
            thread: thread_to_summary(existing),
            previous_mode: previous.as_str().to_string(),
            changed: false,
        });
    }
    let labels = labels_with_mode(existing.labels, mode);
    let updated = conversations::blocking::update_thread_labels(
        dir.to_path_buf(),
        thread_id.to_string(),
        labels,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    .map_err(|err| ThreadsError::from_thread_scoped_store_error(thread_id, err))?;

    // Content-free by design: ids and the two mode names only.
    log::info!(
        "[mode] thread={} {}->{} source={}",
        thread_id,
        previous,
        mode,
        source
    );
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ThreadModeChanged {
        thread_id: thread_id.to_string(),
        from: previous.as_str().to_string(),
        to: mode.as_str().to_string(),
        source: source.to_string(),
    });
    web_channel::publish_web_channel_event(crate::core::socketio::WebChannelEvent {
        event: "thread_mode_changed".to_string(),
        // "system" reaches every connected client; the UI filters on thread_id.
        client_id: "system".to_string(),
        thread_id: thread_id.to_string(),
        args: Some(serde_json::json!({
            "from": previous.as_str(),
            "to": mode.as_str(),
            "source": source,
        })),
        ..Default::default()
    });
    // The cached session agent was built for the old mode (tool surface and
    // prompt addendum). Evict it; the next turn rebuilds for the new mode and
    // re-seeds from this thread's history.
    web_channel::invalidate_thread_sessions(thread_id).await;

    Ok(ThreadModeResult {
        thread: thread_to_summary(updated),
        previous_mode: previous.as_str().to_string(),
        changed: true,
    })
}

/// Sets a user-specified title on a conversation thread, bypassing AI generation.
pub async fn thread_update_title(
    request: UpdateConversationThreadTitleRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationThreadSummary>>, String> {
    let dir = workspace_dir().await?;
    let title = request.title.trim().to_string();
    if title.is_empty() {
        return Err("title must not be empty".to_string());
    }
    let updated = conversations::blocking::update_thread_title(
        dir,
        request.thread_id.clone(),
        title,
        chrono::Utc::now().to_rfc3339(),
    )
    .await
    .map_err(|err| format!("update title: {err}"))?;
    tracing::debug!(
        thread_id = %request.thread_id,
        title_len = updated.title.chars().count(),
        "[threads] user updated thread title"
    );
    Ok(envelope(
        thread_to_summary(updated),
        Some(counts([("num_threads", 1)])),
        None,
    ))
}

/// Start another answer to a question, keeping the one it already has.
///
/// Called before a regenerate sends, and it is what makes the existing answer a
/// variant rather than a stray message: it is tagged with the question it
/// answers and with a turn id of its own, so the reply about to arrive becomes
/// the second answer instead of the first one anybody can see. Tagging only the
/// new answer leaves the original untagged, and an untagged message is not part
/// of any group — it would stay on screen next to its replacement, the count
/// would be one short, and the first regenerate would show no switcher at all.
///
/// Two more things happen here for the same reason they happen when switching.
/// Any stored selection on the question is cleared, so "newest wins" puts the
/// incoming answer in effect — without that, regenerating after switching back
/// would produce an answer that is superseded the moment it lands. And the
/// thread's cached session is evicted, because a turn resumes from the session
/// the agent already holds: left alone, the model would answer again with the
/// previous answer still in its transcript, which is a follow-up, not a
/// regeneration.
///
/// Idempotent: tagging an already-tagged answer writes nothing, so a retried
/// send cannot split one answer across two turns.
pub async fn message_begin_answer_variant(
    request: BeginAnswerVariantRequest,
) -> Result<RpcOutcome<ApiEnvelope<BeginAnswerVariantResponse>>, String> {
    use crate::neppy::memory::conversations::variants;

    let dir = workspace_dir().await?;
    let messages =
        conversations::blocking::get_messages(dir.clone(), request.thread_id.clone()).await?;

    let question = messages
        .iter()
        .find(|message| message.id == request.message_id && message.sender == "user")
        .ok_or_else(|| {
            format!(
                "message '{}' is not a question in thread '{}'",
                request.message_id, request.thread_id
            )
        })?;
    let question_metadata = question.extra_metadata.clone();

    // The turn id is the first untagged answer message's id, so a segmented
    // answer groups under its opening segment and the id stays stable if this
    // runs twice.
    let untagged: Vec<(String, serde_json::Value)> =
        variants::untagged_answers(&messages, &request.message_id)
            .into_iter()
            .map(|message| (message.id.clone(), message.extra_metadata.clone()))
            .collect();
    let turn_id = untagged.first().map(|(id, _)| id.clone());

    let mut tagged = 0usize;
    if let Some(turn_id) = turn_id.as_deref() {
        for (message_id, metadata) in &untagged {
            conversations::blocking::update_message(
                dir.clone(),
                request.thread_id.clone(),
                message_id.clone(),
                ConversationMessagePatch {
                    extra_metadata: Some(variants::metadata_with_variant(
                        metadata,
                        &request.message_id,
                        turn_id,
                    )),
                },
            )
            .await?;
            tagged += 1;
        }
    }

    if variants::active_variant_choice(question).is_some() {
        conversations::blocking::update_message(
            dir.clone(),
            request.thread_id.clone(),
            request.message_id.clone(),
            ConversationMessagePatch {
                extra_metadata: Some(variants::metadata_without_selection(&question_metadata)),
            },
        )
        .await?;
    }

    // Re-read rather than counting from the pre-tag snapshot: the existing
    // answers are the ones just tagged plus any from earlier regenerates, and
    // the store is what knows how that landed. `+ 1` for the answer about to be
    // produced, which is what the switcher will show.
    let after = conversations::blocking::get_messages(dir, request.thread_id.clone()).await?;
    let variant_count = variants::variant_turns(&after, &request.message_id).len() + 1;

    crate::neppy::web_chat::invalidate_thread_sessions(&request.thread_id).await;
    log::info!(
        "[threads] begin answer variant thread_id={} message_id={} turn_id={:?} tagged={} variant_count={}",
        request.thread_id,
        request.message_id,
        turn_id,
        tagged,
        variant_count
    );

    Ok(envelope(
        BeginAnswerVariantResponse {
            variant_turn_id: turn_id,
            tagged,
            variant_count,
        },
        Some(counts([("num_messages", tagged)])),
        None,
    ))
}

/// Choose which answer to a question is the one in effect.
///
/// Three things have to happen together, which is why this is an operation and
/// not a `message_update` from the client. The id is validated against the
/// question's real answers, so a wrong one fails here instead of being stored
/// and quietly ignored at read time. The selection is merged into the message's
/// metadata rather than replacing it, because the store's patch takes a whole
/// object. And the thread's cached session is evicted, because a turn resumes
/// from the session the agent already holds — without the eviction the model
/// would keep the context it was built with and switching would change only
/// what is on screen.
pub async fn message_set_active_variant(
    request: SetActiveVariantRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessageRecord>>, String> {
    use crate::neppy::memory::conversations::variants;

    let dir = workspace_dir().await?;
    let messages =
        conversations::blocking::get_messages(dir.clone(), request.thread_id.clone()).await?;

    if !variants::is_variant_of(&messages, &request.message_id, &request.variant_id) {
        return Err(format!(
            "message '{}' is not an answer to '{}' in thread '{}'",
            request.variant_id, request.message_id, request.thread_id
        ));
    }

    let existing = messages
        .iter()
        .find(|message| message.id == request.message_id)
        .map(|message| message.extra_metadata.clone())
        .unwrap_or(serde_json::Value::Null);

    let updated = conversations::blocking::update_message(
        dir,
        request.thread_id.clone(),
        request.message_id.clone(),
        ConversationMessagePatch {
            extra_metadata: Some(variants::metadata_with_selection(
                &existing,
                &request.variant_id,
            )),
        },
    )
    .await?;

    crate::neppy::web_chat::invalidate_thread_sessions(&request.thread_id).await;
    log::info!(
        "[threads] active answer for message_id={} in thread_id={} is now variant_id={}",
        request.message_id,
        request.thread_id,
        request.variant_id
    );

    Ok(envelope(
        message_to_record(updated),
        Some(counts([("num_messages", 1)])),
        None,
    ))
}

/// Updates metadata on an existing conversation message.
pub async fn message_update(
    request: UpdateConversationMessageRequest,
) -> Result<RpcOutcome<ApiEnvelope<ConversationMessageRecord>>, String> {
    let dir = workspace_dir().await?;
    let message = conversations::blocking::update_message(
        dir,
        request.thread_id.clone(),
        request.message_id.clone(),
        ConversationMessagePatch {
            extra_metadata: request.extra_metadata,
        },
    )
    .await?;
    Ok(envelope(
        message_to_record(message),
        Some(counts([("num_messages", 1)])),
        None,
    ))
}

/// Deletes a conversation thread and its message log.
///
/// The store mutation and every cleanup step it implies run inside one
/// [`run_to_completion`] task, so a caller that disconnects mid-delete cannot
/// leave the thread gone from the store with its sessions, sub-agents and turn
/// snapshot still live.
pub async fn thread_delete(
    request: DeleteConversationThreadRequest,
) -> Result<RpcOutcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
    let dir = workspace_dir().await?;
    run_to_completion("thread_delete", thread_delete_inner(dir, request)).await
}

async fn thread_delete_inner(
    dir: PathBuf,
    request: DeleteConversationThreadRequest,
) -> Result<RpcOutcome<ApiEnvelope<DeleteConversationThreadResponse>>, String> {
    let deleted = conversations::blocking::delete_thread(
        dir.clone(),
        request.thread_id.clone(),
        request.deleted_at.clone(),
    )
    .await?;
    // Invalidate the in-process web-channel session BEFORE the
    // turn-state cleanup. The snapshot deletion is fallible and
    // returns early on error; if invalidation ran after, an active
    // session for the now-deleted thread could linger and try to
    // append to a thread index row that no longer exists.
    web_channel::invalidate_thread_sessions(&request.thread_id).await;
    // Cancel any detached sub-agents this thread spawned BEFORE clearing their
    // queued results: abort the in-flight ones first so a child can't record a
    // completion in the gap between the two calls, then discard anything already
    // queued for delivery. Both target a thread that's being deleted, so there's
    // nowhere left to deliver to — abort + cleanup is the whole behavior.
    let cancelled = crate::neppy::agent::orchestration::running_subagents::cancel_for_thread(
        &request.thread_id,
    );
    let discarded = crate::neppy::agent::orchestration::background_completions::discard_for_thread(
        &request.thread_id,
    );
    log::debug!(
        "[threads] thread_delete thread_id={} cancelled_subagents={} discarded_completions={}",
        request.thread_id,
        cancelled,
        discarded
    );
    // Drop any persisted in-flight turn snapshot for this thread —
    // otherwise `threads_turn_state_list` keeps surfacing it (as
    // `Interrupted` on next restart) for a thread that no longer
    // exists. Failure here is surfaced as an RPC error so callers
    // can't observe a thread "deleted" while its snapshot (which
    // mirrors conversation-derived state) remains on disk; the
    // thread row itself is already gone at this point so the caller
    // sees a partial failure they can act on instead of silent drift.
    turn_state::store::delete(dir, &request.thread_id).map_err(|err| {
        format!(
            "thread {} deleted but turn-snapshot cleanup failed: {err}",
            request.thread_id
        )
    })?;
    Ok(envelope(
        DeleteConversationThreadResponse { deleted },
        None,
        None,
    ))
}

/// [`thread_delete`] against an explicit workspace, for a domain that holds its
/// own `Config` rather than the process-global one — the Pet companion's
/// "Delete all" removes the hand-off threads it created this way. Same store
/// mutation and cleanup (session invalidation, sub-agent cancel, turn snapshot)
/// as the RPC. `Ok(false)` when no such thread exists.
pub(crate) async fn thread_delete_in(dir: PathBuf, thread_id: String) -> Result<bool, String> {
    let request = DeleteConversationThreadRequest {
        thread_id,
        deleted_at: chrono::Utc::now().to_rfc3339(),
    };
    let outcome = run_to_completion("thread_delete", thread_delete_inner(dir, request)).await?;
    Ok(outcome.value.data.is_some_and(|d| d.deleted))
}

/// Purges all conversation threads and messages.
///
/// Same cancellation contract as [`thread_delete`]: the purge and its sub-agent
/// / turn-snapshot cleanup are one [`run_to_completion`] unit, so a dropped
/// caller cannot leave every thread wiped while their sub-agents keep running.
pub async fn threads_purge(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let dir = workspace_dir().await?;
    run_to_completion("threads_purge", threads_purge_inner(dir)).await
}

async fn threads_purge_inner(
    dir: PathBuf,
) -> Result<RpcOutcome<ApiEnvelope<PurgeConversationThreadsResponse>>, String> {
    let stats = conversations::blocking::purge_threads(dir.clone()).await?;
    // No parent thread survives a purge, so cancel every detached sub-agent and
    // wipe every queued result. Same ordering as `thread_delete`: abort the
    // in-flight runs first, then clear the delivery queue. Tombstone each
    // cancelled sub-agent's thread BEFORE the final wipe so a straggler that
    // wins the cooperative-abort race (records after the wipe) is still dropped
    // by `record_completion` rather than delivered into a purged thread.
    use crate::neppy::agent::orchestration::{background_completions, running_subagents};
    let cancelled_threads = running_subagents::cancel_all();
    let mut discarded = 0;
    for thread_id in &cancelled_threads {
        discarded += background_completions::discard_for_thread(thread_id);
    }
    discarded += background_completions::clear_all();
    log::debug!(
        "[threads] threads_purge cancelled_threads={} discarded_completions={}",
        cancelled_threads.len(),
        discarded
    );
    // Threads are gone, so any orphan turn snapshots can never be
    // reattached to a live thread. Wipe them in the same call so
    // `turn_state_list` returns an empty set after a purge. Use the
    // parse-independent `clear_all` so corrupted / half-written
    // snapshot files (which `list()` would warn-and-skip) are also
    // removed — a destructive cleanup must not leave behind anything
    // it failed to deserialize. Failures surface as RPC errors.
    turn_state::store::clear_all(dir.clone())
        .map_err(|err| format!("threads purged but turn-snapshot cleanup failed: {err}"))?;
    Ok(envelope(
        PurgeConversationThreadsResponse {
            messages_deleted: stats.message_count,
            agent_threads_deleted: stats.thread_count,
            agent_messages_deleted: stats.message_count,
        },
        None,
        None,
    ))
}

/// Returns the persisted in-flight turn snapshot for a thread, if any.
pub async fn turn_state_get(
    request: GetTurnStateRequest,
) -> Result<RpcOutcome<ApiEnvelope<GetTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_state = turn_state::store::get(dir, &request.thread_id)?;
    let present = turn_state.is_some();
    Ok(envelope(
        GetTurnStateResponse { turn_state },
        Some(counts([("present", usize::from(present))])),
        None,
    ))
}

/// Lists every persisted turn snapshot — used by the UI on cold boot to
/// surface interrupted turns from a previous process.
pub async fn turn_state_list(
    _request: EmptyRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListTurnStatesResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_states = turn_state::store::list(dir)?;
    let count = turn_states.len();
    Ok(envelope(
        ListTurnStatesResponse { turn_states, count },
        Some(counts([("num_turn_states", count)])),
        None,
    ))
}

/// Lists every persisted turn snapshot for one thread, newest first — the
/// per-turn history that lets the UI render each answer's own process trail.
pub async fn turn_state_history(
    request: GetTurnStateRequest,
) -> Result<RpcOutcome<ApiEnvelope<ListTurnStatesResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_states = turn_state::store::list_thread(dir, &request.thread_id)?;
    let count = turn_states.len();
    Ok(envelope(
        ListTurnStatesResponse { turn_states, count },
        Some(counts([("num_turn_states", count)])),
        None,
    ))
}

/// Returns one specific turn of a thread by its producing request id — used by
/// the UI to lazily load a past turn's full timeline when its insights block is
/// first expanded.
pub async fn turn_state_get_turn(
    request: GetTurnStateForRequestRequest,
) -> Result<RpcOutcome<ApiEnvelope<GetTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let turn_state = turn_state::store::get_turn(dir, &request.thread_id, &request.request_id)?;
    let present = turn_state.is_some();
    Ok(envelope(
        GetTurnStateResponse { turn_state },
        Some(counts([("present", usize::from(present))])),
        None,
    ))
}

/// Clears the persisted turn snapshot for a thread (e.g. after the user
/// dismisses an "interrupted" banner).
pub async fn turn_state_clear(
    request: ClearTurnStateRequest,
) -> Result<RpcOutcome<ApiEnvelope<ClearTurnStateResponse>>, String> {
    let dir = workspace_dir().await?;
    let cleared = turn_state::store::delete(dir, &request.thread_id)?;
    Ok(envelope(ClearTurnStateResponse { cleared }, None, None))
}

/// Request for [`token_usage`]: the thread whose persisted usage to total.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ThreadTokenUsageRequest {
    pub thread_id: String,
}

/// Request for [`transcript_get`]: the thread to project, plus newest-first
/// pagination controls. `cursor` is the opaque token from a prior page's
/// `nextCursor`; `limit` defaults to one screen.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TranscriptGetRequest {
    pub thread_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Project a thread's settled transcript (derived from `session_raw/*.jsonl`)
/// into typed display items, newest-first paginated. Returns an empty page with
/// `hasTranscript: false` when the thread has no persisted transcript yet.
pub async fn transcript_get(
    request: TranscriptGetRequest,
) -> Result<RpcOutcome<ApiEnvelope<crate::neppy::threads::transcript_view::TranscriptPage>>, String>
{
    let dir = workspace_dir().await?;
    let thread_id = request.thread_id.trim();
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    let page = crate::neppy::threads::transcript_view::get_page(
        &dir,
        thread_id,
        request.cursor.as_deref(),
        request.limit,
    );
    let counts = counts([
        ("items", page.items.len()),
        ("total", page.total),
        ("has_transcript", usize::from(page.has_transcript)),
    ]);
    let pagination = Some(PaginationMeta {
        limit: request
            .limit
            .unwrap_or(crate::neppy::threads::transcript_view::DEFAULT_LIMIT),
        offset: request
            .cursor
            .as_deref()
            .and_then(|c| c.trim().parse::<usize>().ok())
            .unwrap_or(0),
        count: page.total,
    });
    Ok(envelope(page, Some(counts), pagination))
}

/// Aggregated token/cost usage for one thread, read back from its persisted
/// session transcripts. Seeds the UI footer when the user selects a thread so
/// the totals reflect prior turns instead of starting at zero.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThreadTokenUsageResponse {
    pub thread_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cost_usd: f64,
    pub turn_count: usize,
    /// Tokens of the most recent turn — numerator for the context-window gauge.
    pub last_turn_input_tokens: u64,
    pub last_turn_output_tokens: u64,
    /// Context window (tokens) inferred from the last model; `0` when unknown.
    pub context_window: u64,
    pub model: Option<String>,
    pub updated: Option<String>,
    /// `false` when the thread has no persisted turns yet (all zeros).
    pub has_usage: bool,
    /// Per-archetype sub-agent spend (re-audited at current pricing). The
    /// top-level totals already include this; it's broken out for the UI's
    /// per-agent footer rows.
    pub subagents: Vec<SubagentUsageDto>,
}

/// One sub-agent archetype's contribution within a thread.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubagentUsageDto {
    pub agent_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    pub runs: usize,
}

/// Total a thread's persisted token/cost usage across its root transcripts.
pub async fn token_usage(
    request: ThreadTokenUsageRequest,
) -> Result<RpcOutcome<ApiEnvelope<ThreadTokenUsageResponse>>, String> {
    let dir = workspace_dir().await?;
    let summary = crate::neppy::agent::harness::session::transcript::read_thread_usage_summary(
        &dir,
        &request.thread_id,
    );

    // Re-audit cost at CURRENT pricing rather than trusting the
    // `charged_amount_usd` persisted in the transcript: those values were
    // stamped at turn time and don't reflect later tier-pricing corrections.
    // Recompute from the persisted token counts using the last-known model's
    // rates; falls back to `fallback` only when the model is unknown.
    let audit_cost =
        |model: Option<&str>, input: u64, output: u64, cached: u64, fallback: f64| match model {
            Some(m) => crate::neppy::agent::cost::estimate_call_cost_usd(
                m,
                &crate::neppy::inference::provider::UsageInfo {
                    input_tokens: input,
                    output_tokens: output,
                    cached_input_tokens: cached,
                    ..Default::default()
                },
            ),
            None => fallback,
        };

    let response = match summary {
        Some(s) => {
            let context_window = s
                .model
                .as_deref()
                .and_then(crate::neppy::inference::model_context::context_window_for_model)
                .unwrap_or(0);

            // Orchestrator (root) spend, re-audited.
            let orchestrator_cost = audit_cost(
                s.model.as_deref(),
                s.input_tokens,
                s.output_tokens,
                s.cached_input_tokens,
                s.cost_usd,
            );

            // Sub-agent archetypes, each re-audited with its own model. Older
            // sub-agent transcripts didn't persist a model on their messages, so
            // fall back to the thread's (root) model rather than pricing them at
            // $0 — sub-agents usually run on the same managed tier as the parent.
            let mut subagents = Vec::with_capacity(s.subagents.len());
            let (mut sub_in, mut sub_out, mut sub_cached, mut sub_cost) = (0u64, 0u64, 0u64, 0.0);
            for g in &s.subagents {
                let sub_model = g.model.as_deref().or(s.model.as_deref());
                let cost = audit_cost(
                    sub_model,
                    g.input_tokens,
                    g.output_tokens,
                    g.cached_input_tokens,
                    0.0,
                );
                sub_in = sub_in.saturating_add(g.input_tokens);
                sub_out = sub_out.saturating_add(g.output_tokens);
                sub_cached = sub_cached.saturating_add(g.cached_input_tokens);
                sub_cost += cost;
                subagents.push(SubagentUsageDto {
                    agent_id: g.agent_id.clone(),
                    input_tokens: g.input_tokens,
                    output_tokens: g.output_tokens,
                    cost_usd: cost,
                    runs: g.runs,
                });
            }

            // Top-level totals = orchestrator + all sub-agents.
            ThreadTokenUsageResponse {
                thread_id: request.thread_id.clone(),
                input_tokens: s.input_tokens.saturating_add(sub_in),
                output_tokens: s.output_tokens.saturating_add(sub_out),
                cached_input_tokens: s.cached_input_tokens.saturating_add(sub_cached),
                cost_usd: orchestrator_cost + sub_cost,
                turn_count: s.turn_count,
                last_turn_input_tokens: s.last_turn_input_tokens,
                last_turn_output_tokens: s.last_turn_output_tokens,
                context_window,
                model: s.model,
                updated: Some(s.updated),
                has_usage: true,
                subagents,
            }
        }
        None => ThreadTokenUsageResponse {
            thread_id: request.thread_id.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            cost_usd: 0.0,
            turn_count: 0,
            last_turn_input_tokens: 0,
            last_turn_output_tokens: 0,
            context_window: 0,
            model: None,
            updated: None,
            has_usage: false,
            subagents: Vec::new(),
        },
    };

    let has_usage = response.has_usage;
    Ok(envelope(
        response,
        Some(counts([("has_usage", usize::from(has_usage))])),
        None,
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
