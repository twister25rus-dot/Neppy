# Channel Contracts

The channel contract layer is the core of TinyChannels. It keeps provider I/O
portable while preserving enough platform detail for OpenHuman to route,
thread, retry, and observe messages correctly.

## Legacy Channel Trait

`src/traits.rs` defines the legacy direct-provider surface:

- `ChannelMessage` - received message with id, sender, reply target, content,
  channel id, timestamp, and optional thread id.
- `SendMessage` - outbound message with content, recipient, optional subject,
  optional thread id, and optional idempotency key.
- `Channel` - object-safe async provider trait with `send`, `listen`,
  `health_check`, typing indicators, reactions, draft updates, and proactive
  target support.
- `ChannelSendExt` - helper that builds a deterministic outbound intent
  idempotency key before delegating to legacy `send`.

This surface is intentionally still available so existing OpenHuman providers
can migrate incrementally.

## Normalized Inbound Envelope

`ChannelInboundEnvelope` is the newer inbound contract. It includes:

- `channel` - `ChannelRef` with channel id and optional account id.
- `message_id` - provider message id.
- `conversation` - `ConversationRef` with kind, id, scope, parent, thread, and
  topic fields.
- `sender` - `SenderRef` with ids, name, bot flag, and roles.
- `text` - message text.
- `access` - DM/group/mention authorization facts.
- `media` - ordered media references.
- `raw` - optional provider payload for diagnostics or adapter-specific needs.

Legacy helpers project between `ChannelMessage` and `ChannelInboundEnvelope`.
Telegram topic ids are kept distinct from generic thread ids when projecting
legacy `thread_ts` values.

## Normalized Outbound Intent

`ChannelOutboundIntent` is the logical outbound send request:

- `idempotency_key`
- `channel_id`
- `conversation_id`
- `reply_to_id`
- `thread_id`
- `durability`
- `payload`

Payload variants cover text, media, voice, files, polls, presentation blocks,
and native channel data. Legacy JSON send payloads are preserved as
`NativeChannelData` while still gaining deterministic idempotency keys.

## Receipts

`MessageReceipt` normalizes platform send results:

- primary platform message id
- all platform message ids produced by a logical send
- per-part receipts for split or multi-part messages
- thread and reply linkage
- edit and delete tokens
- raw source results

The receipt helpers accept heterogeneous provider results and extract stable
ids from message, chat, room, conversation, JID, and poll fields.

## Capabilities

Provider capability data is split by concern:

- `ChannelStaticCapabilities` describes feature support such as polls,
  reactions, edits, threads, media, and native commands.
- `ChannelPresentationCapabilities` describes text limits, length unit,
  markdown dialect, edit support, and action layout constraints.
- `DurableFinalDeliveryCapability` describes whether a provider can guarantee
  durable final delivery for text, media, polls, payloads, replies, threads,
  reconciliation, hooks, batch sends, and commit timing.

## Adapter Traits

New provider work can implement `ChannelAdapter` and optional extension traits
instead of growing the legacy `Channel` trait:

- base adapter: descriptor, ack policy, start, stop, send, status
- setup validation
- directory listing
- target resolution
- typing indicators
- reactions
- edit/delete
- streaming draft upsert

Adapters send normalized inbound envelopes into a `ChannelInboundSink` and
return normalized receipts from outbound sends.
