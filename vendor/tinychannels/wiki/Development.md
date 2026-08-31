# Development

This page captures the local workflow for TinyChannels contributors.

## Required Checks

Run from the repository root:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo build --all-targets
cargo test
```

Use `cargo fmt` before committing Rust changes.

## Documentation Rules

Keep docs source-grounded:

- public API docs should match the exports in `src/lib.rs`
- architecture notes should match `docs/spec/README.md`
- module docs should describe the actual module boundary
- provider docs should distinguish implemented providers from planned setup UI
  definitions

Keep Markdown files at 500 lines or fewer. Split large topics into focused
pages and link them from the relevant index.

## Adding A Provider

1. Read the existing provider module closest to the target platform.
2. Choose the right surface: legacy `Channel`, normalized `ChannelAdapter`, or
   relay connector.
3. Keep secrets in host-owned config or `SecretRef`; do not store secret values
   inside normalized channel types.
4. Normalize inbound payloads into `ChannelInboundEnvelope`.
5. Build outbound sends as `ChannelOutboundIntent`.
6. Preserve platform ids in `MessageReceipt`.
7. Advertise only supported capabilities.
8. Add tests for auth metadata, serialization, idempotency, threading, delivery,
   and provider-specific edge cases.

## Adding Host Capabilities

If a provider needs a new OpenHuman runtime service:

1. Add a small object-safe trait or DTO under `src/host/`.
2. Add an optional accessor to `ChannelHost`.
3. Add a bit to `HostCapabilities` if providers need to branch on it.
4. Keep provider behavior graceful when the capability is absent.
5. Add tests with `NoopHost` and a focused fake host implementation.

## Adding Relay Behavior

Relay changes should include focused tests for:

- descriptor JSON stability
- frame serialization and deserialization
- forged relay-trust field stripping
- auth token/signature verification
- outbound intent to relay action projection
- reconnect and timeout behavior for transport changes

## Git Wiki Workflow

The `wiki/` directory is a separate Git repository tracked by the parent as a
submodule. Commit wiki page changes inside `wiki/`, then stage the parent
gitlink update and `.gitmodules` changes in the root repository.

Useful commands:

```sh
git -C wiki status
git -C wiki add .
git -C wiki commit -m "Document TinyChannels wiki"
git add .gitmodules wiki
git status
```
