//! Endpoint tests for the messaging-family APIs: `MessagesApi`,
//! `ConversationsApi`, `ChannelsApi`, `InboxApi`, and `KeysApi`. Each test
//! points the client at a catch-all mock, invokes a method, and asserts the
//! request method and path. Response bodies are permissive — the goal is to
//! exercise request construction, auth signing, and the response pipeline.

mod common;

use common::*;
use serde_json::json;

use tinyplace::api::channels::{ChannelInput, ChannelMessageInput, ChannelQueryParams};
use tinyplace::api::inbox::{InboxClearParams, InboxQueryParams};
use tinyplace::types::{
    ConversationCreateRequest, ConversationMessageCreateRequest, ConversationQueryParams,
    ConversationUpdateRequest, MessageEnvelope, PreKeysRequest, SignedPreKeyRequest,
};

// --- MessagesApi ------------------------------------------------------------

#[tokio::test]
async fn messages_list() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.messages.list("@alice", None).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/messages"));
}

#[tokio::test]
async fn messages_send() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let envelope: MessageEnvelope = serde_json::from_value(json!({
        "id": "m1",
        "from": "@alice",
        "to": "@bob",
        "timestamp": "",
        "deviceId": 1,
        "type": "CIPHERTEXT",
        "body": "deadbeef",
    }))
    .unwrap();
    let _ = client.messages.send(envelope).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/messages"));
}

#[tokio::test]
async fn messages_acknowledge() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.messages.acknowledge("m1", "@alice").await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/messages"));
}

// --- ConversationsApi -------------------------------------------------------

#[tokio::test]
async fn conversations_list() {
    let server = any_ok(json!({ "conversations": [] })).await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .list(Some(&ConversationQueryParams::default()))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/conversations"));
}

#[tokio::test]
async fn conversations_create() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let request: ConversationCreateRequest = serde_json::from_value(json!({
        "type": "chat",
        "creator": "@alice",
    }))
    .unwrap();
    let _ = client.conversations.create(request).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/conversations"));
}

#[tokio::test]
async fn conversations_get() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.conversations.get("conv1").await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("conv1"));
}

#[tokio::test]
async fn conversations_update() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .update(
            "conv1",
            &ConversationUpdateRequest::default(),
            Some("@alice"),
        )
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("conv1"));
}

#[tokio::test]
async fn conversations_remove() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.conversations.remove("conv1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("conv1"));
}

#[tokio::test]
async fn conversations_join() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.conversations.join("conv1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/join"));
}

#[tokio::test]
async fn conversations_leave() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.conversations.leave("conv1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/leave"));
}

#[tokio::test]
async fn conversations_members() {
    let server = any_ok(json!({ "members": [] })).await;
    let client = client_for(&server);
    let _ = client.conversations.members("conv1").await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/members"));
}

#[tokio::test]
async fn conversations_add_member() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .add_member("conv1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/members"));
}

#[tokio::test]
async fn conversations_remove_member() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .remove_member("conv1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/members"));
}

#[tokio::test]
async fn conversations_approve_member() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .approve_member("conv1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/approve"));
}

#[tokio::test]
async fn conversations_reject_member() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .reject_member("conv1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/reject"));
}

#[tokio::test]
async fn conversations_list_messages() {
    let server = any_ok(json!({ "messages": [] })).await;
    let client = client_for(&server);
    let _ = client.conversations.list_messages("conv1", None).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/messages"));
}

#[tokio::test]
async fn conversations_post_message() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let message: ConversationMessageCreateRequest = serde_json::from_value(json!({
        "author": "@alice",
        "body": "hi",
    }))
    .unwrap();
    let _ = client.conversations.post_message("conv1", message).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/messages"));
}

#[tokio::test]
async fn conversations_delete_message() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .delete_message("conv1", "msg1", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/messages"));
}

#[tokio::test]
async fn conversations_add_moderator() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .add_moderator("conv1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/moderators"));
}

#[tokio::test]
async fn conversations_remove_moderator() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .remove_moderator("conv1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/moderators"));
}

#[tokio::test]
async fn conversations_add_publisher() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .add_publisher("conv1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/publishers"));
}

#[tokio::test]
async fn conversations_remove_publisher() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .conversations
        .remove_publisher("conv1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/publishers"));
}

// --- ChannelsApi ------------------------------------------------------------

#[tokio::test]
async fn channels_list() {
    let server = any_ok(json!({ "channels": [] })).await;
    let client = client_for(&server);
    let _ = client
        .channels
        .list(Some(&ChannelQueryParams::default()))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/channels"));
}

#[tokio::test]
async fn channels_create() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .channels
        .create(ChannelInput {
            name: Some("general".into()),
            creator: Some("@alice".into()),
            ..Default::default()
        })
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/channels"));
}

#[tokio::test]
async fn channels_get() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.channels.get("chan1").await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("chan1"));
}

#[tokio::test]
async fn channels_update() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .channels
        .update("chan1", &ChannelInput::default(), Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("chan1"));
}

#[tokio::test]
async fn channels_remove() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.channels.remove("chan1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("chan1"));
}

#[tokio::test]
async fn channels_join() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.channels.join("chan1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/join"));
}

#[tokio::test]
async fn channels_leave() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.channels.leave("chan1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/leave"));
}

#[tokio::test]
async fn channels_list_messages() {
    let server = any_ok(json!({ "messages": [] })).await;
    let client = client_for(&server);
    let _ = client.channels.list_messages("chan1", None, None).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/messages"));
}

#[tokio::test]
async fn channels_post_message() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .channels
        .post_message(
            "chan1",
            ChannelMessageInput {
                author: Some("@alice".into()),
                body: Some("hi".into()),
                ..Default::default()
            },
        )
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/messages"));
}

#[tokio::test]
async fn channels_delete_message() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .channels
        .delete_message("chan1", "msg1", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/messages"));
}

#[tokio::test]
async fn channels_members() {
    let server = any_ok(json!({ "members": [] })).await;
    let client = client_for(&server);
    let _ = client.channels.members("chan1").await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/members"));
}

#[tokio::test]
async fn channels_moderators() {
    let server = any_ok(json!({ "moderators": [] })).await;
    let client = client_for(&server);
    let _ = client.channels.moderators("chan1").await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/moderators"));
}

#[tokio::test]
async fn channels_add_moderator() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .channels
        .add_moderator("chan1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "POST");
    assert!(req.url.path().contains("/moderators"));
}

#[tokio::test]
async fn channels_remove_moderator() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .channels
        .remove_moderator("chan1", "@bob", Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/moderators"));
}

#[tokio::test]
async fn channels_trending() {
    let server = any_ok(json!({ "channels": [] })).await;
    let client = client_for(&server);
    let _ = client.channels.trending(None).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/trending"));
}

#[tokio::test]
async fn channels_categories() {
    let server = any_ok(json!({ "categories": [] })).await;
    let client = client_for(&server);
    let _ = client.channels.categories().await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/categories"));
}

// --- InboxApi ---------------------------------------------------------------

#[tokio::test]
async fn inbox_list() {
    let server = any_ok(json!({ "items": [], "unreadCount": 0, "totalCount": 0 })).await;
    let client = client_for(&server);
    let _ = client
        .inbox
        .list(Some(&InboxQueryParams::default()), Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/inbox"));
}

#[tokio::test]
async fn inbox_get() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.inbox.get("item1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("item1"));
}

#[tokio::test]
async fn inbox_search() {
    let server = any_ok(json!({ "items": [] })).await;
    let client = client_for(&server);
    let _ = client.inbox.search("hello", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/inbox/search"));
}

#[tokio::test]
async fn inbox_counts() {
    let server = any_ok(json!({
        "unread": 0, "read": 0, "archived": 0, "byType": {}, "urgent": 0
    }))
    .await;
    let client = client_for(&server);
    let _ = client.inbox.counts(Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/inbox/counts"));
}

#[tokio::test]
async fn inbox_mark_read() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.inbox.mark_read("item1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/read"));
}

#[tokio::test]
async fn inbox_mark_read_bulk() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .inbox
        .mark_read_bulk(&["item1".to_string()], Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/inbox/read"));
}

#[tokio::test]
async fn inbox_mark_all_read() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .inbox
        .mark_all_read(Some(&InboxClearParams::default()), Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/inbox/read-all"));
}

#[tokio::test]
async fn inbox_archive() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.inbox.archive("item1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/archive"));
}

#[tokio::test]
async fn inbox_archive_bulk() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .inbox
        .archive_bulk(&["item1".to_string()], Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/inbox/archive"));
}

#[tokio::test]
async fn inbox_unarchive() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.inbox.unarchive("item1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/unarchive"));
}

#[tokio::test]
async fn inbox_remove() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.inbox.remove("item1", Some("@alice")).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("item1"));
}

#[tokio::test]
async fn inbox_remove_bulk() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .inbox
        .remove_bulk(&["item1".to_string()], Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/inbox"));
}

#[tokio::test]
async fn inbox_clear() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client
        .inbox
        .clear(Some(&InboxClearParams::default()), Some("@alice"))
        .await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "DELETE");
    assert!(req.url.path().contains("/inbox/clear"));
}

// --- KeysApi ----------------------------------------------------------------

#[tokio::test]
async fn keys_get_bundle() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.keys.get_bundle("@alice").await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/bundle"));
}

#[tokio::test]
async fn keys_health() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let _ = client.keys.health("@alice").await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "GET");
    assert!(req.url.path().contains("/health"));
}

#[tokio::test]
async fn keys_upload_pre_keys() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let request: PreKeysRequest = serde_json::from_value(json!({
        "preKeys": [{ "keyId": "k1", "publicKey": "pk" }],
    }))
    .unwrap();
    let _ = client.keys.upload_pre_keys("@alice", &request).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/prekeys"));
}

#[tokio::test]
async fn keys_rotate_signed_pre_key() {
    let server = any_empty_ok().await;
    let client = client_for(&server);
    let request: SignedPreKeyRequest = serde_json::from_value(json!({
        "signedPreKey": { "keyId": "k1", "publicKey": "pk" },
    }))
    .unwrap();
    let _ = client.keys.rotate_signed_pre_key("@alice", &request).await;
    let req = only_request(&server).await;
    assert_eq!(req.method.as_str(), "PUT");
    assert!(req.url.path().contains("/signed-prekey"));
}
