# Router And Compressors

The content router is the main TinyJuice engine. It detects content kind, picks
a compressor, applies pass-through rules, and adds CCR recovery when needed.

## Entry Points

```rust
compress_content(content: &str, hint: Option<ContentHint>, opts: &CompressOptions)
route(input: CompressInput<'_>, opts: &CompressOptions)
detect_only(content: &str, hint: &ContentHint)
```

Use `compress_content` for most callers. Use `route` when an adapter already has
command, argv, exit code, or original byte count.

## Detection Order

`detect_content_kind` resolves in this order:

1. `ContentHint.explicit`
2. MIME type
3. file extension
4. source tool prior
5. structural detection

Structural detection checks JSON, diff, HTML, search-result ratio, source-code
shape, log-line ratio, then plain text.

## Router Gates

The router returns exact pass-through when:

- `router_enabled` is false
- `original_bytes < min_bytes_to_compress`
- the content kind has no enabled compressor
- the selected compressor returns `None`
- fallback compressor returns `None`
- compacted output is not smaller
- output is lossy and CCR is disabled
- output is lossy and token estimate is below `ccr_min_tokens`
- output is lossy and the exact original cannot be retained

## CompressOptions

| Field | Default | Meaning |
| --- | --- | --- |
| `router_enabled` | `true` | Master switch |
| `ccr_enabled` | `true` | Allow original offload and retrieval footers |
| `search_enabled` | `true` | Allow search-result compaction |
| `code_enabled` | `true` | Allow code compaction |
| `html_enabled` | `true` | Allow HTML extraction |
| `ml_text_enabled` | `false` | Allow host-provided ML plain-text callback |
| `min_bytes_to_compress` | `2048` | Byte floor |
| `ccr_min_tokens` | `500` | Token-estimate floor for lossy CCR paths |
| `max_inline_chars` | `None` | Rule-engine clamp override |

## JSON SmartCrusher

Module: `src/compressors/json.rs`

Input: JSON arrays of objects with enough rows and columns.

Behavior:

- renders object arrays as tables
- writes each column name once
- keeps all rows for moderate arrays
- for large arrays, keeps head, tail, error rows, and numeric outliers
- declines if the result does not shrink

Lossiness:

- faithful table reformat when all rows are preserved
- lossy row-dropping when large arrays omit rows

## Code Compressor

Module: `src/compressors/code.rs`

Input: source files detected by extension, MIME type, or code-like structure.

Behavior:

- keeps imports, declarations, signatures, and shallow structure
- collapses deep bodies with placeholders
- keeps lines containing markers such as `TODO`, `FIXME`, `error`, `panic`, and
  `unsafe`
- can use tree-sitter when the feature and grammar path are available; falls
  back to the brace-depth heuristic

Lossiness: lossy. The exact file should be recoverable through CCR.

## Log Compressor

Module: `src/compressors/log.rs`

Two paths:

- command output with command/argv: run the rule engine
- non-command log blobs: preserve failure signals and drop noisy lines

Signal path keeps:

- summary lines
- first/last errors
- deduplicated warnings
- stack traces up to caps

It declines when no log signal is present, avoiding blind truncation of
legitimate data.

## Search Compressor

Module: `src/compressors/search.rs`

Input: grep/ripgrep-style `path:line:body` results.

Behavior:

- groups matches by file in first-seen order
- ranks matches by query-term density when `ContentHint.query` exists
- otherwise ranks by distinctiveness and importance signals
- keeps top K per file and writes a `+N more` tally

Lossiness: lossy, with full result set recoverable through CCR.

## Diff Compressor

Module: `src/compressors/diff.rs`

Input: unified diffs and patches.

Behavior:

- keeps file headers and hunk headers
- keeps added and removed lines
- collapses long context windows
- summarizes noisy lockfile/bundle hunks

Lossiness: lossy, with exact patch recoverable through CCR.

## HTML Compressor

Module: `src/compressors/html.rs`

Input: HTML documents.

Behavior:

- strips tags
- decodes readable text shape
- declines if extraction does not shrink

Lossiness: usually lossy because markup and attributes are removed.

## ML Plain Text Slot

Module: `src/compressors/ml_text.rs`, `src/ml.rs`

Plain text has no reliable structure. TinyJuice exposes a host-installed async
callback slot and only calls it when `ml_text_enabled` is true. If no callback
is configured, or if the callback errors or does not shrink, the compressor
declines and routing continues safely.

## Generic Fallback

Module: `src/compressors/generic.rs`

The generic fallback runs only for command output with command/argv metadata. It
uses the rule engine's generic head/tail behavior. It intentionally declines for
domain-tool payloads to avoid blind truncation.

## Agent Notes

- `ContentHint.source_tool = "shell"` is not forced to log; shell output is too
  diverse and goes through structural detection.
- `grep`, `rg`, and related tools force `Search`.
- `run_tests`, `run_linter`, `npm_exec`, `node_exec`, `install_tool`, and `lsp`
  are strong log priors.
- `read_diff` and `git_operations` are strong diff priors.
- A lossy compressor must not append retrieval text itself. Return
  `CompressOutput::lossy` and let the router handle CCR.
