# Host Boundary

The host boundary lets rich channel providers use OpenHuman runtime services
without depending on OpenHuman application modules.

## ProviderContext

Every provider can be constructed from `ProviderContext`:

- `host` - an `Arc<dyn ChannelHost>`
- `channels_config` - resolved channel configuration
- `http_client` - shared HTTP client for REST-backed providers

Lean providers can ignore `host` and use `NoopHost`. Rich providers can ask for
only the capabilities they need.

## ChannelHost

`ChannelHost` is an optional capability aggregator. Every accessor defaults to
`None`, so OpenHuman can implement capabilities incrementally:

- `dispatcher` - run agent turns and stream output
- `transcriber` - speech-to-text
- `synthesizer` - text-to-speech
- `approvals` - human-in-the-loop approval
- `reactions` - response gating for busy channels
- `conversations` - durable conversation history
- `memory` - semantic memory recall
- `events` - domain event publishing
- `lifecycle` - shutdown hook registration
- `ledger` - run and telemetry records

`HostCapabilities` is a cheap descriptor providers can check up front to branch
behavior or advertise accurate capabilities downstream.

## Turn Dispatch

`TurnDispatcher` is the core rich-provider capability. A provider assembles a
`ChannelTurn`, wraps it in a `DispatchRequest`, and receives a cancellable
`TurnHandle` with a stream of `ChannelOutputEvent` values.

Dispatch options include:

- stream partial output or final-only output
- allow or disallow tools
- model override
- locale hint
- timeout

Admission errors return from `dispatch`. Per-turn runtime failures should be
emitted through the output event stream so the provider can render or log them
consistently.

## Harness Types

`src/harness/` owns the typed bridge between channel events and OpenHuman
harness output:

- `ChannelTurn` - the assembled inbound turn.
- `TurnAdmissionVerdict` - whether a turn should be accepted.
- `ChannelOutputEvent` - output stream items such as final text, deltas, tool
  progress, approvals, media, cancellation, or native events.
- `HarnessLifecycleEvent` and `InboundLifecycleStage` - lifecycle markers.
- `translate_output_event` - helper for converting output events for a channel
  bridge.

## Design Rule

Providers should never reach around `ChannelHost` into app-specific runtime
objects. If a provider needs a new host service, add a small object-safe trait
or DTO to the host boundary and let OpenHuman implement it.
