# Examples

TinyJuice currently ships one runnable example plus several source-level usage
patterns.

## Passthrough Example

Run:

```sh
cargo run --example passthrough
```

It exercises the small public `Compressor` scaffold:

```rust
use tinyjuice::{CompressionConfig, CompressionInput, Compressor, PassthroughCompressor};

fn main() -> Result<(), tinyjuice::TinyJuiceError> {
    let compressor = PassthroughCompressor;
    let input = CompressionInput::new("Keep this text unchanged for now.");
    let output = compressor.compress(input, &CompressionConfig::default())?;

    println!("{}", output.text);
    Ok(())
}
```

## Compact A Tool Result

```rust
use serde_json::json;
use tinyjuice::{AgentTokenjuiceCompression, compact_tool_output_with_policy};

async fn run(output: &str) {
    let (text, stats) = compact_tool_output_with_policy(
        "shell",
        Some(&json!({ "command": "git status" })),
        output,
        Some(0),
        AgentTokenjuiceCompression::Full,
    ).await;

    if stats.applied {
        eprintln!("{} -> {}", stats.original_bytes, stats.compacted_bytes);
    }

    // Send `text` to the model.
}
```

## Compact A JSON Payload

```rust
use tinyjuice::{CompressOptions, ContentHint, compress_content};

async fn compact(payload: &str) {
    let result = compress_content(
        payload,
        Some(ContentHint {
            extension: Some("json".to_string()),
            source_tool: Some("read_file".to_string()),
            ..Default::default()
        }),
        &CompressOptions::default(),
    ).await;

    assert!(result.compacted_bytes <= result.original_bytes);
}
```

## Parse And Retrieve CCR Tokens

```rust
use tinyjuice::cache;

fn recover(model_visible_text: &str) -> Option<String> {
    let token = cache::parse_markers(model_visible_text).into_iter().next()?;
    cache::retrieve(&token)
}
```

## Install Test-Like Runtime Config

```rust
use tinyjuice::{CompressOptions, install_config};

fn install() {
    install_config(
        CompressOptions::default(),
        256,
        64 * 1024 * 1024,
        None,
        None,
    );
}
```

## Run The Analytics UI

```sh
cd interface
npm install
npm run dev
```

The UI consumes metadata-only JSON records. The sample schema is documented in
`interface/README.md`.

## Add A Rule Fixture

Fixture tests live under `tests/fixtures/*.fixture.json`. Each fixture contains
a tool execution and expected reduced output. Run:

```sh
cargo test all_reduce_fixtures_produce_expected_output
```

Use fixtures for regression coverage when changing command rules.
