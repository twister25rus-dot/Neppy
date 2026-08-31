# TinyJuice Wiki

**TinyJuice is a Rust token-compression engine for agent context:** a small,
pluggable boundary that turns noisy tool output into compact, inspectable, and
recoverable model-facing text.

The repository `README.md` is the marketing-level overview. This wiki is the
technical companion for humans and agents: exact modules, entry points, profiles,
compressor behavior, recovery semantics, command rules, and development gates.

- crate: `tinyjuice` (Rust 2024 edition, GPL-3.0-only)
- crates.io: <https://crates.io/crates/tinyjuice>
- docs.rs: <https://docs.rs/tinyjuice>
- source: <https://github.com/tinyhumansai/tinyjuice>

> **New here?** Start with [Capabilities](Capabilities). It is the master index
> of implemented surfaces and the shortest path for an agent to choose the
> correct API.

## What TinyJuice Owns

TinyJuice sits between host tool output and model context:

1. detect what kind of content a blob contains
2. choose a specialized compressor for that content kind
3. decline when compression would be unsafe, too small, disabled, or not smaller
4. offload exact originals to CCR when a lossy view is returned
5. report the content kind, compressor kind, byte counts, and recovery token

It does not own model execution, OpenHuman runtime state, provider pricing, or
host analytics persistence. Hosts install those policies through small callback
and adapter seams.

## The Technical Surfaces

1. **[Router and Compressors](Router-and-Compressors)** (`src/compress.rs`,
   `src/compressors/`) - content detection and specialized reducers for JSON,
   code, logs, search results, diffs, HTML, and plain text.
2. **[CCR Recovery](CCR-Recovery)** (`src/cache/`) - bounded exact-original
   storage, retrieval markers, memory/disk tiers, TTL, and range retrieval.
3. **[Rule Engine](Rule-Engine)** (`src/rules/`, `src/reduce.rs`) - built-in,
   user, and project JSON rules for command output reduction.
4. **[SDK and Plugin Integration](SDK-and-Plugin-Integration)** (`src/sdk.rs`,
   `src/bin/tinyjuice.rs`) - Rust SDK usage, host-neutral request/response
   shape, and the `tinyjuice reduce-json` protocol.
5. **[OpenHuman Integration](OpenHuman-Integration)** (`src/tool_integration.rs`,
   `src/openhuman/`) - host-facing compaction adapter, profiles, configuration,
   ML callback, and savings recorder seams.
6. **Public strategy scaffold** (`src/compressor/`, `src/config/`) - the small
   `Compressor` trait, `CompressionInput`, `CompressionOutput`, and
   `CompressionConfig` surface.
7. **[Analytics Interface](Analytics-Interface)** (`interface/`) - local-first
   UI for metadata-only compression run records.
8. **[Development](Development)** (`tests/`, `benches/`, `docs/references/`) -
   fixture tests, e2e recovery checks, hot-path benchmarks, and design specs.

## Agent Notes

- Prefer `compact_tool_output_with_policy` for tool-result compaction.
- Prefer `compress_content` for arbitrary blobs with a `ContentHint`.
- Use `AgentTokenjuiceCompression::Light` when exact coding output matters more
  than lossy savings.
- Never re-compact `tokenjuice_retrieve` output.
- Do not log raw prompt, context, tool output, or credentials.
- Do not claim percentage savings unless benchmark fixtures prove them.

## Page Index

Getting started

- [Home](Home) - this page.
- [Capabilities](Capabilities) - master functionality index.
- [Quick Start](Quick-Start) - install, run, and call the main APIs.
- [Examples](Examples) - usage patterns and runnable local examples.

Concepts

- [Architecture](Architecture) - how the layers compose.
- [Router and Compressors](Router-and-Compressors) - detection, routing, and
  per-kind behavior.
- [CCR Recovery](CCR-Recovery) - exact-original recovery and marker semantics.

Modules

- [Rule Engine](Rule-Engine) - JSON rule schema, overlay layers, classification,
  and reduction.
- [SDK and Plugin Integration](SDK-and-Plugin-Integration) - Rust SDK and
  `reduce-json` protocol details.
- [OpenHuman Integration](OpenHuman-Integration) - profile and host adapter
  contracts.
- [Analytics Interface](Analytics-Interface) - local dashboard data contract.
- [Development](Development) - checks, tests, fixtures, benchmarks, and docs.

Agent docs

- [Agent Guide](Agent-Guide) - compact machine-oriented reference.
- [Security and Privacy](Security-and-Privacy) - sensitive data handling,
  logging, CCR, and reporting rules.

## Repository Links

- [Main repository](https://github.com/tinyhumansai/tinyjuice)
- [Reference specs](https://github.com/tinyhumansai/tinyjuice/tree/main/docs/references)
- [Example](https://github.com/tinyhumansai/tinyjuice/blob/main/examples/passthrough.rs)
- [Contributing guide](https://github.com/tinyhumansai/tinyjuice/blob/main/CONTRIBUTING.md)
