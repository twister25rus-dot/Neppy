# Architecture

TinyChannels separates channel-specific I/O from OpenHuman-owned runtime
behavior. Providers can live in this crate because they depend on portable
contracts instead of application internals.

## Layered Model

```text
                         +----------------------------+
                         | OpenHuman application      |
                         | config, auth, harness, UI  |
                         +--------------+-------------+
                                        |
                                        v
                         +----------------------------+
                         | ChannelBackend             |
                         | connect/status/test/send   |
                         +--------------+-------------+
                                        |
                                        v
+----------------+       +----------------------------+       +----------------+
| Controller UI  |------>| ChannelManager             |------>| Channel defs   |
| setup/status   |       | validation + delegation    |       | auth fields    |
+----------------+       +----------------------------+       +----------------+
                                        |
                                        v
                         +----------------------------+
                         | ProviderContext            |
                         | config + HTTP + host       |
                         +--------------+-------------+
                                        |
                    +-------------------+-------------------+
                    |                                       |
                    v                                       v
        +------------------------+              +------------------------+
        | Direct providers       |              | Relay connectors       |
        | Slack, Discord, etc.   |              | descriptor + frames    |
        +-----------+------------+              +-----------+------------+
                    |                                       |
                    v                                       v
        +------------------------+              +------------------------+
        | Channel contracts      |              | Relay action contract  |
        | envelope, intent,      |              | send, passthrough,     |
        | receipt, capabilities  |              | inbound ack            |
        +-----------+------------+              +-----------+------------+
                    |                                       |
                    +-------------------+-------------------+
                                        |
                                        v
                         +----------------------------+
                         | Delivery queue             |
                         | durability + retry policy  |
                         +----------------------------+
```

## Boundary Rules

TinyChannels owns portable contracts:

- what a channel message looks like before and after normalization
- what a provider can advertise as capabilities
- what a UI setup definition needs to render auth forms
- what a relay connector sends over the socket
- what durable delivery state means

OpenHuman owns host-specific behavior:

- user accounts and credential storage
- REST/JWT/session authorization
- model and harness execution
- conversation memory and run ledger storage
- event bus fanout and process lifecycle management

The dependency direction is deliberate: providers call small host capability
traits, and OpenHuman implements those traits.

## Main Data Flow

Inbound:

```text
platform event
  -> provider or relay connector
  -> ChannelInboundEnvelope
  -> ChannelInboundSink or ChannelHost TurnDispatcher
  -> OpenHuman harness turn
  -> ChannelOutputEvent stream
```

Outbound:

```text
agent output or controller send
  -> ChannelOutboundIntent
  -> durability negotiation
  -> provider send or relay send action
  -> MessageReceipt
  -> delivery ack or retry state
```

Setup and control:

```text
UI action
  -> ChannelManager
  -> ChannelDefinition credential validation
  -> ChannelBackend implementation in OpenHuman
  -> ChannelConnectionResult / ChannelStatusEntry / ChannelTestResult
```

## Compatibility Strategy

The crate still supports legacy shapes (`ChannelMessage`, `SendMessage`, and raw
controller JSON payloads), but newer code should normalize as early as possible:

- project inbound legacy messages into `ChannelInboundEnvelope`
- build outbound sends as `ChannelOutboundIntent`
- carry idempotency keys through `SendMessage` when using legacy providers
- return `MessageReceipt` from direct adapters where possible

This lets OpenHuman migrate provider by provider without requiring a single
large runtime rewrite.
