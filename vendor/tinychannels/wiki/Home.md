# TinyChannels Wiki

**TinyChannels is a Rust channel and messaging contract library for OpenHuman.**
It owns the portable channel types, provider adapters, relay contracts, delivery
queue policy, controller metadata, and host boundary that let OpenHuman connect
external messaging surfaces to its harness without pulling application internals
into each provider.

The repository `README.md` is the short product overview. This wiki is the
implementation-oriented companion: what each surface owns, how to set up the
crate, how providers fit together, and where OpenHuman plugs in.

- crate: `tinychannels` (Rust 2024 edition, GPL-3.0-only)
- crates.io: <https://crates.io/crates/tinychannels>
- docs.rs: <https://docs.rs/tinychannels>
- source: <https://github.com/tinyhumansai/tinychannels>

## Why TinyChannels Exists

OpenHuman talks to people and agents through many channel surfaces: Discord,
Slack, email, iMessage, Lark, DingTalk, IRC, Signal, WhatsApp, QQ, Yuanbao, and
future relay-backed platforms. Each surface has different auth, addressing,
threading, formatting, delivery, and lifecycle rules.

TinyChannels keeps those differences behind typed contracts:

- inbound messages become `ChannelInboundEnvelope`
- outbound sends become `ChannelOutboundIntent`
- provider sends return normalized `MessageReceipt` values
- durable delivery is negotiated before retrying or downgrading
- rich providers call back into OpenHuman through `ChannelHost`
- external connectors speak the relay frame and descriptor contracts

## The Main Surfaces

1. **[Channel Contracts](Channel-Contracts)** (`src/channel/`, `src/traits.rs`)
   - normalized inbound envelopes, outbound intents, receipts, provider
   capability flags, and the legacy `Channel` trait.
2. **[Providers](Providers)** (`src/providers/`)
   - portable provider implementations for direct channel integrations.
3. **[Host Boundary](Host-Boundary)** (`src/host/`, `src/harness/`)
   - optional host capabilities for turn dispatch, speech, approvals,
   conversation history, events, lifecycle hooks, and telemetry.
4. **[Relay](Relay)** (`src/relay/`)
   - connector descriptor, auth, frame, transport, and action contracts for
   Hermes/OpenClaw-style platform relays.
5. **[Delivery](Delivery)** (`src/delivery/`)
   - durable outbound delivery queue policy, retry state, unknown-send
   reconciliation, and capability negotiation.
6. **[Controller API](Controller-API)** (`src/controllers/`, `src/backend.rs`)
   - UI-facing channel definitions and the `ChannelBackend` delegation layer
   that OpenHuman implements.

## Page Index

Getting started

- [Home](Home) - this page.
- [Capabilities](Capabilities) - master index of crate functionality.
- [Quick Start](Quick-Start) - install, test, and navigate the crate.

Concepts

- [Architecture](Architecture) - how the surfaces fit together end to end.
- [Channel Contracts](Channel-Contracts) - normalized inbound/outbound types.
- [Host Boundary](Host-Boundary) - how providers call back into OpenHuman.

Modules

- [Providers](Providers) - built-in provider modules and their role.
- [Relay](Relay) - connector handshakes, frames, auth, and transport.
- [Delivery](Delivery) - durable delivery queue behavior.
- [Controller API](Controller-API) - setup/status/send backend contracts.

Contributing

- [Development](Development) - local checks, docs rules, and extension notes.

## Repository Links

- [Main repository](https://github.com/tinyhumansai/tinychannels)
- [System specification](https://github.com/tinyhumansai/tinychannels/blob/main/docs/spec/README.md)
- [Porting plan](https://github.com/tinyhumansai/tinychannels/blob/main/docs/spec/tinychannels-execution-plan.md)
- [OpenClaw and Hermes porting spec](https://github.com/tinyhumansai/tinychannels/blob/main/docs/spec/openclaw-hermes-channel-porting.md)
