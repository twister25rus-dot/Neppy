# Development

This is the contributor and local-development guide for `tinyagents`, the
recursive language-model (RLM) harness for Rust. TinyAgents is a typed, durable
runtime where models call models, agents call agents, and graphs run graphs, so
the code is deliberately organized into small, inspectable modules. This page
covers the toolchain, the canonical commands, the repo layout, and the testing
and documentation expectations every change is held to.

## Toolchain

- **Rust 2024 edition.** Install a stable Rust toolchain new enough to support
  the 2024 edition (`rustup update stable`). No nightly features are required.
- The crate is a single library crate rooted at `Cargo.toml` (crate name
  `tinyagents`, license `GPL-3.0-only`).
- The default build is **offline**: the OpenAI (and OpenAI-compatible) provider
  `harness::providers::openai` is compiled in by default but makes no network
  call unless invoked, and the tests that would call it skip without a key (see
  [Live / OpenAI tests](#live--openai-tests)).
- Two Cargo features gate the heavier, optional backends, both off by default:
  - `sqlite` — the embedded SQLite-backed checkpointer
    (`graph::checkpoint::SqliteCheckpointer`), pulling in vendored `rusqlite`.
  - `repl` — the embedded Rhai-backed `.ragsh` session runtime
    (`repl::session`), pulling in `rhai`.

## Canonical commands

Run these from the repository root. They mirror the CI gate and the pull-request
checklist — every one must pass before a change is ready for review.

```sh
cargo fmt --check                          # verify formatting (no changes)
cargo clippy --all-targets -- -D warnings  # lint lib + tests + examples, warnings = errors
cargo build --all-targets                  # compile every target
cargo test                                 # run the full suite (offline)
cargo run --example basic_graph            # run the bundled graph example
```

Use `cargo fmt` (without `--check`) to apply formatting before committing.
Always format with stock `rustfmt`; do not hand-tune layout.

The OpenAI adapter and its unit tests are already part of the default
`cargo test`. To additionally compile the optional backends:

```sh
cargo test --features sqlite               # compiles graph::checkpoint::sqlite
cargo test --features repl                 # compiles the Rhai .ragsh session runtime
cargo test --all-features                  # all optional backends at once
```

Tests that would make real network calls are guarded (see below), so
`cargo test` stays green without credentials.

## Repository layout

`tinyagents` follows a strict module-as-directory convention. Public API exports
are centralized in `src/lib.rs` so downstream users have one predictable surface.

```text
Cargo.toml            crate manifest, feature flags
src/lib.rs            centralized public exports
src/error.rs          TinyAgentsError + crate Result<T> alias
src/harness/          the harness: provider-neutral model calls, tools,
                        middleware, streaming, sub-agents, providers/
src/graph/            durable typed state-graph runtime (nodes, edges,
                        checkpoints, subgraphs)
src/registry/         named capability catalog bound by .rag / .ragsh
src/language/         the declarative .rag blueprint language (lexer → parser
                        → compiler)
src/repl/             the imperative .ragsh REPL (the RLM/CodeAct loop surface)
tests/                integration tests (currently serialization-focused)
examples/             runnable usage examples
docs/                 design notes and module specs
wiki/                 this developer-facing wiki
```

### The module convention

New feature areas live in **module directories**, not broad multi-purpose files.
Within each module directory:

- `mod.rs` wires the pieces together and exposes the smallest useful API.
- `types.rs` holds the module's type definitions.
- `test.rs` holds module-local unit tests (declared `#[cfg(test)] mod test;`).

Prefer small modules that do one thing extremely well. Public types and traits
are `PascalCase`; modules, files, functions, methods, fields, and locals are
`snake_case`. Prefer small, typed APIs returning the crate `Result<T>` over
panics or stringly-typed errors.

## Where documentation lives

Keep all of these aligned with code changes:

- `README.md` — marketing-forward overview and 30-second quick start.
- `docs/spec/README.md` — top-level architecture reference.
- `docs/modules/` — per-module design docs. Complex modules must include a
  module-level `README.md` explaining design, public surface, and operational
  constraints.
- `wiki/` — developer-facing, example-rich guides (this page lives here).
- In-source rustdoc (`//!` module headers and `///` item docs) — precise and
  technical; module headers should state the module's role in the recursive
  architecture.

Keep **every Markdown file at 500 lines or fewer**. When a topic outgrows that,
split it into focused files and link them from the module's `README.md`.

> Doc-tests run. When adding illustrative code to rustdoc, prefer ` ```text `
> (or ` ```ignore ` / ` ```no_run `) unless you have confirmed a ` ```rust `
> block compiles, so the doc-test build stays green.

## Testing and coverage expectations

- Integration tests go in `tests/` with descriptive names such as
  `serializes_chat_messages`.
- Module-local unit tests go in the module's `test.rs`.
- For async behavior, use the existing `tokio` dev-dependency rather than
  introducing another runtime. `futures` is available for driving streams and
  `dotenvy` for example env loading.
- **Maintain at least 80% test coverage for meaningful library behavior.** Add
  or update tests with every behavior change, and document any intentionally
  untested edge case in the PR description.
- Add focused tests whenever you change serialization, graph routing, tool
  invocation, sub-agent/sub-graph recursion, or public model request/response
  shapes.

Much of the suite leans on `MockModel` — a deterministic, network-free
`ChatModel` with `echo`, `constant`, `with_responses`, and `with_tool_call`
constructors — so graph and harness behavior can be tested without any provider.

## Live / OpenAI tests

Network-backed provider code is always compiled (see
[Providers](Providers.md)); the live tests below skip without credentials. To
exercise it:

1. Compile and run the provider unit tests:

   ```sh
   cargo test
   ```

2. Run the OpenAI-backed examples, which require both the feature and a real
   key. Set `OPENAI_API_KEY` in the environment or a `.env` file at the repo
   root (loaded via `dotenvy`):

   ```sh
   export OPENAI_API_KEY=sk-...
   cargo run --example openai_chat
   cargo run --example openai_tools
   cargo run --example openai_structured
   cargo run --example openai_graph_agent
   cargo run --example openai_self_blueprint
   ```

Keep tests that perform real network calls opt-in (feature-gated and/or
credential-guarded) so the default `cargo test` run stays offline and
deterministic for every contributor and for CI.

## Pull-request checklist

Before opening a PR (always against the upstream `tinyhumansai/tinyagents`
repository, not a fork):

- `cargo fmt --check`
- `cargo clippy --all-targets -- -D warnings`
- `cargo build --all-targets`
- `cargo test` (and `cargo test` if you touched provider code)
- add or update tests for behavior changes
- update `README.md`, `docs/`, wiki, and rustdoc when public APIs, architecture,
  or expected usage change
- keep the PR focused on one logical change

### Commits

Use concise, imperative commit subjects (for example, `Add graph route
validation tests` or `Document expressive language safety boundary`). Make small,
focused commits: each should cover one logical change, build independently, and
avoid mixing formatting, refactors, and behavior changes unless they are
inseparable.

## See also

- [Providers](Providers.md) — configuring model providers.
- `CONTRIBUTING.md` and `AGENTS.md` — the authoritative source for these
  conventions.
- `docs/spec/README.md` — the architecture reference.
