# Agent Guide

This page is a compact, machine-oriented reference for agents modifying or using
TinyJuice.

## First Choice API

Use this for tool output:

```rust
compact_tool_output_with_policy(tool_name, arguments, output, exit_code, profile).await
```

Use this for arbitrary content:

```rust
compress_content(content, Some(hint), &opts).await
```

Use this for command-output reducer tests:

```rust
reduce_execution_with_rules(input, &load_builtin_rules(), &ReduceOptions::default())
```

## Do Not

- Do not log raw prompts, raw context, raw tool output, or credentials.
- Do not compact output from `tokenjuice_retrieve` or `retrieve_tool_output`.
- Do not claim compression percentages without benchmark fixtures.
- Do not add OpenHuman runtime dependencies to core modules.
- Do not make a lossy compressor return `reformatted`.
- Do not append CCR footers inside a compressor.
- Do not blindly head/tail truncate non-command domain data.

## Add A Compressor

1. Add a module under `src/compressors/`.
2. Implement `compressors::Compressor`.
3. Return `None` when unsupported or not smaller.
4. Return `CompressOutput::lossy` when data is dropped.
5. Return `CompressOutput::reformatted` only when every value is preserved.
6. Register the static compressor in `src/compressors/mod.rs`.
7. Add or update content detection in `src/detect/` if needed.
8. Add focused unit tests and an e2e test when CCR behavior changes.
9. Update this wiki and README if public behavior changes.

## Add A Rule

1. Create a JSON rule in the desired layer:
   - built-in: `src/vendor/rules/`
   - user: `~/.config/tokenjuice/rules/`
   - project: `.tokenjuice/rules/`
2. Include match criteria with enough specificity.
3. Use filters, transforms, counters, and summarize settings.
4. Add a fixture in `tests/fixtures/*.fixture.json`.
5. Run `cargo test`.

## Debug A Missed Compaction

Check in order:

1. Was `router_enabled` true?
2. Was the payload below `min_bytes_to_compress`?
3. Did detection choose the expected `ContentKind`?
4. Was the per-kind compressor enabled?
5. Did the compressor return `None`?
6. Was the compacted output actually smaller?
7. Was it lossy with CCR disabled?
8. Was it lossy below `ccr_min_tokens`?
9. Could CCR retain the original?
10. Was the agent profile `Light` or `Off`?

## Debug Bad Recovery

Check:

- Does output contain a parseable marker?
- Is the token a fixed-shape hex token?
- Is the memory store still within TTL?
- Was a disk tier configured?
- Was the original too large for memory and disk disabled?
- Was the output from a recovery tool accidentally re-compacted?

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo bench
cargo run --example passthrough
```

## File Map

```text
src/lib.rs                  public exports
src/types.rs                router, rule, and profile types
src/compress.rs             router
src/compressors/            compressors
src/cache/                  CCR
src/rules/                  rule loader/compiler/builtins
src/reduce.rs               reduction pipeline
src/tool_integration.rs     host tool adapter
src/ml.rs                   host ML callback
src/savings.rs              host savings callback
tests/e2e_tool_output.rs    profile + CCR end-to-end checks
tests/fixtures.rs           rule fixtures
benches/compression.rs      hot-path throughput benches
```

## Preferred Wording

Use:

- "content-aware compaction"
- "recoverable partial view"
- "exact original via `tokenjuice_retrieve`"
- "estimated token savings"
- "benchmark fixtures required before percentage claims"

Avoid:

- "guaranteed 90% savings"
- "semantic compression is always safe"
- "lossless" when rows, lines, markup, context, or bodies are omitted
