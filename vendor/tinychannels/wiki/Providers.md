# Providers

`src/providers/` contains portable provider modules. The public exports are
centralized from `src/lib.rs` and `src/providers/mod.rs`.

## Exported Providers

| Provider | Export |
|----------|--------|
| DingTalk | `DingTalkChannel` |
| Discord | `DiscordChannel` |
| Email | `EmailChannel` |
| iMessage | `IMessageChannel` |
| IRC | `IrcChannel`, `IrcChannelConfig` |
| Lark | `LarkChannel` |
| Linq | `LinqChannel`, `verify_linq_signature` |
| Mattermost | `MattermostChannel` |
| QQ | `QQChannel` |
| Signal | `SignalChannel` |
| Slack | `SlackChannel` |
| WhatsApp | `WhatsAppChannel` |
| Yuanbao | `YuanbaoChannel` |

## Provider Shapes

TinyChannels supports several provider shapes:

- lean direct providers that implement `Channel`
- richer providers that consume `ProviderContext` and host capabilities
- providers with optional adapter traits for setup, directory, typing, reaction,
  edit, delete, or draft streaming support
- relay-backed connectors that project normalized intents into relay actions

Provider work should normalize platform-specific events into
`ChannelInboundEnvelope` as early as possible and convert outbound work into
`ChannelOutboundIntent` before delivery or relay projection.

## Controller Registry

The setup UI registry currently exposes definitions for:

- Telegram
- Discord
- Web
- iMessage
- Lark
- DingTalk
- Email
- Yuanbao

That registry is intentionally smaller than the config/provider set. Some
config-backed providers remain hidden until their setup flows are promoted into
`ChannelDefinition` metadata.

## Provider Implementation Checklist

When adding or porting a provider:

1. Add provider configuration to `src/config.rs` if needed.
2. Add or update a provider module under `src/providers/`.
3. Export the provider from `src/providers/mod.rs` and `src/lib.rs` only if it
   is part of the public API.
4. Normalize inbound platform events into `ChannelInboundEnvelope`.
5. Build outbound sends as `ChannelOutboundIntent` or preserve legacy
   `SendMessage` idempotency through `ChannelSendExt`.
6. Return `MessageReceipt` where the provider can identify platform message ids.
7. Advertise honest capabilities.
8. Add setup metadata to `src/controllers/definitions.rs` only when the UI flow
   is ready.
9. Add focused tests for serialization, routing, delivery, and provider edge
   cases.

## Rich Provider Rule

If a provider needs harness turns, speech, approvals, history, events, shutdown,
or telemetry, use `ProviderContext::host`. Do not import OpenHuman application
modules into TinyChannels provider code.
