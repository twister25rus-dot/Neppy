# TinyAgents Wiki

**TinyAgents is a recursive language-model (RLM) harness for Rust:** a typed,
durable runtime where language models call models, agents call agents, graphs
run graphs, and a model can author, compile, and run the very workflow it is
standing inside — all as inspectable, checkpointed, policy-checked Rust.

> **New here?** Jump to **[Capabilities](Capabilities)** — the single master
> index where an agent or human can discover *every* functionality of the crate
> in one place and branch straight to the deep page for each.

The repository `README.md` is the marketing-level overview (hero, diagrams,
30-second start). This wiki is the implementation-oriented companion: how each
surface works, how to set things up, how to develop against the crate, and the
RLM execution model in depth.

- crate: `tinyagents` (Rust 2024 edition, GPL-3.0-only)
- crates.io: <https://crates.io/crates/tinyagents>
- docs.rs: <https://docs.rs/tinyagents>
- source: <https://github.com/tinyhumansai/tinyagents>

## Why "recursive"

TinyAgents is architected around the Recursive Language Model execution model:
treat a long prompt as an external *environment* a model interacts with through
a REPL — examining, decomposing, and recursively calling itself (or sub-models)
over snippets — instead of stuffing everything into one context window. The
crate brings that model to Rust as a typed, durable harness. Recursion shows up
concretely as:

- **Sub-agents** — a harness agent is exposed *as a tool* to another agent
  (`harness::subagent`), so orchestration is just a model calling a model.
- **Recursion policy + depth tracking** — runs carry `root_run_id` /
  `parent_run_id`, a recursion limit is enforced, and child runs roll their
  events, usage, and cost up to the parent.
- **Graphs that run graphs** — subgraph nodes (`graph::subgraph`) embed another
  compiled graph; the REPL can drive a graph from inside a graph node.
- **The REPL as the RLM core** — `.ragsh` makes context and prompts runtime
  *values*; the model writes small programs, inspects output, and calls
  sub-models / sub-agents / sub-graphs as functions.
- **Self-authoring** — a model can emit a `.rag` blueprint that compiles through
  the same registry-bound path as a human-authored file and runs on the same
  runtime the model is already executing in.

See [Recursion and RLM](Recursion-and-RLM) for the full treatment and research
lineage (Zhang, Kraska, Khattab, "Recursive Language Models," 2025).

## The five surfaces

TinyAgents is organized around five surfaces, used consistently across the docs:

1. **[Harness](Harness)** (`src/harness/`) — provider-neutral model calls, typed
   tools, middleware, structured output, streaming, usage/cost, retry/limits,
   cache, memory/embeddings, sub-agents, steering, summarization, testkit.
2. **[Graph runtime](Graph-Runtime)** (`src/graph/`) — durable typed state
   graphs: `START`/`END`, nodes, edges, conditional routing, commands, `Send`
   fanout, reducers/channels, checkpoints, interrupts, subgraphs, streaming,
   topology export, time travel.
3. **[Registry](Registry)** (`src/registry/`) — named capability catalog
   (models, tools, agents, graphs, stores, middleware, policy) that
   `.rag`/`.ragsh` bind by name.
4. **[Expressive language `.rag`](Expressive-Language-RAG)** (`src/language/`) —
   declarative, side-effect-free blueprint format that compiles into the same
   graph/harness runtime; the safe boundary for agent-authored plans.
5. **[REPL language `.ragsh`](REPL-Language-RAGSH)** (`src/repl/`) — imperative,
   capability-bound interactive orchestration; the RLM/CodeAct loop surface.

## Page index

Getting started

- [Home](Home) — this page.
- [Capabilities](Capabilities) — master index of every crate functionality.
- [Quick Start](Quick-Start) — install, test, run an example, crate layout.
- [Examples](Examples) — runnable graph, harness, provider, tool, structured
  output, sub-agent, and `.rag` examples.

Concepts

- [Recursion and RLM](Recursion-and-RLM) — the RLM execution model, the concrete
  recursive surfaces, and research lineage.
- [Architecture](Architecture) — how the five surfaces fit together.

Modules

- [Harness](Harness) — model calls, tools, middleware, sub-agents, testkit.
- [Graph Runtime](Graph-Runtime) — durable typed state graphs.
- [Registry](Registry) — named capability catalog.
- [Expressive Language (.rag)](Expressive-Language-RAG) — declarative blueprints.
- [REPL Language (.ragsh)](REPL-Language-RAGSH) — interactive orchestration.

Providers and contributing

- [Providers](Providers) — OpenAI, Anthropic, Ollama, DeepSeek, Groq, xAI,
  OpenRouter, Together, Mistral, and OpenAI-compatible endpoints.
- [Development](Development) — building, testing, linting, and contributing.

## Repository links

- [Main repository](https://github.com/tinyhumansai/tinyagents)
- [System specification](https://github.com/tinyhumansai/tinyagents/blob/main/docs/spec/README.md)
- [Examples directory](https://github.com/tinyhumansai/tinyagents/tree/main/examples)
- [Contributing guide](https://github.com/tinyhumansai/tinyagents/blob/main/CONTRIBUTING.md)
