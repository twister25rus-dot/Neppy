# SDK And Plugin Integration

This page collects the technical integration material that used to live in the
root README. The README should stay focused on quick setup and product-level
positioning; host authors should use this page when wiring TinyJuice into an
agent runtime.

## How It Works

```text
tool output / file / web payload
        |
        v
ContentHint + structural detection
        |
        v
JSON | Code | Log | Search | Diff | HTML | PlainText
        |
        v
specialized compressor or command-rule reducer
        |
        v
pass-through if unsafe, too small, disabled, or not smaller
        |
        v
CCR offload + retrieval footer when the view is lossy
```

The router is intentionally fail-soft. If it cannot shrink safely, it returns
the original bytes unchanged.

## Compression Surfaces

- **JSON SmartCrusher** - renders repeated object arrays as compact tables and
  keeps anomaly rows when large arrays are row-dropped.
- **Code compressor** - keeps imports, signatures, shallow structure, and
  important markers while collapsing deep bodies.
- **Log compressor** - preserves failures, warnings, summaries, stack traces,
  and command-rule outputs while dropping passing noise.
- **Search compressor** - groups grep/ripgrep output by file, ranks matches,
  and keeps top hits with per-file tallies.
- **Diff compressor** - keeps patch structure and changed lines, collapses long
  context and noisy lockfile/bundle hunks.
- **HTML compressor** - extracts readable text from rendered markup.
- **Plain-text ML slot** - optional host-provided callback for learned text
  compression; disabled by default.
- **Generic command fallback** - line-oriented head/tail reduction for command
  output when no specialized rule wins.

TinyJuice does not publish production-corpus compression percentage claims yet.
The checked-in benchmark corpus exercises retained facts, latency,
reversibility, and regression safety without claiming universal savings.

## Rust Dependency

Add TinyJuice to a Rust project:

```toml
[dependencies]
tinyjuice = "0.2"
```

## Simple Trait Scaffold

Use the small public trait scaffold when you want a simple strategy boundary:

```rust
use tinyjuice::{CompressionConfig, CompressionInput, Compressor, PassthroughCompressor};

fn main() -> Result<(), tinyjuice::TinyJuiceError> {
    let compressor = PassthroughCompressor;
    let output = compressor.compress(
        CompressionInput::new("Keep this text unchanged for now."),
        &CompressionConfig::default(),
    )?;

    assert_eq!(output.report.strategy, "passthrough");
    Ok(())
}
```

## Content Router

Use the content router for arbitrary tool-output, file, web, HTML, JSON, or log
payloads:

```rust
use tinyjuice::{CompressOptions, ContentHint, compress_content};

async fn compact_payload(big_payload: &str) {
    let hint = ContentHint {
        source_tool: Some("read_file".to_string()),
        extension: Some("json".to_string()),
        ..Default::default()
    };

    let result = compress_content(big_payload, Some(hint), &CompressOptions::default()).await;
    if result.applied {
        println!("{} -> {} bytes", result.original_bytes, result.compacted_bytes);
    }
}
```

## OpenHuman-Style Tool Output

OpenHuman-style tool output integration goes through:

```rust
use tinyjuice::{AgentTokenjuiceCompression, compact_tool_output_with_policy};

async fn compact_command_output(command_output: &str) {
    let (_text, _stats) = compact_tool_output_with_policy(
        "shell",
        Some(&serde_json::json!({ "command": "cargo test" })),
        command_output,
        Some(101),
        AgentTokenjuiceCompression::Full,
    ).await;
}
```

## SDK Paths

TinyJuice exposes two integration paths:

- Rust hosts use the crate SDK directly.
- Non-Rust plugins and harnesses call the `tinyjuice reduce-json` protocol.

The SDK accepts a host-neutral `ToolExecutionInput` with tool name, command,
argv, stdout/stderr or combined text, exit code, cwd, and metadata. The response
contains the inline text plus metadata about the applied content kind,
compressor, token estimate, byte counts, and CCR recovery token when one was
created.

Do not log the request body from adapters; tool output may contain prompts,
credentials, or private context.

## Rust SDK

```rust
use tinyjuice::{
    AgentTokenjuiceCompression, TinyJuiceHost, TinyJuiceSdk, ToolExecutionInput,
};

async fn compact_for_harness(tool_output: String, exit_code: i32) {
    let sdk = TinyJuiceSdk::new(TinyJuiceHost::RustHarness)
        .with_profile(AgentTokenjuiceCompression::Full);

    let response = sdk
        .compress_tool_output(ToolExecutionInput {
            tool_name: "shell".to_string(),
            command: Some("cargo test".to_string()),
            argv: Some(vec!["cargo".to_string(), "test".to_string()]),
            combined_text: Some(tool_output),
            exit_code: Some(exit_code),
            ..Default::default()
        })
        .await;

    println!("{}", response.inline_text);
}
```

Use `TinyJuiceHost::OpenHuman` for OpenHuman adapters and
`TinyJuiceHost::RustHarness` for standalone Rust harnesses. Hosts should map
their own config into `CompressOptions`, choose the per-agent profile, and expose
CCR retrieval before enabling lossy compaction in production.

## JSON Protocol

Build the binary locally:

```sh
cargo build --release --bin tinyjuice
```

Send a full SDK request:

```json
{
  "host": "generic-json",
  "profile": "full",
  "input": {
    "toolName": "shell",
    "command": "cargo test",
    "argv": ["cargo", "test"],
    "combinedText": "large tool output...",
    "exitCode": 0,
    "metadata": {
      "source": "custom-harness"
    }
  },
  "options": {
    "minBytesToCompress": 512,
    "maxInlineChars": 1200,
    "ccrEnabled": true
  }
}
```

Run it through the protocol:

```sh
tinyjuice reduce-json payload.json
cat payload.json | tinyjuice reduce-json --host generic-json -
```

A bare `ToolExecutionInput` object is also accepted when the host, profile, and
options can stay at defaults.

## Crate Layout

```text
src/
  compress.rs         Universal content router
  compressors/        JSON, code, log, search, diff, HTML, ML, generic paths
  detect/             Content-kind hints and structural detection
  cache/              CCR offload, retrieval markers, memory/disk store
  rules/              Built-in + user + project command reduction rules
  reduce.rs           Rule-engine reduction pipeline
  sdk.rs              Host-neutral SDK and reduce-json request/response types
  tool_integration.rs OpenHuman-style tool-output adapter
  compressor/         Small public Compressor trait scaffold
  config/             Small public CompressionConfig scaffold
  openhuman/          Runtime-neutral OpenHuman adapter types
  savings.rs          Host-installed savings attribution hook
interface/            Self-hostable analytics UI
wiki/                 Technical GitHub wiki source
docs/references/      Design references and candidate strategy specs
```

## Related Pages

- [Quick Start](Quick-Start)
- [Capabilities](Capabilities)
- [Router and Compressors](Router-and-Compressors)
- [CCR Recovery](CCR-Recovery)
- [OpenHuman Integration](OpenHuman-Integration)
- [Development](Development)
