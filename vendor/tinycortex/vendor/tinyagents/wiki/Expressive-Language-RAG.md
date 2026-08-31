# Expressive Language (`.rag`)

The expressive language is TinyAgents' **declarative blueprint format**. A `.rag`
file describes an agent graph — its start node, state channels, nodes, and
routing — as compact, side-effect-free source text. It compiles into the **same**
[`graph`](Graph-Runtime) and [`harness`](Harness) structures as hand-written
Rust: the runtime never knows whether a graph came from a Rust builder or from a
`.rag` source string.

Crucially, `.rag` source can only **reference capabilities by name** — models,
tools, subgraphs, routers, reducers — that Rust has already registered and
allowed. It can never define behaviour or embed host code. That property is what
makes `.rag` the **safe boundary for agent-authored plans**: a language model can
emit a blueprint, and the same compiler + registry gate that validates a
human-written file validates the model's output before anything runs. See
[Recursion and RLM](Recursion-and-RLM) for where self-authoring fits in the
recursive picture.

Source lives in [`src/language/`](https://github.com/tinyhumansai/tinyagents/tree/main/src/language);
the module spec is
[`docs/modules/expressive-language/README.md`](https://github.com/tinyhumansai/tinyagents/blob/main/docs/modules/expressive-language/README.md).

## The pipeline

A `.rag` file flows through four fixed phases. Each phase is a separate submodule
so callers can stop at the level of safety they need:

```text
source  -->  lexer  -->  tokens  -->  parser  -->  AST  -->  compiler  -->  Blueprint
        (lexer.rs)              (parser.rs)              (compiler.rs)
```

| Phase    | Module                       | Input        | Output            | Validates                                  |
|----------|------------------------------|--------------|-------------------|--------------------------------------------|
| Lex      | `language::lexer::tokenize`  | `&str`       | `Vec<SpannedToken>` | tokens, strings, numbers, `//` comments  |
| Parse    | `language::parser::parse`    | tokens       | `Program` (AST)   | structure: well-formed blocks, expected tokens |
| Compile  | `language::compiler::compile`| `Program`    | `Vec<Blueprint>`  | semantics: duplicate names, start/targets, routing |
| Bind     | `language::resolver::Resolver` / `language::compiler::CapabilityResolver` | `Program`/`Blueprint` + registry | diagnostics / `()` | capability references against the registry |

The compiler then offers a final, optional phase — `build_graph` — that
materialises a `Blueprint` into a runnable `CompiledGraph` using a Rust-supplied
`NodeFactory`. Behaviour is always Rust's job; the blueprint only describes
topology.

### 1. Lexer (`lexer.rs`)

`tokenize(source)` turns text into [`SpannedToken`]s, each carrying a 1-based
line/column [`Span`]. The token set is deliberately tiny:

- identifiers / keywords: `[A-Za-z_][A-Za-z0-9_]*` (keywords like `graph`,
  `node`, `start` are lexed as identifiers and recognised contextually)
- numbers: integer or decimal, optionally signed (`50`, `1.5`, `-3`)
- double-quoted strings with `\n`, `\t`, `\r`, `\\`, `\"` escapes
- punctuation: `{ } [ ] ,` and the arrow `->`
- `//` line comments (skipped)

Lexical errors (unterminated string, invalid escape, malformed number, stray
character) surface as `TinyAgentsError::Parse` with the offending line/column.

### 2. Parser (`parser.rs`)

`parse(&tokens)` (or the one-shot `parse_str(source)`) is a small hand-written
recursive-descent parser. It performs **structural** validation only —
expected-token checks and well-formed blocks — and produces a `Program` AST. It
does *not* check whether names exist or routes are valid; that is the compiler's
job. Errors are `TinyAgentsError::Parse` with the span of the offending token.

### 3. Compiler (`compiler.rs`)

`compile(&program)` lowers each `graph` declaration into one serializable
[`Blueprint`] and runs the semantic checks:

- duplicate node names within a graph are rejected,
- a graph must declare a `start` node, and it must be defined,
- every `next`, route, edge, `command` `goto`, `send`, and `join` target must be
  a defined node or the reserved `END`,
- duplicate route labels on a node are rejected,
- a node may use static routing (`next` / an incident edge) **or** command
  routing (`routes`), never both.

Failures are `TinyAgentsError::Compile`. Routing precedence when lowering a node
is: explicit `routes` > `next` > `command` `goto` > top-level edge > terminal. A
`next END`, a `goto END`, or an edge to `END` becomes [`Routing::Terminal`].

`compile_with_provenance(&program, origin)` runs the same validation and lowering
but attaches source [`BlueprintProvenance`] — see [Provenance](#provenance).

### 4. Capability binding (the registry gate)

A compiled `Blueprint` is inert until its references are checked against what
Rust has registered. This is the safety boundary, and there are three entry
points covering two underlying gates.

The **blueprint gate** (`language::compiler`) works on a compiled span-less
`Blueprint`:

- **Minimal / manual** — `bind_capabilities(&blueprint, &resolver)` checks only
  `model` and `tool` references against a `CapabilityResolver` allowlist. Build
  one with `CapabilityResolver::from_lists(models, tools)` or the chaining
  `allow_model` / `allow_tool` / `allow_subgraph` / `allow_router` /
  `allow_reducer` / `with_node_kinds` helpers.
- **Strict / registry-backed** — `bind_capabilities_with_registry(&blueprint,
  &registry)` builds a fully populated resolver from a live
  [`CapabilityRegistry`](Registry) (models, tools, subgraphs, routers, reducers —
  *including aliases* — plus the default node kinds) and runs
  `CapabilityResolver::bind_blueprint`. This additionally validates node `kind`s,
  subgraph/router references, and channel reducers.

The **source gate** (`language::resolver::Resolver`) is the richer,
span-aware path and the recommended one. Built with `Resolver::from_registry`
(or `Resolver::from_capabilities`), it resolves a parsed `Program` *before*
compilation and additionally validates `subagent` node `agent` references against
a registered-agent allowlist (`allow_agent`):

- `Resolver::resolve_program(&program)` returns a `Vec<Diagnostic>` — one spanned
  [`Diagnostic`](#diagnostics) per offending reference, collected so a caller can
  surface them all at once. Each carries a stable code (`E-rag-unknown-model`,
  `E-rag-unknown-tool`, `E-rag-unknown-subgraph`, `E-rag-unknown-router`,
  `E-rag-unknown-agent`, `E-rag-unknown-reducer`, `E-rag-invalid-node-kind`).
- `Resolver::check_program(&program, source)` folds the *first* diagnostic into a
  `TinyAgentsError` (with a caret-underline rendering when `source` is supplied).
- `Resolver::resolve_blueprint(&blueprint)` is the span-less counterpart for an
  already-compiled blueprint, returning the same error variants and messages as
  the compiler's blueprint gate, extended with the agent check.

The default recognised node kinds (`DEFAULT_NODE_KINDS`) are `agent`, `model`,
`tool_executor`, `subgraph`, `graph`, `subagent`, `repl_agent`, `router`,
`interrupt`, `join`, `human`. Reference conventions used by the strict paths:

- `subgraph` / `graph`: the node's `graph "name"` field (falling back to the
  legacy `model` field) names a registered subgraph blueprint,
- `router`: the `model` field names a registered router function,
- `subagent`: the `agent "name"` field names a registered agent,
- everything else: the `model` field names a registered chat model.

An unknown kind is a `Compile` error; the first unregistered model/tool/agent/
subgraph/router/reducer reference is a `Capability` error.

Two convenience façades run the whole chain in one call:

- `compile_source(source, &registry)` — `parse -> compile -> registry-bind`
  (blueprint gate).
- `resolve_source(source, &registry)` — `parse -> resolve (spanned) -> compile`,
  routing generated and file-backed source through the *same* `Resolver` gate.

### Materialising into the runtime

```text
Blueprint  --build_graph(&blueprint, &factory)-->  CompiledGraph<State, State>
```

`build_graph` walks each `NodeSpec`, asks the caller's `NodeFactory::make` for a
runnable handler, and wires routing into durable graph topology:

- `Routing::Next(target)` → `GraphBuilder::add_edge` (a static successor),
- `Routing::Conditional(_)` → `GraphBuilder::mark_command_routing` (the node
  decides its route at runtime by returning a `Command` `goto`),
- `Routing::Terminal` → `GraphBuilder::set_finish` (route to `END`).

The blueprint's `start` node becomes the graph entry. Because the factory is the
only source of node *behaviour*, declarative source can never smuggle in
arbitrary code.

## The `Blueprint` artifact

A [`Blueprint`] is the inspectable, fully serializable output of the compiler. It
can be stored, diffed, reviewed in a UI, and reloaded independently of the
source text — which is exactly what the agent-authoring and review workflows
need.

```text
Blueprint {
  graph_id:   String,             // the graph name
  start:      String,             // validated start node
  channels:   Vec<ChannelSpec>,   // { name, reducer, args }
  nodes:      Vec<NodeSpec>,       // see below
  edges:      Vec<EdgeSpec>,      // { from, to }
  defaults:   Vec<(String, Literal)>,
  input:      Vec<IoFieldSpec>,   // { name, ty }  (graph input shape)
  output:     Vec<IoFieldSpec>,   // { name, ty }  (graph output shape)
  checkpoint: Option<String>,     // graph-level checkpoint policy
  interrupt:  Option<String>,     // graph-level interrupt policy
  joins:      Vec<JoinSpec>,      // { sources, target } barriers
  provenance: Option<BlueprintProvenance>,
}
```

A `NodeSpec` carries far more than topology: `name`, `kind`, `model?`, `prompt?`,
`tools`, `routing`, plus `agent?` (a `subagent` reference), `subgraph?`,
`script?` (a `repl_agent` script capability — declaration only, never inline
code), `input?` (an input-mapping name), `command?` ([`CommandSpec`] `{ goto?,
update }`), `sends` ([`SendSpec`] `{ target, input? }` fanout), `join_sources`,
`options` (interrupt choices), `checkpoint?`, `timeout?`, `retry`, and
`metadata`. `NodeSpec::routing` is one of `Next(target)`,
`Conditional([(label, target), …])`, or `Terminal`. An unspecified node `kind`
defaults to `"model"` during compilation. A `ChannelSpec` additionally carries
`args` (reducer policy arguments such as a named aggregate reducer or a barrier
count). All optional/empty fields are skipped on serialization.

## Grammar

The grammar below is exactly what `parser.rs` accepts today:

```text
program       = graph_decl*
graph_decl    = "graph" ident "{" graph_item* "}"

graph_item    = "start" ident
              | "defaults"   "{" ( ident literal )* "}"
              | "input"      "{" ( ident ident )* "}"    // name type
              | "output"     "{" ( ident ident )* "}"
              | "checkpoint" ident                        // graph-level policy
              | "interrupt"  ident
              | "channel" ident ident ( string | number )*  // name reducer arg*
              | "join" "[" ident_list "]" "->" ident
              | node_decl
              | edge_decl                                  // ident "->" ident

node_decl     = "node" ident "{" node_item* "}"
node_item     = "kind"    ident
              | "model"   string
              | "system"  string                  // alias for `prompt`
              | "prompt"  string
              | "tools"   "[" ( string ("," string)* )? "]"
              | "next"    ident
              | "routes"  "{" ( ident "->" ident )* "}"
              | "agent"   string                  // subagent reference
              | "graph"   string                  // subgraph reference
              | "script"  string                  // repl_agent script name
              | "input"   string                  // input mapping name
              | "command" "{" ( "goto" ident | "update" "{" (ident literal)* "}" )* "}"
              | "sends"   "[" ( "send" ident string? )* "]"
              | "sources" "[" ident_list "]"       // join node upstreams
              | "options" "[" ( string ("," string)* )? "]"
              | "checkpoint" ident
              | "timeout"  literal
              | "retry"    "{" ( ident literal )* "}"
              | "metadata" "{" ( ident literal )* "}"

literal       = string | number | ident
ident_list    = ( ident ("," ident)* )?
```

Notes:

- `END` is a reserved terminal target; it is written as a bare identifier.
- Keywords have no dedicated tokens — they are lexed as identifiers and
  recognised contextually, so a name that happens to match a keyword can still be
  used where the grammar allows it.
- `system` and `prompt` both populate a node's prompt; `system` is accepted as an
  alias.
- Not yet implemented (the AST/`Blueprint` leave room): state-schema declarations
  (`state Name { … }`), `steering` policy lowering, and duration literals like
  `60s` (write timeouts as a number or quoted string).

## A worked example

```text
// A support workflow with a tool loop.
graph support_agent {
  start agent

  defaults {
    recursion_limit 50
    backoff "exponential"
    checkpoint inherit
  }

  channel messages messages
  channel tool_calls append

  node agent {
    kind agent
    model "default"
    system "Resolve support requests using tools when useful."
    tools ["lookup_user", "create_ticket"]
    routes {
      tool_call -> tools
      final -> END
    }
  }

  node tools {
    kind tool_executor
    next agent
  }
}
```

Compiling and binding it from Rust (see the runnable
[`examples/rag_blueprint.rs`](https://github.com/tinyhumansai/tinyagents/blob/main/examples/rag_blueprint.rs)):

```text
use tinyagents::language::parser::parse_str;
use tinyagents::language::compiler::{compile, bind_capabilities, CapabilityResolver};

let program   = parse_str(SUPPORT_AGENT)?;        // lex + parse
let blueprint = compile(&program)?.remove(0);     // semantic compile

let allow = CapabilityResolver::from_lists(
    ["default".to_string()],                       // allowed models
    ["lookup_user".to_string(), "create_ticket".to_string()], // allowed tools
);
bind_capabilities(&blueprint, &allow)?;            // registry gate
```

The compiled blueprint reports:

- `start` = `agent`
- channels `messages` (reducer `messages`) and `tool_calls` (reducer `append`)
- node `agent` (kind `agent`, model `"default"`, tools `[lookup_user,
  create_ticket]`) with conditional routes `tool_call -> tools`, `final -> END`
- node `tools` (kind `tool_executor`) with `next -> agent`

This is the textbook agent loop: the agent node calls the model, routes to the
tool executor when there is a tool call, and back, until it routes `final` to
`END`.

Run it:

```text
cargo run --example rag_blueprint
```

## Self-authoring: a model emits, compiles, and runs `.rag`

The deepest recursion in TinyAgents is a model **writing the workflow it runs
inside**. Because `.rag` is declarative and registry-bound, a model's output
passes through the *exact same* `parse -> compile -> bind -> build_graph` path as
a human-authored file — with the capability allowlist as the safety boundary.
The model never executes code; it only produces source that a Rust-side
`NodeFactory` materialises.

[`examples/openai_self_blueprint.rs`](https://github.com/tinyhumansai/tinyagents/blob/main/examples/openai_self_blueprint.rs)
demonstrates the full loop:

1. Ask the model (OpenAI) to output **only** `.rag`
   source, handing it the grammar plus a worked example in the system prompt.
2. Strip any ``` fences and feed the text to `parse_str` → `compile`.
3. Bind the blueprint against a `CapabilityResolver` allowlist — only
   allowlisted models/tools pass; anything else is rejected.
4. `build_graph` with a trivial `NodeFactory`, then run to `END`.

If the model's output fails to parse, compile, or bind, the diagnostic and the
offending source are surfaced — never executed. This is precisely why generated
topology must flow through the compiler and policy checks instead of being
installed directly: producing a graph never grants new capabilities.

```text
cargo run --example openai_self_blueprint
```

## Provenance

`compile_with_provenance(&program, origin)` records where every piece of a
blueprint came from. The result, surfaced through `Blueprint::provenance()`, is a
[`BlueprintProvenance`] holding the [`Origin`] plus the source [`Span`] of the
`graph` declaration and of each node, channel, and static edge:

- `Origin::File(path)` (`Origin::file`) — authored by a human at a path.
- `Origin::Generated(Option<label>)` (`Origin::generated` /
  `Origin::generated_by`) — emitted by a model/REPL session.

`Origin` is the trust-relevant half: review tooling treats a `Generated`
blueprint differently from a `File` one even though both compile through the same
gate. `BlueprintProvenance::node_span(name)` and `channel_span(name)` map a name
back to its declaration span, letting a UI or test trace each compiled piece to
its exact source. Plain `compile` leaves `provenance` as `None`, so its output is
byte-for-byte unchanged.

## Diffing blueprints

`blueprint_diff(&old, &new)` computes a structured, deterministic
[`BlueprintDiff`] — the basis for reviewing a model-authored plan against the
version it replaces. It reports graph-identity changes (`graph_id`, `start`),
nodes added / removed / `nodes_changed` (per-field [`NodeDiff`] over `kind`,
`model`, `prompt`, `tools`, `routing`, `agent`, `subgraph`, `script`, `input`,
`join_sources`, `options`, `checkpoint`, `timeout`), channels added / removed /
reducer-changed ([`ChannelDiff`]), and static edges added / removed. The diff
ignores provenance — only compiled topology and bindings are compared, so the
same pair always yields the same diff. `BlueprintDiff::is_empty()` tests
equivalence, and its `Display` renders a `+`/`-`/`~` summary.

## Diagnostics

Lexer, parser, and resolver errors are built from structured [`Diagnostic`]s
(`language::diagnostic`). A `Diagnostic` carries a [`Severity`]
(`Error`/`Warning`/`Note`), a headline, a primary [`Span`], optional secondary
[`Label`]s, an optional stable `code`, and optional `help`. Rendered against a
[`SourceFile`] (or a [`SourceMap`] of many), it produces a caret-underlined
presentation pointing at the offending source; `render_plain` gives a source-free
`line:column` anchor when no text is available. [`Span`]s are byte+line/column
ranges that `merge`. This is the machinery behind both `parse_str`'s caret
errors and the resolver's coded diagnostics.

## Testing helpers

`language::testkit` offers deterministic helpers for asserting on compiled
output: `try_compile` / `compile_all` / `blueprint` (compile source to a single
blueprint), `blueprint_with_provenance`, `node`, and the assertions
`assert_kind`, `assert_next`, `assert_terminal`, and `assert_route`.

## Where `.rag` fits

- `.rag` defines graph **topology and bindings** declaratively.
- [`.ragsh`](REPL-Language-RAGSH) is the **imperative** counterpart: it inspects,
  scripts, and recursively orchestrates harness/graph runs, and it can draft,
  validate, compile, and (under policy) register `.rag` blueprints through this
  same compiler.
- Both lower into the same [graph](Graph-Runtime) + [harness](Harness) runtime as
  hand-written Rust.

## Errors

| Error                          | Phase                | Examples                                            |
|--------------------------------|----------------------|-----------------------------------------------------|
| `TinyAgentsError::Parse`       | lexer / parser       | unterminated string, invalid escape, unexpected token |
| `TinyAgentsError::Compile`     | `compile` / node kind | duplicate node, missing/undefined start, unknown target, mixed routing, unknown node kind |
| `TinyAgentsError::Capability`  | binding              | unregistered model, tool, agent, subgraph, router, or reducer |

The source-gate `Resolver` attaches stable codes to its diagnostics
(`E-rag-unknown-model`, `-tool`, `-subgraph`, `-router`, `-agent`, `-reducer`,
`E-rag-invalid-node-kind`); an invalid node kind folds into `Compile`, every
other reference into `Capability`.

## See also

- [Graph Runtime](Graph-Runtime) — the durable runtime blueprints lower into.
- [Registry](Registry) — the capability catalog `.rag` binds against by name.
- [REPL Language (.ragsh)](REPL-Language-RAGSH) — the imperative RLM/CodeAct surface.
- [Recursion and RLM](Recursion-and-RLM) — self-authoring and the recursive model.
