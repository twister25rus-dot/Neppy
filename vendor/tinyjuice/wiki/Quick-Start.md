# Quick Start

The shortest path from clone to working TinyJuice APIs.

## Install

Add the crate:

```sh
cargo add tinyjuice
```

Or pin it in `Cargo.toml`:

```toml
[dependencies]
tinyjuice = "0.2"
```

The default feature set enables the `tokenjuice-treesitter` feature, which
includes optional tree-sitter dependencies for richer source-code handling.

## Clone, Test, And Run

```sh
git clone https://github.com/tinyhumansai/tinyjuice.git
cd tinyjuice
cargo test
```

CI-equivalent local gates:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Run the small public trait example:

```sh
cargo run --example passthrough
```

Run hot-path benchmarks:

```sh
cargo bench
```

## Use The Simple Trait Scaffold

Use this when you need a stable compression strategy boundary and report shape:

```rust
use tinyjuice::{CompressionConfig, CompressionInput, Compressor, PassthroughCompressor};

fn main() -> Result<(), tinyjuice::TinyJuiceError> {
    let compressor = PassthroughCompressor;
    let input = CompressionInput::new("Keep this text unchanged for now.");
    let output = compressor.compress(input, &CompressionConfig::default())?;

    assert_eq!(output.text, "Keep this text unchanged for now.");
    assert_eq!(output.report.strategy, "passthrough");
    Ok(())
}
```

## Use The Content Router

Use this for arbitrary blobs: file contents, web extracts, HTML, command output,
or anything large headed toward model context.

```rust
use tinyjuice::{CompressOptions, ContentHint, compress_content};

async fn compact_json(payload: &str) {
    let hint = ContentHint {
        extension: Some("json".to_string()),
        source_tool: Some("read_file".to_string()),
        ..Default::default()
    };

    let result = compress_content(payload, Some(hint), &CompressOptions::default()).await;
    if result.applied {
        println!(
            "{} compressed by {}: {} -> {} bytes",
            result.content_kind.as_str(),
            result.compressor.as_str(),
            result.original_bytes,
            result.compacted_bytes,
        );
    }
}
```

## Use The Tool Adapter

This is the API an agent runtime usually wants:

```rust
use tinyjuice::{AgentTokenjuiceCompression, compact_tool_output_with_policy};

async fn compact_shell_output(output: &str) {
    let args = serde_json::json!({ "command": "cargo test" });
    let (text, stats) = compact_tool_output_with_policy(
        "shell",
        Some(&args),
        output,
        Some(101),
        AgentTokenjuiceCompression::Full,
    ).await;

    println!(
        "applied={} rule={} ratio={:.3}",
        stats.applied,
        stats.rule_id,
        stats.ratio(),
    );

    // Show `text` to the model.
}
```

## Configure Runtime Options

Hosts install process-global options at startup:

```rust
use tinyjuice::{CompressOptions, install_config};

let mut opts = CompressOptions::default();
opts.min_bytes_to_compress = 2048;
opts.ccr_min_tokens = 500;
opts.ml_text_enabled = false;

install_config(
    opts,
    256,          // max CCR entries
    64 * 1024 * 1024, // max CCR bytes
    None,         // no TTL
    None,         // no disk tier
);
```

## Recover An Original

When a compressed output includes a footer with `tokenjuice_retrieve`, the
original can be recovered from CCR:

```rust
use tinyjuice::cache;

let tokens = cache::parse_markers(&model_visible_text);
if let Some(token) = tokens.first() {
    if let Some(original) = cache::retrieve(token) {
        println!("recovered {} bytes", original.len());
    }
}
```

## Crate Layout

```text
src/compress.rs          router entry point
src/compressors/         per-kind compressors
src/detect/              content detection and hints
src/cache/               CCR marker and store
src/rules/               rule loading and compilation
src/reduce.rs            command-output reduction engine
src/tool_integration.rs  OpenHuman-style adapter
src/compressor/          simple trait scaffold
src/config/              simple config scaffold
tests/                   fixture and e2e tests
benches/                 Criterion hot-path benchmarks
interface/               local analytics UI
wiki/                    GitHub wiki source
```

## Next Steps

- Read [Capabilities](Capabilities) for the full API index.
- Read [Router and Compressors](Router-and-Compressors) before changing
  compression behavior.
- Read [CCR Recovery](CCR-Recovery) before changing marker, cache, or retrieval
  semantics.
- Read [OpenHuman Integration](OpenHuman-Integration) before wiring TinyJuice
  into a host runtime.
