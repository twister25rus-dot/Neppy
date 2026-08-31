# Security And Privacy

TinyJuice handles model-facing context. Treat every input as potentially
sensitive.

## Sensitive Inputs

Inputs may contain:

- prompts
- user messages
- tool output
- file contents
- source code
- credentials
- database rows
- logs with secrets
- private URLs
- conversation identifiers

TinyJuice should not log raw content. Prefer metadata: byte counts, token
estimates, content kind, compressor kind, rule ID, profile, and status.

## Lossy Output

Lossy compaction must be explicit. A lossy compressor should return
`CompressOutput::lossy`, and the router should only emit it when the exact
original is retained or policy says a lossy view is safe.

Current router policy requires CCR for lossy output.

## CCR Store

CCR stores exact originals. That means it is useful and sensitive.

Rules:

- configure memory caps
- configure TTL where appropriate
- place disk tier roots under host-controlled workspaces
- validate tokens before disk access
- never expose arbitrary filesystem reads through retrieval
- never write CCR tokens into public logs with associated raw content

## Recovery Tools

`tokenjuice_retrieve` returns exact original content. It should follow the host
runtime's normal tool authorization and audit behavior. TinyJuice itself only
provides marker parsing and store retrieval helpers.

## Analytics

Analytics records should include metadata only:

- IDs
- timestamps
- algorithm labels
- content kind
- status
- bytes and token estimates
- latency
- lossy/CCR flags
- source and profile

Do not include raw prompt, context, tool output, file contents, or credentials.

## Reports And Docs

Do not claim exact savings percentages, quality retention, or safety guarantees
without benchmark fixtures. The current benchmark harness measures throughput
for hot paths, not retained-fact quality.

## Agent Notes

Before changing privacy-sensitive code, inspect:

- `src/cache/store.rs`
- `src/cache/marker.rs`
- `src/tool_integration.rs`
- `src/compress.rs`
- `SECURITY.md`

When in doubt, pass through unchanged rather than producing a smaller but
irrecoverable or misleading view.
