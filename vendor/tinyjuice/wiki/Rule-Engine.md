# Rule Engine

The rule engine compacts command output using JSON rules. It is used by the log
compressor when `CompressInput` carries command or argv metadata, and by the
generic fallback for command-shaped payloads.

## Entry Points

```rust
load_builtin_rules() -> Vec<CompiledRule>
load_rules(&LoadRuleOptions) -> Vec<CompiledRule>
reduce_execution_with_rules(input, rules, options) -> CompactResult
classify_execution(input, rules, forced_rule_id)
```

## Rule Layers

Rules load in three layers:

1. built-in rules embedded in the crate
2. user rules from `~/.config/tokenjuice/rules/`
3. project rules from `<cwd>/.tokenjuice/rules/`

When the same `id` appears in multiple layers, the higher-priority layer wins:
project over user over built-in. `generic/fallback` is always sorted last.

## Rule Shape

A rule has:

- `id`
- `family`
- optional description and priority
- match conditions
- line filters
- transforms
- summarize settings
- counters
- output-match messages
- failure behavior

Matching can inspect:

- tool name
- `argv[0]`
- required argv groups
- any-of argv groups
- command substrings
- any-of command substrings

Transforms can:

- strip ANSI
- trim empty edges
- dedupe adjacent lines
- pretty-print JSON

Filters can:

- remove skip-pattern lines
- keep only keep-pattern lines

Counters extract named facts by regex. Bad regex patterns are dropped with a
debug log instead of crashing the engine.

## Reduction Pipeline

```text
ToolExecutionInput
        |
        v
normalize command/argv
        |
        v
classify against compiled rules
        |
        v
strip/process output text
        |
        v
apply matchOutput, filters, transforms, counters
        |
        v
summarize or clamp
        |
        v
CompactResult
```

Small outputs under the tiny-output threshold pass through even when a rule
could compact them.

## CompactResult

```rust
CompactResult {
    inline_text,
    preview_text,
    facts,
    stats: ReductionStats {
        raw_chars,
        reduced_chars,
        ratio,
    },
    classification: ClassificationResult {
        family,
        confidence,
        matched_reducer,
    },
}
```

## Built-In Rule Coverage

The built-in set covers common command families:

- archive tools
- Cargo and JS build tools
- cloud CLIs
- database CLIs
- Docker and Kubernetes
- filesystem listing/search
- git status, diff, show, logs, remotes, branches, stash
- package installs
- linters and formatters
- media tools
- network tools
- system and observability commands
- test runners
- task runners
- transfers

The current source embeds many JSON files from `src/vendor/rules/`.

## Fixture Tests

Rule behavior is tested through JSON fixtures in `tests/fixtures/*.fixture.json`.
Add a fixture when changing a rule or reproducing a reducer bug. The fixture
runner compares expected output exactly after trimming trailing whitespace.

## Agent Notes

- Prefer a project rule in `.tokenjuice/rules/` when behavior is specific to a
  repository.
- Prefer a user rule in `~/.config/tokenjuice/rules/` for local operator
  preferences.
- Keep `generic/fallback` broad but low-priority.
- Do not use the rule engine for arbitrary domain payloads without command
  context; the generic compressor intentionally declines that case.
