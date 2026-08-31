# Quick Start

The shortest path from zero to a working TinyChannels checkout, plus the crate
layout you will navigate when adding or porting channel providers.

## Install From crates.io

Add the crate to a Rust project:

```sh
cargo add tinychannels
```

The default feature set is intentionally small. The `relay-websocket` feature
keeps the WebSocket relay transport module behind an opt-in flag while the core
relay frame and descriptor contracts remain available.

```sh
cargo add tinychannels --features relay-websocket
```

## Clone, Test, And Build

To work against the source:

```sh
git clone https://github.com/tinyhumansai/tinychannels.git
cd tinychannels
cargo test
```

Useful local checks:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build --all-targets
cargo test
```

## Minimal Usage Shape

Lean providers implement the legacy `Channel` trait when all they need is
receive, send, listen, and health:

```rust
use async_trait::async_trait;
use tinychannels::{Channel, ChannelMessage, SendMessage};

struct MyChannel;

#[async_trait]
impl Channel for MyChannel {
    fn name(&self) -> &str {
        "my-channel"
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        println!("send {} to {}", message.content, message.recipient);
        Ok(())
    }

    async fn listen(
        &self,
        tx: tokio::sync::mpsc::Sender<ChannelMessage>,
    ) -> anyhow::Result<()> {
        let _ = tx;
        Ok(())
    }
}
```

Newer provider and relay work should prefer the normalized channel contract:

```rust
use tinychannels::{
    ChannelOutboundIntent, DeliveryDurability, OutboundPayload,
};

let intent = ChannelOutboundIntent {
    idempotency_key: "send:user-42:hello".to_string(),
    channel_id: "discord".to_string(),
    conversation_id: "channel-123".to_string(),
    reply_to_id: None,
    thread_id: None,
    durability: DeliveryDurability::BestEffort,
    payload: OutboundPayload::Text {
        text: "hello".to_string(),
    },
};
```

## Crate Layout

- `src/lib.rs` - public export surface.
- `src/traits.rs` - legacy `Channel`, `ChannelMessage`, and `SendMessage`.
- `src/channel/` - normalized channel references, envelopes, intents, receipts,
  adapter traits, capabilities, and session-key helpers.
- `src/providers/` - direct provider implementations.
- `src/host/` - host capability traits used by rich providers.
- `src/harness/` - typed turn and output-event contracts.
- `src/relay/` - connector descriptors, auth, frames, actions, and transport.
- `src/delivery/` - durable outbound queue state machine and retry policy.
- `src/controllers/` - setup UI definitions and backend response DTOs.
- `src/backend.rs` - `ChannelBackend` and `ChannelManager`.
- `docs/spec/` - system specs and porting plans.
- `wiki/` - this GitHub wiki source.

## Next Steps

- [Architecture](Architecture) - read the full system map.
- [Channel Contracts](Channel-Contracts) - learn the normalized message types.
- [Providers](Providers) - see how built-in provider modules are organized.
- [Development](Development) - follow the repo checks and docs rules.
