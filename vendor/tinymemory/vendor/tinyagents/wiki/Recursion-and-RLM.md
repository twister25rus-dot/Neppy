# Recursion and the Recursive Language Model (RLM)

> **TinyAgents is a recursive language-model (RLM) harness for Rust:** a typed,
> durable runtime where language models call models, agents call agents, graphs
> run graphs, and a model can author, compile, and run the very workflow it is
> standing inside — all as inspectable, checkpointed, policy-checked Rust.

Recursion is not a feature bolted onto TinyAgents; it is the spine the five
surfaces (Harness, Graph runtime, Registry, `.rag`, `.ragsh`) are arranged
around. This page explains the RLM execution model, cites the research it draws
on, and then walks through **each concrete recursive mechanism** with the real
type and module names you will find in `src/`.

---

## 1. The RLM execution model

A conventional agent stuffs everything — instructions, history, retrieved
documents, tool output — into a single context window and asks the model to
reason over the whole blob in one pass. As the prompt grows, quality degrades
("context rot"), and the effective reasoning budget shrinks.

A **Recursive Language Model (RLM)** flips this around. The long prompt is
treated as an external *environment* — a value the model interacts with through
a REPL — rather than a wall of text it must read all at once. The model:

- **examines** the environment (peeks at slices, sizes, structure),
- **decomposes** the problem into smaller sub-problems,
- **recursively calls itself or sub-models** over individual snippets, and
- **iterates**, folding partial results back into its working state.

Because each recursive call sees only the slice it needs, the *effective*
context the system can handle exceeds the raw window of any single model call.
Prompts and context become **runtime values you can pass to functions**, not
just text you concatenate.

### Research lineage

TinyAgents is **inspired by and architected around** the RLM execution model. It
is *not* a reimplementation of the paper and makes no claim to reproduce its
benchmark results. The references:

- **Paper:** "Recursive Language Models," Alex L. Zhang, Tim Kraska, Omar
  Khattab (MIT CSAIL), 2025. arXiv:2512.24601 —
  <https://arxiv.org/abs/2512.24601>
- **Blog:** Alex L. Zhang, "Recursive Language Models" —
  <https://alexzhang13.github.io/blog/2025/rlm/>
- **Reference implementation:** <https://github.com/alexzhang13/rlm>

It also borrows the **CodeAct** idea — agents that act by *writing and running
small programs* instead of emitting opaque tool-call JSON — and the durable
state-graph execution model from LangGraph.

### What TinyAgents adds to the idea

The RLM papers describe the execution model. TinyAgents brings it to Rust as a
**production-shaped harness**: sub-model / sub-agent / sub-graph calls are
ordinary typed function calls, sessions are persistent values, depth is tracked
and capped, and every nested call rolls up into one observable run tree with
events, usage, and cost. The recursion is *typed, durable, and policy-checked*
rather than ad hoc.

---

## 2. The recursive mechanisms in TinyAgents

There are five concrete places recursion is implemented or specified. Each is
grounded in a real module below.

| # | Mechanism | Where | Recursion shape |
|---|-----------|-------|-----------------|
| 1 | Sub-agents (harness + graph node) | `harness::subagent`, `graph::subagent_node` | agent → agent (as a tool or graph node) |
| 2 | Depth tracking + recursion policy | `harness::context`, `harness::limits`, `graph::recursion` | bounded, observable run tree |
| 3 | Subgraphs | `graph::subgraph`, `graph::compiled` | graph → graph |
| 4 | `.ragsh` REPL | `repl` | model → REPL → sub-model/agent/graph |
| 5 | Self-authoring | `language`, examples | model → `.rag` → runtime it runs in |

---

## 3. Sub-agents: agents calling agents

The most direct recursion is the **agent-calling-agent** primitive in
[`harness::subagent`](Harness.md). Three public types:

- **`SubAgent`** wraps an `Arc<AgentHarness<State, Ctx>>` plus a stable `name`,
  `description`, and optional fixed `system_prompt`. Invoking it always produces
  a **child run** one level deeper in the recursion tree than the caller.
- **`SubAgentTool`** adapts a `SubAgent` into a `Tool`. This is the key move:
  *an entire agent becomes a single tool call*. When a parent model decides to
  delegate, it "calls a tool" — and that tool is another full agent loop.
- **`SubAgentSession`** keeps one `SubAgent` (and its harness) alive across many
  turns, accumulating the conversation transcript. This is *post-completion
  reuse* for human-in-the-loop flows — distinct from *steering*, which
  interrupts a still-running agent.

```text
parent agent loop
  └─ model emits tool call: "researcher" { input: "summarize doc X" }
       └─ SubAgentTool::call
            └─ SubAgent::invoke  →  child AgentHarness loop (depth + 1)
                 └─ returns final assistant text as the ToolResult
```

Exposing an agent as a tool (real signatures from `src/harness/subagent`):

```rust
let researcher = SubAgent::new("researcher", "Researches a topic", harness)
    .with_system_prompt("You are a careful research assistant.");

// Wrap the whole agent as one tool the parent model can call.
let tool = SubAgentTool::new(Arc::new(researcher));
parent_harness.register_tool(Arc::new(tool));
```

Reusing the same child across human turns with `SubAgentSession`:

```rust
let mut session = SubAgentSession::from_subagent(researcher);

// Turn 1: the child runs over the transcript and folds its reply back in.
let run = session.send(&state, ctx, vec![Message::user("Outline the report")]).await?;
// ... obtain human feedback out of band ...
// Turn 2: same harness, full prior context retained — nothing is rebuilt.
let run = session.send(&state, ctx, vec![Message::user("Now expand section 2")]).await?;
```

See `examples/orchestrator_subagents.rs` for an orchestrator that fans work out
to several specialist sub-agents.

---

## 4. Recursion policy: depth tracking and the run tree

Unbounded self-calling is a footgun, so recursion in TinyAgents is **bounded and
observable**.

**Depth tracking.** Every run carries a `depth` in its `RunConfig`
(`harness::context`). A top-level run is depth `0`. When a `SubAgent` is invoked
at `parent_depth`, the child run is created at `parent_depth + 1`. The child run
also gets a derived id/name (e.g. `"{name}-d{child_depth}"`, or
`"{name}-t{turn}-d{depth}"` for a session) so nested runs are distinguishable in
logs and checkpoints.

**The depth cap.** The limit is `RunLimits::max_depth`
(`harness::limits`, default `RunLimits::DEFAULT_MAX_DEPTH` = `8`), read from the
child harness's `RunPolicy`. If a child run *would* exceed the cap, the
invocation fails fast with `TinyAgentsError::SubAgentDepth(max_depth)` — a
deterministic guard that triggers **before any model call**, so runaway
recursion is cheap to stop.

```text
RunConfig.depth  : 0 ──► 1 ──► 2 ──► …  (each sub-agent invocation +1)
RunLimits.max_depth = 8  (TinyAgentsError::SubAgentDepth if a child would exceed)
```

**One observable run tree.** Each invocation emits `AgentEvent::SubAgentStarted`
and `SubAgentCompleted` (carrying the sub-agent name and child depth);
`SubAgentSession` adds `SubAgentReused` on reuse turns. When a sub-agent is
invoked with a shared `EventSink` — via `SubAgent::invoke_with_events` or
`SubAgent::invoke_in_parent` — the child run's own events also flow onto the
parent sink, so one observer sees the **entire nested tree** of runs, usage, and
cost rolled up to the parent.

**Parent / root run identity (graph form).** The graph-embedded sub-agent node
is implemented in [`graph::subagent_node`](Graph-Runtime.md). `subagent_node(...)`
lowers a `SubAgentNode` (an agent `ComponentId` plus an `InputMapper`, an
`OutputMapper`, and a `SubAgentPolicy`) and a `CapabilityRegistry` into an
ordinary graph `Handler`: it resolves the agent by name, creates a **distinct
child `run_id` that preserves the run tree's `root_run_id`** and is **parented to
the enclosing graph run**, applies timeout / retry / `SubAgentBudget` policy,
maps the child output back into the parent update, records the child run (with
its usage) onto the parent execution rollup, and forwards the child's harness
events onto the node's event sink. `HarnessSubAgent` adapts a harness `SubAgent`
into a registry-storable `HarnessAgent` so the same primitive from section 3 can
back a graph node. The design notes live in
`docs/modules/graph/subagents-recursion.md`.

**The recursion stack and run tree (graph form).** `graph::recursion` makes the
run tree a first-class, serializable value. A `RecursionStack` holds the chain of
`RecursionFrame`s (each carries `graph_id`, optional `node_id`, `run_id`,
`namespace`, `depth`, and the `parent` run id) from the root run down to the
currently executing run, bounded by a `RecursionPolicy` with **three independent
caps**:

- `max_depth` (default `25`) bounds run-tree depth; pushing past it fails with
  `TinyAgentsError::SubAgentDepth`.
- `max_visits_per_node` (default unset) optionally bounds how many times one node
  may activate within a run, failing with `TinyAgentsError::NodeVisitLimit`.
- `max_total_steps` (default `1000`) bounds super-steps per run, failing with
  `TinyAgentsError::RecursionLimit`.

After a run completes, a `RunTree` (`run_id`, `root_run_id`, optional
`parent_run_id`, and the `children: Vec<ChildRun>` spawned from its nodes) is the
after-the-fact summary a caller reads; child runs are accumulated through a
`ChildRunSink`. This is the graph-level counterpart to the harness `RunLimits`
depth cap (section 3) — the two enforce recursion bounds at the graph-step and
agent-loop layers respectively.

---

## 5. Graphs that run graphs (subgraphs)

A node in one [graph](Graph-Runtime.md) can embed an entire **compiled graph**.
This is recursion at the topology level: a durable Pregel-style superstep
executor (`graph::compiled`, `CompiledGraph`) driving another `CompiledGraph`
inside one of its nodes.

`graph::subgraph` provides two adapters:

- **`shared_subgraph_node(child)`** — parent and child share the same
  `State`/`Update` channel (`Update == State`). The child runs over the parent
  state passed into the node; its final state becomes the parent update.
- **`adapter_subgraph_node(child, to_child, from_child)`** — parent and child
  have *different* state shapes. `to_child` projects parent state `P` into the
  child input `C`; `from_child` folds the child's final state back into a parent
  update `PU`.

Both adapters append the embedding node id to the child's **checkpoint
namespace**, so parent and child checkpoint ids never collide — durability
composes recursively.

```text
parent CompiledGraph
  START ─► plan ─► [ shared_subgraph_node(child) ] ─► finish ─► END
                          │
                          └─ child CompiledGraph: START ─► … ─► END
                             (checkpoints namespaced under the node id)
```

```rust
let child: CompiledGraph<State, State> = child_builder.compile()?;
parent_builder.add_node("worker", shared_subgraph_node(child));
```

Because a node handler is just async Rust, the graph → REPL → graph composition
also holds: a node can drive a `.ragsh` session that itself runs another graph,
nesting orchestration and execution layers.

---

## 6. The `.ragsh` REPL as the RLM core

The [`repl`](REPL-Language-RAGSH.md) module (`.ragsh`) is the surface that maps
most directly onto the RLM / CodeAct loop. It is **imperative and
capability-bound**: an operator (a human *or* a parent orchestrator model)
drives a session by issuing typed commands that are policy-checked before they
reach the runtime.

The RLM correspondence:

- **Context and prompts are values, not just text.** A `ReplSession` holds
  `variables: HashMap<String, serde_json::Value>` — session-scoped state set with
  `set <key> <value>` and read with `get <key>`. A long document, a partial
  result, or a sub-prompt is a *named value* you pass around, exactly the "prompt
  as an environment / variable" idea from the RLM paper.
- **Sub-model / sub-agent / sub-graph calls are functions.** The `call
  <capability> <json>` verb invokes a registered capability by name; `run <graph>
  <input>` drives a compiled graph; `compile <name>` and `load <path>` bring new
  capabilities into scope. The model writes a small program (a sequence of
  commands), inspects each command's `ReplOutcome`, and iterates.
- **The capability boundary is the safety gate.** A `CapabilityPolicy` allowlist
  governs which capability names a session may invoke; calling anything off the
  list is rejected. The model can act, but only through named, audited
  capabilities — never arbitrary host code.

```text
# A .ragsh trajectory: context as values, sub-calls as functions
set doc      "<a very long document>"
set question "What are the three key risks?"
call summarizer {"input": "<slice of doc>"}   # recursive sub-model call
run triage_graph "<intermediate result>"      # graph driven from the REPL
show vars                                      # inspect accumulated state
```

Two REPL surfaces exist. The line-oriented command grammar (`help`, `quit`,
`load`, `compile`, `run`, `set`, `get`, `show`, `call`) over the simple
`repl::ReplSession` remains a skeleton. The *wired* surface is the Rhai-backed
`repl::session::ReplSession` (behind the `repl` Cargo feature): session
variables are JSON values and recursive sub-calls are real function built-ins —
`model_query`, `tool_call`, `agent_query`, `graph_run` (and `*_batched`
variants) — each bounded by a `ReplPolicy` allowlist and bridged to the live
harness/graph runtime. Either way the *shape* — values + capability-bound
sub-calls + iteration — is the RLM loop. See
[REPL Language (.ragsh)](REPL-Language-RAGSH) for the full surface.

---

## 7. Self-authoring: the deepest recursion

The deepest form of recursion is a model that **authors the workflow it is
running inside**. A model emits a `.rag` blueprint; that blueprint compiles
through the *same* registry-bound compiler path a human-authored file uses, and
runs on the *same* runtime the model is already executing in.

`examples/openai_self_blueprint.rs` does exactly this:

```text
1. Ask the model to output ONLY .rag source (grammar + worked example in prompt).
2. Strip ``` fences from the reply  →  raw .rag text.
3. Run the SAFE pipeline:
     parse_str ─► compile ─► Blueprint
       ─► bind_capabilities(resolver)   # policy gate: only allowlisted models/tools
       ─► build_graph(factory)          # Rust factory materialises node behaviour
       ─► graph.run(...)                # execute to END
```

Two safety properties make this sound:

- **The model never executes code.** `.rag` is declarative and side-effect-free
  ([`language`](Expressive-Language-RAG.md)): it only references capabilities by
  name and describes topology. Node *behaviour* is supplied by a Rust
  `NodeFactory`.
- **The capability allowlist is the boundary.** `bind_capabilities` against a
  `CapabilityResolver` rejects any model or tool the generated blueprint
  references that is not allowlisted — anything the model invented outside the
  allowed set is caught at bind time, before execution.

`examples/rag_blueprint.rs` shows the human-authored counterpart: the same
`parse_str → compile → bind_capabilities` pipeline over a hand-written
`support_agent` blueprint — proving the model-authored and human-authored paths
are *one path*.

```text
        ┌─────────────── one compiler path ───────────────┐
human ──┤ .rag source ─► parse ─► compile ─► bind ─► run   │
model ──┘  (self-authored, same grammar, same policy gate) │
        └──────────────────────────────────────────────────┘
```

This is what "the harness can describe and re-enter itself" means concretely.

---

## 8. Two languages, one runtime

Both authored languages **lower into the exact same `graph` + `harness` types**
as hand-written Rust:

- **`.rag`** ([Expressive Language](Expressive-Language-RAG.md)) — declarative,
  side-effect-free blueprints; the safe boundary for *agent-authored plans*.
  Pipeline: `lexer → parser → compiler → Blueprint → graph`.
- **`.ragsh`** ([REPL Language](REPL-Language-RAGSH.md)) — imperative,
  capability-bound interactive orchestration; the RLM / CodeAct *loop surface*.

A program in either language is interpreted by the same runtime it targets — a
language whose programs are the runtime that interprets them. That closure is the
final turn of the recursive spiral.

---

## See also

- [Architecture](Architecture.md) — how the five surfaces layer with recursion
  as the spine.
- [Harness](Harness.md) — sub-agents, sessions, depth limits, events.
- [Graph Runtime](Graph-Runtime.md) — compiled graphs, subgraphs, checkpoints.
- [REPL Language `.ragsh`](REPL-Language-RAGSH.md) — the RLM/CodeAct loop.
- [Expressive Language `.rag`](Expressive-Language-RAG.md) — declarative
  blueprints and self-authoring.
