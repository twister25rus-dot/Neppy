# OpenHuman Integration

TinyJuice is built for OpenHuman, but the core crate does not depend on
OpenHuman runtime internals. Hosts provide policy through adapter calls,
callbacks, and plain configuration structs.

## Tool Output Adapter

Primary function:

```rust
compact_tool_output_with_policy(
    tool_name: &str,
    arguments: Option<&serde_json::Value>,
    output: &str,
    exit_code: Option<i32>,
    profile: AgentTokenjuiceCompression,
) -> (String, CompactionStats)
```

The adapter:

1. resolves the agent profile
2. skips recovery tools
3. extracts command/argv from tool arguments
4. extracts extension and query hints
5. builds `CompressInput`
6. calls the router
7. returns final text plus stats

## Agent Profiles

| Profile | Behavior |
| --- | --- |
| `Off` | Returns original output unchanged |
| `Light` | Disables CCR and ML; lossy compressors decline |
| `Full` | Uses current global options |
| `Auto` | Treated as `Full` inside TinyJuice; hosts may resolve it first |

`Light` is the safer profile for coding agents that need exact command output.
`Full` is the stronger profile for exploratory, research, or high-volume tool
flows where recoverable partial views are acceptable.

## Runtime Configuration

Hosts call `install_config` at startup:

```rust
install_config(
    options,
    max_cache_entries,
    max_cache_bytes,
    ccr_ttl_secs,
    disk_tier_root,
);
```

This installs:

- router and compressor options
- CCR memory limits
- CCR TTL
- optional CCR disk tier

Use `configure(options)` only when CCR limits and disk-tier settings are handled
elsewhere.

## ML Callback

Plain text compression is host-provided:

```rust
tinyjuice::ml::configure_callback(Some(callback));
```

The callback receives the text and `CompressOptions`, then returns
`Result<Option<String>, String>`. Returning `None`, returning text that does not
shrink, or returning an error makes the ML compressor decline without failing the
agent loop.

## Savings Recorder

Hosts can install:

```rust
tinyjuice::savings::configure_recorder(Some(recorder));
```

The recorder receives:

- content kind
- compressor kind
- estimated original tokens
- estimated compacted tokens

TinyJuice uses a rough character-based token estimate because it runs before a
provider call. Model pricing and dashboard persistence remain host policy.

## OpenHuman Context Type

`OpenHumanCompressionContext` currently carries:

- `conversation_id`
- `model`

This type is intentionally small. Do not add runtime dependencies to the core
crate without a feature or adapter boundary.

## Recovery Tool Contract

Any host that shows CCR footers to an agent must provide a callable recovery
tool:

- canonical name: `tokenjuice_retrieve`
- legacy alias: `retrieve_tool_output`

The tool should return exact original content for a token. Its output must not
be re-compacted.

## Agent Notes

- Feed raw tool output into TinyJuice only after host credential scrubbing.
- Keep compaction stats, but do not persist raw output in analytics records.
- Treat `CompactionStats.rule_id` as a compressor/rule label, not a proof of
  semantic quality.
- If a coding workflow fails because a partial view hid relevant data, switch
  that agent or tool to `Light` or `Off` before changing compressors globally.
