# Architecture

TinyAgents is a **recursive language-model (RLM) harness for Rust**: a typed,
durable runtime where models call models, agents call agents, graphs run graphs,
and a model can author, compile, and run the very workflow it is standing inside.
It synthesizes ideas from LangChain (the harness model), LangGraph (durable state
graphs), and CodeAct / Recursive Language Models (context as runtime values,
recursive sub-calls as functions) — without cloning any one of them. The goal is
to make durable, *self-composing* agent systems fit naturally in Rust.

See [Recursion and the RLM](Recursion-and-RLM.md) for the conceptual core. This
page shows how the five surfaces layer together, with recursion as the spine.

## The five surfaces

| Surface | Module | Owns |
|---------|--------|------|
| **Harness** | `src/harness/` | provider-neutral model calls, typed tools, middleware, structured output, streaming, usage/cost, retry/limits, cache, memory, **sub-agents**, steering |
| **Graph runtime** | `src/graph/` | durable typed state graphs: `START`/`END`, nodes, edges, conditional routing, commands, `Send` fanout, reducers/channels, checkpoints, interrupts, **subgraphs**, parallel-agent orchestration/forking, streaming, observability, time travel |
| **Registry** | `src/registry/` | named capability catalog (models, tools, agents, graphs, stores, middleware, policy) that `.rag`/`.ragsh` bind by name |
| **Expressive `.rag`** | `src/language/` | declarative, side-effect-free blueprints; the safe boundary for agent-authored plans |
| **REPL `.ragsh`** | `src/repl/` | imperative, capability-bound interactive orchestration; the RLM/CodeAct loop surface |

## Layered model

The two authored languages compile *down* into the same graph + harness types as
hand-written Rust. The registry binds names to capabilities for both. Recursion
threads vertically through every layer (right-hand column).

```text
                              author / inspect
                         +------------------------+        recursion spine
                         | .rag blueprint source  |   model authors .rag
                         | .ragsh REPL commands    |   REPL: context = values,
                         +-----------+------------+    sub-calls = functions
                                     |                          |
                                     v                          |
                         +------------------------+             |
                         | Lexer / parser / compiler            |
                         | syntax -> Blueprint     |            |
                         +-----------+------------+             |
                                     |                          |
                                     v                          |
+----------------+       +------------------------+       +-----v----------+
| Application    |------>| Capability Registry    |<------| Model Catalog  |
| Rust builders  |       | names -> capabilities  |       | prices/profile |
+-------+--------+       +-----------+------------+       +----------------+
        |                            |
        |                            v
        |                +------------------------+        graph runs graph
        +--------------->| Graph Runtime          |  ----> subgraph node
                         | state | reducers       |        embeds a
                         | nodes | routes | runs  |        CompiledGraph
                         +-----------+------------+
                                     |
                                     v
                         +------------------------+        agent calls agent
                         | Agent Harness          |  ----> SubAgentTool runs
                         | prompts | context      |        a child run at
                         | tools | middleware     |        depth + 1
                         | events | usage | retry |
                         +-----+------------+-----+
                               |            |
                               v            v
                    +----------------+  +----------------+
                    | Provider       |  | Tool Registry  |
                    | Adapters       |  | typed schemas  |
                    +-------+--------+  +-------+--------+
                            |                   |
                            v                   v
                    +----------------+  +----------------+
                    | OpenAI         |  | Local systems  |
                    | Anthropic      |  | APIs, stores   |
                    | Ollama, etc.   |  | side effects   |
                    +----------------+  +----------------+
```

## Recursion as the spine

Each layer composes with itself, and the layers compose with each other. This is
what earns TinyAgents the RLM label:

```text
  model ──► .rag blueprint ──► same compiler ──► same runtime the model runs in
    (self-authoring: examples/openai_self_blueprint.rs)

  agent ──► SubAgentTool ──► child agent run (depth + 1, capped, observable)
    (harness::subagent)

  graph ──► shared_subgraph_node / adapter_subgraph_node ──► child CompiledGraph
    (graph::subgraph, namespaced checkpoints)

  REPL ──► call <capability> / run <graph> ──► sub-model / sub-agent / sub-graph
    (repl: context as values, sub-calls as functions)
```

Every nested call stays *typed, durable, and observable*: depth is tracked and
capped, child events/usage/cost roll up to the parent, and child checkpoints are
namespaced so durability composes. See
[Recursion and the RLM](Recursion-and-RLM.md) for the full walkthrough.

## Harness

The harness owns nondeterministic agent work:

- model request construction and provider adapter calls
- tool registration and execution
- middleware hooks, structured output strategy, streaming deltas
- provider error normalization, usage and cost records
- retries, limits, cancellation, cache, memory/embeddings
- **sub-agents** (`harness::subagent`): a whole agent exposed as a tool, run as a
  child at depth `+1`, capped by `RunLimits::max_depth` and reported through
  `SubAgentStarted` / `SubAgentCompleted` / `SubAgentReused` events
- deterministic model and tool test doubles (`harness::testkit`)

The graph can call the harness, but provider details stay outside graph
execution. See [Harness](Harness.md).

## Graph runtime

The graph owns deterministic state movement:

- typed state, reducers/channels, and updates
- nodes, static and conditional edges, commands, `Send` fanout
- checkpoints, interrupts, time travel, topology export, run status, observability
- **child-work orchestration** (`graph::orchestration`): typed task controls
  (`spawn`/`await`/`cancel`/`kill`/`status`/`list`/`timeout`/`race`/`yield`/
  `steer`) exposed as harness tools, so an orchestrator manages concurrent child
  tasks by stable `TaskId` without touching raw executor handles
- **subgraphs** (`graph::subgraph`): a node embeds another `CompiledGraph`, with
  the child's checkpoint namespace extended by the embedding node id

Graph execution should be understandable from topology, state, routes,
checkpoints, and emitted events. See [Graph Runtime](Graph-Runtime.md).

## Registry

The registry owns named capabilities, letting Rust code, `.rag` blueprints, and
`.ragsh` sessions refer to capabilities by stable names instead of reaching into
process globals. Entries describe models, tools, agents, graphs, stores,
middleware, and policies — making generated or user-authored workflow source
bindable and auditable *before* execution.

## Expressive language `.rag`

`.rag` is the declarative, side-effect-free blueprint format. It cannot embed
arbitrary code; it only references capabilities by name, which the compiler binds
and validates against a registry — the **safe boundary for agent-authored
plans**. Pipeline:

```text
.rag source -> lexer -> parser -> compiler -> Blueprint
            -> bind_capabilities (policy gate) -> build_graph -> run to END
```

Because the model-authored and human-authored paths use this *same* pipeline
(`examples/openai_self_blueprint.rs` vs. `examples/rag_blueprint.rs`),
self-authoring is just the runtime re-entering itself through the front door. See
[Expressive Language `.rag`](Expressive-Language-RAG.md).

## REPL language `.ragsh`

`.ragsh` is the imperative, capability-bound control surface and the RLM/CodeAct
loop: a `ReplSession` holds session variables as JSON *values*, and verbs (`set`,
`get`, `call`, `run`, `compile`, `load`, `show`) let an operator or orchestrator
inspect state and invoke sub-models / sub-agents / sub-graphs as functions. A
`CapabilityPolicy` allowlist is the safety gate. See
[REPL Language `.ragsh`](REPL-Language-RAGSH.md).

## Design rule

Each layer keeps its contract narrow:

- the harness calls providers and tools
- the graph moves typed state through explicit topology
- the registry binds names to capabilities
- `.rag` describes workflow source
- `.ragsh` controls registered capabilities interactively

No layer should bypass another layer's policy, observability, or validation
boundary — and that invariant is *exactly* what makes recursion safe: a
self-authored blueprint, a nested sub-agent, and a subgraph all pass through the
same gates as the top-level run.
