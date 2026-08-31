# Relay

`src/relay/` owns the connector contract used when an external platform
connector talks to a gateway instead of running as a direct in-process provider.

## Descriptor

`CapabilityDescriptor` is the handshake payload that describes a connector:

- `contract_version`
- `platform`
- `label`
- `max_message_length`
- `supports_draft_streaming`
- `supports_edit`
- `supports_threads`
- `markdown_dialect`
- `len_unit`
- `emoji`
- `platform_hint`
- `pii_safe`

`RelayPlatformEntry` mirrors the platform metadata needed to build a descriptor.
`RelayDescriptorOptions` supplies live adapter capability bits.

## Frames

Gateway-to-connector frames:

- `hello`
- `outbound`
- `interrupt`
- `going_idle`
- `inbound_ack`

Connector-to-gateway frames:

- `descriptor`
- `inbound`
- `outbound_result`
- `interrupt_inbound`
- `going_idle_ack`
- `passthrough_forward`

Inbound and passthrough frames can carry a `bufferId`. The frame helpers can
derive the matching inbound acknowledgement. Authenticated inbound events strip
forged relay-trust fields from untrusted payloads before marking the event as
delivered through an authenticated relay.

## Actions

`relay_send_action_from_outbound_intent` projects a normalized
`ChannelOutboundIntent` into the relay `send` action:

- `chat_id` comes from `conversation_id`
- `content` is derived from the outbound payload
- `reply_to` carries reply linkage
- metadata carries idempotency, channel, conversation, durability, thread, and
  non-text payload details

## Auth

The relay auth module provides signed token and delivery payload helpers:

- upgrade token creation and verification
- delivery signature creation and verification
- timestamp skew and TTL defaults
- stable header names for delivery signatures and timestamps

## Transport

The transport module defines the reconnectable frame I/O boundary:

- `RelayFrameIo`
- `RelayFrameDialer`
- `RelayTransport`
- `RelayReconnectPolicy`
- inbound, interrupt, and passthrough handlers

The `relay-websocket` feature enables the WebSocket transport module and exports
the WebSocket dialer/config helpers.

## Relay Design Rule

Relay code should not know OpenHuman runtime internals. It should authenticate,
parse frames, normalize inbound events, project outbound intents, and delegate
host-owned behavior through the same channel contracts as direct providers.
