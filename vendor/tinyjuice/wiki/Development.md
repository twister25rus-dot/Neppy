# Development

TinyJuice is a Rust 2024 crate. Keep changes small, explicit, and testable.

## Required Checks

Run before opening a PR:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Useful additional checks:

```sh
cargo build --all-targets
cargo run --example passthrough
cargo bench
```

## Test Structure

- Unit tests live next to modules or in module-local `test.rs` files.
- Rule-engine fixture tests live in `tests/fixtures/*.fixture.json`.
- End-to-end profile and CCR behavior lives in `tests/e2e_tool_output.rs`.
- Hot-path performance benches live in `benches/compression.rs`.

## Documentation Rules

Update README or wiki pages when changing:

- public API names
- compressor behavior
- content detection
- recovery marker semantics
- agent profiles
- rule loading behavior
- analytics schema
- OpenHuman adapter contracts

Keep README marketing-heavy and high-level. Keep wiki pages technical and
agent-friendly.

## Module Conventions

Prefer:

- small modules
- `types.rs` for shared data
- `test.rs` for module-local tests
- explicit pass-through behavior
- deterministic reducers
- fixtures for command-rule regressions

Avoid:

- OpenHuman runtime dependencies in the core crate without a feature or adapter
  boundary
- raw-content logging
- broad refactors mixed with behavior changes
- compression percentage claims without benchmark fixtures

## Adding Public Behavior

1. Read the source module first.
2. Add focused tests around the behavior.
3. Keep failure modes pass-through safe.
4. Update wiki and README if public behavior or integration contracts changed.
5. Run the required checks.

## Agent Notes

- Use `rg` to find APIs and rules quickly.
- Use fixture tests for exact reducer output.
- Use e2e tests for profile, CCR, and recovery behavior.
- Preserve unrelated local changes.
- If the repo is dirty, stage or commit only the requested slice.
