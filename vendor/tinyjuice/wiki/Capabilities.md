# Capabilities

This is the master index of TinyJuice functionality. Use it to pick the right
entry point before reading a deep page.

## Primary APIs

| Need | API | Module | Notes |
| --- | --- | --- | --- |
| Compact arbitrary content | `compress_content(content, hint, opts)` | `src/compress.rs` | Detects kind, routes, applies CCR when lossy |
| Route already-shaped content | `route(input, opts)` | `src/compress.rs` | Lower-level router for adapters |
| Compact tool output | `compact_tool_output_with_policy(...)` | `src/tool_integration.rs` | Best default for OpenHuman-style tool loops |
| Minimal tool compaction | `compact_output(...)` | `src/tool_integration.rs` | Content + tool name + enabled flag |
| Install runtime options | `install_config(...)` | `src/tool_integration.rs` | Options + CCR limits + disk tier |
| Snapshot options | `current_options()` | `src/tool_integration.rs` | Reads process-global options |
| Run command reducer | `reduce_execution_with_rules(...)` | `src/reduce.rs` | Rule-engine path for command output |
| Load rules | `load_rules(...)`, `load_builtin_rules()` | `src/rules/` | Built-in/user/project overlay |
| Recover original | `cache::retrieve(token)` | `src/cache/` | Full original by CCR token |
| Recover range | `cache::retrieve_range(...)` | `src/cache/` | Byte or line slices |
| Parse markers | `cache::parse_markers(text)` | `src/cache/marker.rs` | Finds `tj` and legacy tokens |
| Install ML callback | `ml::configure_callback(...)` | `src/ml.rs` | Host-provided plain-text compressor |
| Install savings recorder | `savings::configure_recorder(...)` | `src/savings.rs` | Host-owned attribution |
| Simple strategy scaffold | `Compressor` trait | `src/compressor/` | Small public trait boundary |

## Content Kinds

| Kind | Detector signals | Compressor | Behavior |
| --- | --- | --- | --- |
| `Json` | MIME/extension or JSON object/array structure | `SmartCrusher` | Array-of-objects to table, anomaly rows retained when row-dropping |
| `Code` | Source extensions, MIME, or structural code heuristics | `Code` | Keeps imports/signatures/shallow structure, collapses deep bodies |
| `Log` | Tool prior or log-line ratio | `Log` | Keeps errors, warnings, summaries, stack traces; command output uses rule engine |
| `Search` | Search tool prior or `path:line:body` ratio | `Search` | Groups by file, ranks matches, keeps top K per file |
| `Diff` | Diff extension, hunk headers, `diff --git` | `Diff` | Keeps patch structure and changed lines, collapses long context |
| `Html` | HTML MIME/extension/tags | `Html` | Extracts readable text |
| `PlainText` | Fallback | `MlText` or pass-through | ML callback slot when enabled; otherwise generic command fallback only |

## Agent Profiles

| Profile | Effect |
| --- | --- |
| `auto` | Resolved by host agent definition; TinyJuice treats as `full` when called directly |
| `full` | Uses configured router and CCR behavior unchanged |
| `light` | Disables CCR-backed lossy compression and ML plain-text compression |
| `off` | Exact pass-through |

## Safety Capabilities

- Pass-through when the router is disabled.
- Pass-through below `min_bytes_to_compress`.
- Pass-through when a compressor declines.
- Pass-through when output is not smaller.
- Pass-through for lossy output when CCR is disabled or below `ccr_min_tokens`.
- Pass-through for lossy output when the original cannot be retained.
- Recovery tools are never re-compacted.
- CCR tokens are fixed-shape hex tokens and reject path traversal.

## Built-In Rule Families

TinyJuice embeds command-output rules for common families:

- archive: `tar`, `zip`, `unzip`
- build: `cargo build`, `cargo doc`, `vite`, `webpack`, `tsc`, `esbuild`,
  `tsdown`
- cloud: `aws`, `az`, `flyctl`, `gcloud`, `gh`, `vercel`
- database: `mongosh`, `mysql`, `psql`, `redis-cli`, `sqlite3`
- devops: `docker`, `docker compose`, `kubectl`
- filesystem: `find`, `ls`
- git: branch, diff stats, logs, show, stash, status, remotes
- install: `bun`, `npm`, `pnpm`, `yarn`
- lint: `biome`, `cargo fmt`, `cargo clippy`, `eslint`, `oxlint`,
  `prettier`
- media: `ffmpeg`, `mediainfo`
- network: `curl`, `dig`, `nslookup`, `ping`, `ssh`, `traceroute`, `wget`
- observability/system/service/package/test/task/transfer rules
- generic fallback

## Agent Notes

When integrating with a tool loop:

1. Extract `tool_name`, raw JSON arguments, output text, and exit code.
2. Pass them to `compact_tool_output_with_policy`.
3. Show `text` to the model.
4. Preserve `stats` for telemetry.
5. Ensure `tokenjuice_retrieve` is always available if the model can see CCR
   footers.

When writing docs or product copy:

- Say "compacts" or "reduces model-facing context".
- Do not say "saves X%" until benchmark fixtures exist.
- Mention recoverability for partial views.
- Mention that raw content should not be logged.
