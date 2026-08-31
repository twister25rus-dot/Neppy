# Capabilities

This page is the master index of TinyChannels functionality.

## Channel Contracts

- `ChannelMessage` - legacy received/sent message shape.
- `SendMessage` - legacy outbound message request with subject, thread, and
  idempotency support.
- `ChannelInboundEnvelope` - normalized inbound message event.
- `ChannelOutboundIntent` - normalized outbound send request.
- `OutboundPayload` - text, media, voice, files, poll, presentation blocks, or
  native channel data.
- `MessageReceipt` - normalized platform result for one logical send.
- `ChannelStaticCapabilities` - chat, poll, reaction, edit, reply, thread,
  media, command, and streaming support flags.
- `ChannelPresentationCapabilities` - text limits, length units, markdown
  dialect, action layout limits, and edit support.

See [Channel Contracts](Channel-Contracts).

## Adapter Contracts

- `ChannelAdapter` - base start/stop/send/status adapter surface.
- `ChannelInboundSink` - normalized inbound envelope sink.
- `ChannelReceiveAckPolicy` - when inbound receive should be acknowledged.
- Optional adapter traits:
  - `ChannelSetup`
  - `ChannelDirectory`
  - `ChannelResolver`
  - `ChannelTyping`
  - `ChannelReaction`
  - `ChannelEdit`
  - `ChannelDelete`
  - `ChannelStreamingDraft`

## Host Capabilities

- `ChannelHost` - optional capability aggregator.
- `ProviderContext` - standard provider construction context.
- `HostCapabilities` - cheap capability descriptor.
- `TurnDispatcher` - run a channel turn through the host harness.
- `Transcriber` and `SpeechSynthesizer` - voice input/output.
- `ApprovalGate` and `ReactionGate` - human approval and response gating.
- `ConversationStore` - durable per-session history.
- `EventSink` - host domain events.
- `LifecycleRegistry` - provider shutdown hooks.
- `RunLedger` - run and telemetry records.

See [Host Boundary](Host-Boundary).

## Relay Contracts

- `CapabilityDescriptor` - connector handshake descriptor.
- `RelayPlatformEntry` and `RelayDescriptorOptions` - descriptor projection
  inputs.
- `GatewayToConnectorFrame` - gateway frames: hello, outbound, interrupt,
  going idle, inbound ack.
- `ConnectorToGatewayFrame` - connector frames: descriptor, inbound,
  outbound result, interrupt inbound, going idle ack, passthrough forward.
- Relay auth helpers - signed upgrade and delivery payload verification.
- `relay_send_action_from_outbound_intent` - outbound intent to relay `send`
  action projection.
- `RelayTransport` and related traits - reconnectable frame transport.

See [Relay](Relay).

## Delivery Capabilities

- `DeliveryDurability` - required, best effort, or disabled.
- `DurableFinalDeliveryCapability` - text, media, poll, payload, reply, thread,
  reconciliation, after-send, and after-commit capability flags.
- `negotiate_delivery_durability` - downgrade missing durable capabilities to
  best effort.
- `compute_backoff_ms` - OpenClaw-compatible recovery backoff.
- `recover_pending_deliveries` - drain pending entries in enqueue order.
- Unknown-send reconciliation for crash or transport ambiguity after platform
  send starts.

See [Delivery](Delivery).

## Controller And Backend Capabilities

- `ChannelDefinition` - UI setup metadata.
- `ChannelAuthMode` - API key, bot token, OAuth, managed DM.
- `ChannelCapability` - runtime UI capability list.
- `ChannelManager` - validation plus backend delegation.
- `ChannelBackend` - OpenHuman-owned implementation for connect, disconnect,
  status, test, send, reactions, threads, Telegram login, Discord link, and
  default-channel management.

See [Controller API](Controller-API).

## Built-In Providers

Exported providers include:

- `DingTalkChannel`
- `DiscordChannel`
- `EmailChannel`
- `IMessageChannel`
- `IrcChannel`
- `LarkChannel`
- `LinqChannel`
- `MattermostChannel`
- `QQChannel`
- `SignalChannel`
- `SlackChannel`
- `WhatsAppChannel`
- `YuanbaoChannel`

See [Providers](Providers).
