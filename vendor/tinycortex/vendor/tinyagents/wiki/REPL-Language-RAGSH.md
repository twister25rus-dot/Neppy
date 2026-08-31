# REPL Language (`.ragsh`)

The REPL language is TinyAgents' **imperative orchestration surface** — the
RLM/CodeAct loop. Where [`.rag`](Expressive-Language-RAG) declares graph
topology, `.ragsh` is an interactive, session-oriented language for inspecting,
scripting, and **recursively orchestrating** harness and graph runs. It is
explicitly inspired by Recursive Language Models (Zhang, Kraska, Khattab, 2025;
[`alexzhang13/rlm`](https://github.com/alexzhang13/rlm)) and CodeAct-style
agents, where a model writes small programs, inspects their output, calls
sub-models / sub-agents / sub-graphs as functions, and iterates until it has a
final answer.

The core RLM idea this surface ports: **context and intermediate state live in a
persistent REPL namespace as runtime values**, while model calls, recursive
sub-calls, and tools are exposed as capability-bound functions inside that
namespace — instead of being stuffed into one context window. See
[Recursion and RLM](Recursion-and-RLM) for the lineage and how this mitigates
"context rot."

A non-negotiable rule runs through the whole design: **`.ragsh` never bypasses
the registry, policy, or run limits.** It is an orchestration surface, not a
privilege-escalation surface.

Source lives in [`src/repl/`](https://github.com/tinyhumansai/tinyagents/tree/main/src/repl);
the module spec is
[`docs/modules/repl-language/README.md`](https://github.com/tinyhumansai/tinyagents/blob/main/docs/modules/repl-language/README.md)
and the detailed design (recursion, CodeAct loop, Rhai embedding, events) is
[`docs/modules/repl-language/design.md`](https://github.com/tinyhumansai/tinyagents/blob/main/docs/modules/repl-language/design.md).

## Two surfaces

`src/repl/` ships **two** session types, deliberately compiling side by side:

1. **The line-oriented command session** — `repl::ReplSession` (in
   [`src/repl/types.rs`](https://github.com/tinyhumansai/tinyagents/blob/main/src/repl/types.rs)),
   driven by `parse_command(line)` into a [`ReplCommand`]. This is the original
   skeleton: side-effect-free verbs (`set`, `get`, `show`, `help`, `quit`)
   execute; the runtime verbs (`load`, `compile`, `run`, `call`) are
   policy-checked and returned as a `ReplOutcome::Planned` record rather than
   executed. It is always in the default build.

2. **The Rhai-backed scripting session** — `repl::session::ReplSession`,
   re-exported at the crate root as `tinyagents::ReplSession` when the `repl`
   cargo feature is on. This is the **implemented** RLM/CodeAct surface: a
   persistent Rhai namespace plus host-registered capability functions that lower
   to the real registries, harness, and `.rag` compiler. It is gated behind
   `repl = ["dep:rhai"]` so the default build stays free of the embedded engine.

Because both surfaces name a type `ReplSession`, the scripting session is *not*
re-exported under `repl::ReplSession`; reach it via `repl::session::ReplSession`
or the crate-root `tinyagents::ReplSession` (feature `repl`).

## Status (honest)

- The **Rhai scripting session** evaluates cells against a persistent namespace
  today, with **all capability built-ins wired to the live registries**:
  `model_query`, `tool_call`, `agent_query`, `graph_run`, their `*_batched`
  variants, the `graph_define`/`graph_validate`/`graph_compile`/`graph_diff`/
  `graph_register` authoring surface, and the `emit`/`answer`/`show_vars` session
  built-ins, plus `print`/`debug` capture. Policy limits (operations, bytes, call
  counts, recursion depth, concurrency) are enforced fail-closed.
- The async capability calls run through a **blocking bridge**
  (`futures::executor::block_on`) for v1 — the only blocking surface, confined to
  `session/builtins.rs`. The design's longer-term direction is command recording.
- Two pieces remain **designed, not yet wired**: the model-driven **CodeAct
  driver** (`crate::repl::codeact`, referenced but not yet a module) and the part
  of `graph_run` that **materializes a `CompiledGraph` and drives its
  super-steps** — today `graph_run` resolves the registered blueprint and hands
  back a reference (graph id, start node, node count). The **Python
  out-of-process sandbox** is future work (`R7`).
- The **line-oriented command session** is still a skeleton: `load`, `compile`,
  `run`, and `call` are parsed and policy-checked, then returned as
  `ReplOutcome::Planned`.

## The Rhai scripting session

An orchestrator (a human, or a model acting as one) drives a
`repl::session::ReplSession` one **cell** at a time. Each cell is a small Rhai
script evaluated with `eval_cell(script)`, returning a [`ReplResult`].

```text
use tinyagents::ReplSession;          // feature = "repl"

let mut session = ReplSession::new();
let r1 = session.eval_cell("let counter = 5; counter")?;   // value = Int(5)
let r2 = session.eval_cell("counter + 1")?;                // value = Int(6)
```

Top-level `let` bindings survive into the next cell — the same idea as RLM's
persistent locals: a model can stash an intermediate result in a variable on one
line and consume it on the next, instead of re-deriving it from a giant prompt.
Construct a default stateless session with `ReplSession::new()`; supply
registries, a custom policy, application state, or a run context with the `with_*`
builder methods (`with_capabilities`, `with_policy`, `with_state`). Each rebuilds
the sandboxed engine so the capability functions resolve against the new wiring.

### Persistent namespace and reserved names

The namespace is a [`ReplVariables`] wrapper around a persistent Rhai `Scope`.
After **every cell** the runtime restores a set of reserved names to their
session baseline, so a script may read or temporarily shadow them but cannot
permanently replace the session's data slots or capability functions.

- **Reserved variables** (`RESERVED_VARIABLES`): `context`, `state`, `messages`,
  `history`, `run`, `answer`. Seed the data slots with `set_context(...)` /
  `set_state_var(...)`; arbitrary non-reserved variables go through
  `ReplVariables::set` (which rejects reserved names).
- **Reserved capability functions** (`RESERVED_FUNCTIONS`): the 16 host built-ins
  below. Rhai resolves call expressions against the function namespace, which is
  independent of variables, so a `let` cannot replace them; the runtime also
  scrubs any same-named variable a script introduces.

`reserved_names()` iterates both lists.

## Capability built-ins

Every built-in registered on the engine is a **host capability**, not a
script-native side effect: each resolves a name through the session's
[`CapabilityRegistry`], enforces the [`ReplPolicy`] call/recursion limits,
records a [`ReplCallRecord`], and lowers to the real harness/graph runtime.

| Built-in | Lowers to | Notes |
|----------|-----------|-------|
| `model_query(#{model, system?, prompt?, structured?})` | `registry.model(name).invoke` | one provider-neutral model call; returns text, or a `#{content, finish_reason}` map when `structured: true` |
| `model_query_batched([...])` | bounded-concurrency model calls | order preserved; concurrency = `max_concurrency` |
| `tool_call(#{tool, arguments?, structured?})` | `registry.tool(name).call` | returns content string, or `#{content, raw}` when `structured` and a raw value exists |
| `tool_call_batched([...])` | bounded-concurrency tool calls | order preserved |
| `agent_query(#{agent, prompt?/input?})` | `registry.agent(name).run` | a sub-task needing model–tool iteration; depth-checked |
| `agent_query_batched([...])` | bounded-concurrency agent runs | depth-checked per item |
| `graph_run(#{graph})` | `registry.graph_blueprint(name)` | resolves the registered blueprint, returns `#{graph, start, nodes, resolved}`; super-step execution is a later slice |
| `graph_run_batched([...])` | per-item `graph_run` | order preserved |
| `graph_define(#{name, source})` | `.rag` parser + `compile_with_provenance` | drafts a generated blueprint, returns a descriptor `#{name, nodes, compiled, requires_review}` |
| `graph_validate(descriptor)` | `Resolver::resolve_program` | returns an array of diagnostic messages |
| `graph_compile(descriptor)` | `Resolver::resolve_blueprint` | binds the draft through the resolver gate, marks it `compiled` |
| `graph_diff(name_or_draft, draft)` | `blueprint_diff` | diffs a registered graph or draft against a draft |
| `graph_register(#{graph, review_id?})` | review gate + registry intent | requires `compiled`; honors the review gate; returns the graph name |
| `emit(name)` / `emit(name, #{...})` | event sink | records a custom `ReplCallKind::Emit` |
| `answer(content)` | session control | sets the cell's `final_answer` |
| `show_vars()` | stdout | prints the pre-cell namespace snapshot |

`print(...)` and `debug(...)` are captured into the cell's `stdout` buffer.

## Policy limits

A session is bounded by [`ReplPolicy`], enforced **fail-closed** — a cell that
would exceed a bound returns an error rather than truncating or running unbounded
work. Defaults (from `Default for ReplPolicy`):

| Field | Default | Enforced where |
|-------|---------|----------------|
| `max_operations` | `1_000_000` | `Engine::set_max_operations`; runaway → `LimitExceeded` |
| `max_iterations` | `16` | CodeAct loop iterations (designed) |
| `max_script_bytes` | `64 KiB` | per-cell source size; also bounds `graph_define` source |
| `max_output_bytes` | `256 KiB` | per-cell stdout + value size |
| `max_model_calls` | `64` | `model_query` (and per-item batched); also bounds `agent_query` |
| `max_tool_calls` | `128` | `tool_call` (and per-item batched) |
| `max_graph_calls` | `32` | `graph_run` (and per-item batched) |
| `max_graph_definitions` | `8` | `graph_define` drafts |
| `max_depth` | `8` | sub-agent / sub-graph recursion; child past it → `SubAgentDepth` |
| `timeout` | `Some(30s)` | per-cell wall-clock |
| `max_concurrency` | `4` | batched call concurrency |
| `generated_graphs_require_review` | `true` | review token gate on `graph_register` |

Call counters are session-cumulative (shared across cells). Recursion depth is
checked against the harness recursion bookkeeping: a sub-run executes one level
below the session's run depth, and exceeding `max_depth` fails closed.

## Cell results

`eval_cell` returns a [`ReplResult`]:

- `stdout: String` — captured `print`/`debug` output.
- `value: Option<ReplValue>` — the cell's final expression value.
- `variables_changed: Vec<String>` — persistent (non-reserved) names the cell
  added or changed.
- `calls: Vec<ReplCallRecord>` — capability calls and emitted events, each with a
  `call_id`, `kind` ([`ReplCallKind`]: `Model`, `Tool`, `Graph`, `Agent`,
  `Emit`), `name`, structured `detail`, and `elapsed`.
- `final_answer: Option<String>` — set when the cell called `answer(...)`.
- `elapsed: Duration`.

[`ReplValue`] is the typed projection across the host/script boundary
(`Unit`, `Bool`, `Int`, `Float`, `String`, `Array`, `Map`), with `to_json()` and
`byte_len()` helpers. Opaque Rhai values are stringified rather than leaking a
host type across the boundary.

## Capabilities wiring

A session binds to named capabilities through [`ReplCapabilities`]. The design
document sketches separate model/tool/graph/agent registries; this crate unifies
all four under the single name-addressable [`CapabilityRegistry`], so
`ReplCapabilities` wraps that registry (shared via `Arc`) plus a long-term
[`StoreRegistry`] and an optional [`LanguageCompiler`] handle. Per-kind accessors
`models()`, `tools()`, `graphs()`, and `agents()` preserve the documented surface.

## Graph authoring never installs topology directly

The `graph_*` authoring surface lets a session draft and register its *own*
graph without acquiring arbitrary topology-mutation power. A generated graph
flows `graph_define` → `graph_validate` → `graph_compile` → (review) →
`graph_register`, exactly as a human-authored `.rag` blueprint does:

- `graph_define` lowers `.rag` source through the [`.rag`](Expressive-Language-RAG)
  parser and `compile_with_provenance`, stamping an `Origin::Generated`
  provenance label (the session id) onto the draft. Drafts persist across cells
  in the session, keyed by name, and are bounded by `max_graph_definitions` and
  `max_script_bytes`.
- `graph_compile` binds the draft through the **same capability resolver gate**
  file-backed `.rag` source passes — generated topology is never trusted blindly.
  A draft becomes `compiled` only after that bind.
- `graph_register` refuses an uncompiled draft, and when
  `generated_graphs_require_review` is set it refuses to register without a
  `review_id`. The compiled topology is handed to the host for installation
  through the registry resolver — the REPL never installs it directly.

The draft itself ([`GraphBlueprintHandle`]) lives host-side; scripts see only an
opaque descriptor map (`name`, `nodes`, `compiled`, `requires_review`).

## It never bypasses the registry, policy, or limits

This is the design's spine. A `.ragsh` session — even a fully model-driven one —
can only:

- call **registered** models, tools, agents, and graphs (capability functions
  resolve names through the `CapabilityRegistry`; unregistered names error with
  `ModelNotFound`, `ToolNotFound`, or `Capability`),
- within **bounded** operation counts, output size, call counts, recursion
  depth, concurrency, and the review gate above.

It has **no** direct filesystem, network, environment-variable, or process
access — the only host surface is the registered capability functions. The
sandboxed Rhai engine is configured with `set_max_operations` and granted no I/O.

## The CodeAct loop *(designed)*

A model-driven REPL agent follows this lifecycle (from `design.md`):

1. Create a `ReplSession` and load the `context`, `state`, `messages`,
   `history`, and `run` reserved variables.
2. Build a model request describing the available REPL functions, then invoke
   the model through the [harness](Harness) (using `app_state()` so the driver
   model and in-cell capabilities share state).
3. Extract fenced `ragsh` blocks from the assistant message.
4. Execute each block with `eval_cell`; capture stdout, changed variables, call
   records, events, and errors from the [`ReplResult`].
5. Append a compact execution result as the next user message.
6. Repeat until `answer(...)` is called or `max_iterations` is reached; then
   persist events, usage, cost, and the final answer.

When this loop runs **inside a graph node** (`kind repl_agent`), the graph still
owns node routing, checkpointing, interrupts, recursion depth, and failure
policy — so you get graph → REPL → (sub-model / sub-agent / sub-graph) recursion
with one consistent observability and policy story. The driver module
(`crate::repl::codeact`) is referenced by the session API but not yet wired.

## The line-oriented command session

The original skeleton models the loop as **data**: `parse_command(line)` returns
a [`ReplCommand`], `repl::ReplSession::execute(cmd)` returns a [`ReplOutcome`].

```text
line   = verb ( ws+ arg )* ws*
verb   = [a-zA-Z][a-zA-Z0-9_-]*
arg    = quoted | bare
quoted = '"' ( <any> | '\\' <any> )* '"'      // \\  \"  \n  \t escapes
bare   = ( <non-whitespace> )+
```

The first token is the verb, matched **case-insensitively**. For `call`, the
**remainder of the line** after the capability name is parsed as a single JSON
value. `parse_command` returns `TinyAgentsError::Parse` for empty input, an
unknown verb, a missing argument, an unterminated quoted string, or invalid JSON.

| Verb | Signature | Status today |
|------|-----------|--------------|
| `help` (`?`) | `help` | executes (prints verb list) |
| `quit` (`exit`, `q`) | `quit` | executes → `ReplOutcome::Quit` |
| `set` | `set <key> <value>` | executes (stores a string value) |
| `get` | `get <key>` | executes → `Value` (or `null`) |
| `show` | `show vars\|graphs\|status` | executes |
| `load` | `load <path>` | policy-checked `"load"` → `Planned` |
| `compile` | `compile <name>` | policy-checked `"compile"` → `Planned` |
| `run` | `run <graph> <input>` | policy-checked `"run"` → `Planned` |
| `call` | `call <capability> <json>` | policy-checked (named capability) → `Planned` |

Here the gate is a [`CapabilityPolicy`] — a deny-by-default allowlist of names. A
fresh session allows nothing; grant access with `CapabilityPolicy::allow("run")`
or `CapabilityPolicy::from_list(["run", "my_tool"])`. A gated command whose
capability is not on the allowlist returns `TinyAgentsError::Capability` *before*
it would touch the runtime.

`repl::ReplSession` holds a JSON-value variable map, the `CapabilityPolicy`, and
a `history: Vec<ReplCommand>` (every command is appended before it executes, so a
session is replayable). `ReplOutcome` variants: `Message(String)`,
`Value(serde_json::Value)`, `Planned { action, detail }`, and `Quit`.

```text
use tinyagents::repl::{ReplSession, CapabilityPolicy};

let policy = CapabilityPolicy::from_list(["my_tool"]);
let mut session = ReplSession::new().with_policy(policy);
session.set("x", serde_json::json!(42));
assert_eq!(session.get("x"), Some(&serde_json::json!(42)));
```

## Backend direction

The implemented in-process backend is **Rhai**: a Rust-native, sandboxed
embedded scripting language whose host API lets TinyAgents register exactly the
capability functions a script may use, with `Engine::set_max_operations` to fail
closed on runaway scripts. **Python** is documented as a future *out-of-process*
compatibility sandbox (`R7`) for RLM-style workflows where the sandbox boundary
must be explicit. Neither backend changes the rule that every capability is
registered, typed, and policy-checked at the Rust boundary.

## See also

- [Expressive Language (.rag)](Expressive-Language-RAG) — the declarative
  blueprint format `.ragsh` drafts, compiles, and registers.
- [Graph Runtime](Graph-Runtime) — the durable runtime `run` / `graph_run` drive.
- [Harness](Harness) — model calls, sub-agents, and the CodeAct host loop.
- [Registry](Registry) — the capability catalog the policy gates resolve against.
- [Recursion and RLM](Recursion-and-RLM) — the RLM execution model and lineage.
