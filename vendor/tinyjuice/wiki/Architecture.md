# Architecture

TinyJuice is a **token-compression boundary for Rust agent hosts**. It keeps the
core crate focused on content reduction and recovery while leaving model
execution, OpenHuman runtime state, pricing, and persistence to the host.

## Layered Model

```text
Application / agent runtime
        |
        | tool output, file content, web extract, command output
        v
+--------------------------+
| tool_integration.rs      |
| profiles, config, stats  |
+------------+-------------+
             |
             v
+--------------------------+
| compress.rs              |
| detect, route, CCR gate  |
+------------+-------------+
             |
             v
+--------------------------+
| compressors/             |
| json code log search     |
| diff html ml generic     |
+------------+-------------+
             |
             +------------------------------+
             |                              |
             v                              v
+--------------------------+     +--------------------------+
| cache/ CCR               |     | savings / analytics hook |
| original recovery        |     | host attribution          |
+--------------------------+     +--------------------------+
```

The command-output path additionally enters the rule engine:

```text
CompressInput with command/argv
        |
        v
LogCompressor
        |
        v
reduce_execution_with_rules
        |
        v
builtin/user/project rules -> classification -> filters -> transforms -> clamp
```

## Core Modules

| Module | Owns | Does not own |
| --- | --- | --- |
| `compress.rs` | Router, pass-through decisions, CCR offload, savings event | Host config schema |
| `compressors/` | Per-content-kind reduction algorithms | CCR storage |
| `detect/` | `ContentHint` interpretation and structural detection | Policy toggles |
| `cache/` | Original storage, markers, retrieval, range retrieval | Agent tool registration |
| `rules/` | JSON rule loading, overlay, compilation | Reduction execution |
| `reduce.rs` | Command-output normalization and reduction pipeline | Rule discovery |
| `tool_integration.rs` | Runtime options, agent profiles, tool-output adapter | OpenHuman runtime deps |
| `ml.rs` | Host-installed async callback slot | Python server startup |
| `savings.rs` | Host-installed attribution callback | Pricing model |
| `compressor/` | Small trait scaffold | Content-aware routing |

## Data Flow

1. A caller passes raw text plus a `ContentHint` or tool metadata.
2. The detector resolves `ContentKind` using explicit hint, MIME/extension,
   tool prior, then structural heuristics.
3. The router checks global gates: enabled, minimum bytes, per-kind toggles.
4. A specialized compressor returns either `None` or `CompressOutput`.
5. The router falls back to the generic command compressor when allowed.
6. The router declines if the output is not smaller.
7. If the output is lossy, the router requires CCR to retain the exact original.
8. The final `CompressedOutput` carries text, kind, compressor, byte counts,
   lossy flag, applied flag, and optional CCR token.

## Pass-Through First

TinyJuice is built so the safe default is exact pass-through:

- small payloads pass through
- disabled router passes through
- disabled per-kind compressor passes through
- unknown or unsupported payloads pass through
- no shrinkage passes through
- lossy output without retained original passes through
- recovery-tool output passes through

This rule matters more than maximizing token savings. A compressor that cannot
prove a smaller, policy-valid output should decline.

## Recovery Boundary

Compressors do not write to CCR. They only return a body, a compressor kind, and
a `lossy` bit. The router owns recovery because it has global policy and can
decide whether the exact original was retained. That keeps every lossy path
consistent.

## Host Boundary

TinyJuice does not depend on OpenHuman runtime internals. Hosts map their own
configuration and runtime state into:

- `CompressOptions`
- CCR limits and disk root
- `AgentTokenjuiceCompression`
- optional ML compression callback
- optional savings recorder
- optional analytics export

The `openhuman` module currently contains adapter data types, not a runtime
dependency.

## Agent Notes

- Prefer adding behavior behind a new compressor or rule rather than hardcoding
  special cases in `tool_integration.rs`.
- Keep raw content out of logs. Log kinds, byte counts, token estimates,
  compressor labels, and rule IDs.
- If you add a lossy compressor, make sure it returns `lossy=true` and leaves
  CCR to the router.
- If you add a faithful reformat, return `reformatted(...)`; it may run without
  CCR when no data is dropped.
- If source and docs disagree, trust source and update docs in the same change.
