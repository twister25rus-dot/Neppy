# CCR Recovery

CCR means **Compress, Cache, Retrieve**. When TinyJuice returns a partial view,
the exact original is stored under a short content hash and the compacted text
gets a retrieval footer.

## Why CCR Exists

Lossy compaction is useful only if the agent can recover the omitted data when
needed. CCR makes partial views explicit and reversible:

```text
large original output
        |
        v
lossy compressor returns compact body
        |
        v
router stores original in CCR
        |
        v
router appends footer with tokenjuice_retrieve token
```

If the original cannot be retained, TinyJuice declines the lossy output and
returns the original unchanged.

## Marker Format

Canonical marker:

```text
tj:<hash>
```

In model-visible output it appears as:

```text
[compacted tool output - this is a PARTIAL view; the full original (...) is
available by calling tokenjuice_retrieve with token "..."]
```

The source marker helper formats the marker as `tj` inside bracket glyphs and
also parses legacy `retrieve_tool_output("...")` forms. Agent docs should refer
to the tool name and token, not depend on visual marker formatting.

## Recovery Tools

Current recovery tool names:

- `tokenjuice_retrieve`
- `retrieve_tool_output` (legacy alias)

Both are special:

- they must be advertised when an agent can see CCR footers
- their outputs must never be re-compacted
- they return exact original content, not another partial view

## Store Behavior

Module: `src/cache/store.rs`

The CCR store is process-global and bounded by:

- max entry count
- max total bytes
- optional TTL
- optional disk tier root

Tokens are short SHA-256-derived hex strings. Re-offloading identical content is
idempotent. The disk tier is best-effort and allows originals to survive memory
eviction when configured.

## Range Retrieval

`retrieve_range(hash, start, end, unit)` can return slices by:

- bytes
- lines

Byte retrieval clamps to UTF-8 character boundaries. Line retrieval clamps
out-of-bounds ends and returns an empty string when the start is beyond the
original.

## Security Rules

- Tokens must match the fixed generated hex shape.
- Invalid tokens return `None` before touching the disk tier.
- Disk joins never accept arbitrary path components.
- Raw original content should not be logged.
- Disk-tier roots should live under a host-controlled workspace directory.
- TTL and byte caps are host policy, not compressor policy.

## API Reference

```rust
cache::offload(content) -> String
cache::offload_checked(content) -> (String, bool)
cache::retrieve(hash) -> Option<String>
cache::retrieve_range(hash, start, end, RangeUnit::Lines)
cache::parse_markers(text) -> Vec<String>
cache::stats() -> (entries, bytes)
cache::configure(max_entries, max_bytes, ttl_secs)
cache::enable_disk_tier(root)
cache::disable_disk_tier()
cache::is_recovery_tool(tool_name)
```

## Agent Notes

When the model sees a partial-view footer:

1. Continue with the compact view if enough information is visible.
2. Call `tokenjuice_retrieve` with the token when exact omitted data matters.
3. Do not summarize the retrieved original and then throw away the token; keep
   the token available for later exact retrieval.
4. Never compact the recovery tool output.
