# Examples

The repository keeps runnable examples under
[`examples/`](../examples). Run every command from the repository root.

The catalog below is an annotated tour of **every** example, ordered from the
simplest local graph to the most deeply recursive — an agent that authors and
runs its own workflow. For each example: what it shows, how to run it, and which
recursive-language-model (RLM) concept it demonstrates.

Examples whose name starts with `openai_` (plus `orchestrator_subagents`) call a
real hosted provider and require the `openai` Cargo feature and an API key:

```sh
export OPENAI_API_KEY=...        # or copy .env.example to .env
cargo run --example <name>
```

The default build is **offline**: `basic_graph`, `complex_graph`,
`durable_graph`, `resilient_graph`, `agent_loop_tools`, `rag_blueprint`, and
`subconscious_loop` run with no key and no feature flag.

---

## 1. `basic_graph` (offline)

```sh
cargo run --example basic_graph
```

A minimal durable graph: a whole-state agent/tool loop. A `GraphBuilder` over
`Update == State` (the overwrite reducer, so each node returns the full next
state) wires an `agent` node and a `tool` node. Conditional edges route `agent ->
tool` while `needs_tool` is set, otherwise `agent -> END`; `tool` clears the flag
and loops back to `agent`.

**Recursive concept:** the base case. A typed state graph *is* a loop — a model
node and a tool node calling each other until a condition ends the run. This is
the smallest unit the deeper examples nest and compose.

## 2. `durable_graph` (offline)

```sh
cargo run --example durable_graph
```

Durable execution with **partial updates, a reducer, and checkpointing**. State
is a `Counter`; each node returns a small `i64` partial update, and a
`ClosureStateReducer` folds updates into the running counter while appending an
audit-log line. The graph runs on a thread backed by an `InMemoryCheckpointer`,
and the example lists the checkpoints written at each superstep boundary.

**Recursive concept:** durability/observability of a run. Checkpoints make each
superstep an inspectable value — the persistence foundation that lets a parent
pause, resume, and reason about a child computation.

## 2b. `resilient_graph` (offline)

```sh
cargo run --example resilient_graph
```

**Surviving network blips and restarting a failed run.** A `fetch` node fails
its first two attempts (a simulated connectivity blip) and is absorbed by
`with_node_retry` without operator involvement. A `commit` node then fails past
the retry budget; on a checkpointed thread the run persists a **resumable
failure-boundary checkpoint** and returns the error. The "outage" clears and
`retry(thread)` restarts the run from that checkpoint to completion — or, to
continue on user feedback, edit state with `update_state` before retrying.

**Recursive concept:** fault tolerance of a run. Transient failures are retried
in place; a hard failure becomes a durable, restartable value rather than a lost
computation. See [fault tolerance](../docs/modules/graph/fault-tolerance.md).

## 3. `complex_graph` (offline)

```sh
cargo run --example complex_graph
```

A nested **subgraph embedded inside a parallel fan-out / join**. `dispatch`
fans out (via `Command::goto`) into two branches that run concurrently. One
branch (`branch_sub`) is a subgraph embedded with `adapter_subgraph_node`; its
child *itself* embeds a grandchild via `shared_subgraph_node`, so a value flows
up through two subgraph levels (`0 -> +5 -> +1 = 6`). The reducer deterministically
joins the branches before `join` finalizes.

**Recursive concept:** *graphs that run graphs.* A node embeds another compiled
graph, which embeds another — a subgraph inside a subgraph — the structural form
of recursion in the graph runtime.

## 4. `agent_loop_tools` (offline)

```sh
cargo run --example agent_loop_tools
```

The harness agent loop end-to-end with a **real tool** and no network. A
`ScriptedModel` is scripted to first request a tool call, then (after seeing the
tool result) produce a final answer. A small `CalculatorTool` (`add`) is
registered so the loop has something to run. This is the canonical
model→tool→model loop that every higher example builds on.

**Recursive concept:** the harness agent loop — the substrate. A sub-agent is
just *this* loop invoked one level deeper, so understanding it is the key to the
recursive examples.

## 5. `openai_chat` (needs `OPENAI_API_KEY`)

```sh
cargo run --example openai_chat
```

The simplest hosted happy path: register an `OpenAiModel` in an `AgentHarness`,
set it as the default model, run one question through the default agent loop, and
print the answer plus token usage.

**Recursive concept:** the leaf call. A single provider-neutral model invocation
— the atom that recursion is built from.

## 6. `openai_tools` (needs `OPENAI_API_KEY`)

```sh
cargo run --example openai_tools
```

The full tool-calling loop against a real provider. A local `get_weather` `Tool`
is registered alongside an `OpenAiModel`; a question triggers the tool, the
harness runs it locally, feeds the result back, and the model produces the final
answer.

**Recursive concept:** model → tool → model over the network. The tool boundary
here is exactly the boundary a sub-agent later occupies.

## 7. `openai_structured` (needs `OPENAI_API_KEY`)

```sh
cargo run --example openai_structured
```

Schema-constrained output. `RunPolicy::default_response_format` is set to a
`ResponseFormat::json_schema` describing a `{sentiment, score}` object; the
harness attaches it to every request and extracts the parsed JSON into the
run's structured response.

**Recursive concept:** typed channels between levels. Structured output is how a
parent reliably *reads* a child's result as data — the typed wire an orchestrator
needs to route on a sub-agent's answer.

## 8. `openai_graph_agent` (needs `OPENAI_API_KEY`)

```sh
cargo run --example openai_graph_agent
```

A durable graph whose node drives a real OpenAI-backed harness. An `AgentHarness`
is wrapped in an `Arc`, captured in a graph-node closure, and the graph runs
`START -> agent -> END`: the node calls the harness (which talks to OpenAI),
stores the answer in graph state, and ends.

**Recursive concept:** the two runtimes compose. The durable graph runtime and
the harness agent loop nest cleanly — a graph node *is* an agent — which is what
makes graph→agent→graph recursion possible.

## 9. `orchestrator_subagents` (needs `OPENAI_API_KEY`)

```sh
cargo run --example orchestrator_subagents
```

Flagship registry showcase: an orchestrator that *designs* which sub-agents to
call at runtime. Several specialized `SubAgent`s (`researcher`, `coder`,
`summarizer`) — each an `OpenAiModel` with a distinct system prompt — are wrapped
as `SubAgentTool`s and registered by name in a `CapabilityRegistry`. The flow:

1. **Register** named capabilities.
2. **Discover** the available names + descriptions back out of the registry
   (nothing hard-coded into the planner).
3. **Design** — an orchestrator agent, given the task plus the discovered menu,
   decides via structured output *which* sub-agents to invoke.
4. **Bind at runtime** — each chosen name is resolved with
   `CapabilityRegistry::tool`, the sub-agents run in parallel (`join_all`), and
   their results are composed into a final answer.

**Recursive concept:** *agents calling agents.* A model decides which other
agents to run, then runs them as tools — orchestration is one model invoking
models, with capabilities named, discovered, and bound at runtime rather than
wired in statically.

## 10. `rag_blueprint` (offline)

```sh
cargo run --example rag_blueprint
```

Compiles the spec's `support_agent` `.rag` blueprint. The example parses the
expressive-language source into a `Program`, `compile`s it into a `Blueprint`,
prints the node/edge/route structure, then `bind_capabilities` resolves the
blueprint's referenced model and tools against a `CapabilityResolver`.

**Recursive concept:** *programs as runtime values.* A declarative `.rag`
workflow lowers — lexer → parser → compiler — into the same graph/harness runtime
as hand-written Rust. This is the human-authored half of self-authoring, and the
safe boundary the next example lets a model write into.

## 11. `openai_self_blueprint` (needs `OPENAI_API_KEY`)

```sh
cargo run --example openai_self_blueprint
```

The deepest recursion: **the agent authors its own graph.** OpenAI is asked to
emit *only* `.rag` source for a small agent graph (given the grammar plus a
worked example), the `.rag` text is extracted from the reply, then run through the
*same safe pipeline* as a human-authored blueprint: `parse_str` -> `compile` ->
print the `Blueprint` -> `bind_capabilities` against a `CapabilityResolver`
allowlist (the policy gate — only allowlisted models/tools pass) -> `build_graph`
with a `NodeFactory` -> run to `END`. The model never executes code; it only
produces declarative source that a Rust-side factory materializes, and the
capability allowlist is the safety boundary. Parse/compile failures print the
diagnostic and the offending source instead of panicking.

**Recursive concept:** *self-authoring* — a model emits a blueprint that compiles
through the same registry-bound compiler path and runs on the same runtime the
model is already executing in. The harness describes and re-enters itself, with
the capability allowlist as the policy gate.

## 12. `subconscious_loop` (offline)

```sh
cargo run --example subconscious_loop
```

A multi-file example (under `examples/subconscious_loop/`) that maps a
LangGraph-style **autonomous closed-loop harness with a dedicated subconscious
layer** onto the typed graph runtime, kept fully deterministic so it runs offline
and under `cargo test`. The graph models three cognitive tiers — a *quick* layer
(`frontend_agent` turns channel payloads into macro instructions, then compiles
the final response), a *reasoning* layer (`agent_execution` does mock retrieval
from long-term memory, simulates sub-agent execution, extracts semantic traces,
emits a sequential world-state diff, and decides whether to escalate), and a
*subconscious* layer (`subconscious_eval` consumes gated world summaries and
emits a short steering directive, then resets the trigger). A
`summarization_gate` compresses diffs and a `context_manager_hook` evicts
semantic history into a mock vector store when `context_utilization` crosses a
threshold. Nodes return a `StatePatch` merged through a `ClosureStateReducer`
rather than overwriting state. The matching integration tests live in
`tests/e2e_subconscious_loop_example.rs`.

**Recursive concept:** *a runtime that steers itself.* A bounded loop feeds its
own compressed world-state back into a subconscious node that emits steering for
the next cycle — recursion as a self-regulating control loop over durable graph
state, all without a network call.

---

## 13. `goals_and_todos` (offline)

```sh
cargo run --example goals_and_todos
```

Wires the two per-thread productivity primitives — a durable **goal**
(`graph::goals`) and a kanban **task board** (`graph::todos`) — together on one
thread and one shared `InMemoryStore`, and lets the goal *drive* the board. A
`ThreadGoal` ("Ship the v2 release") is the completion contract with a token
budget; a `TaskBoard` holds three cards; a `goal_gate_node` forms a self-driving
loop where each iteration advances the board by one kanban transition (Todo →
InProgress → Done) and completes the goal once every card is done. The gate keeps
looping while the goal is Active and under budget, accounting each iteration's
token usage, and routes to `END` when the goal completes. Prints the board
markdown and final goal status.

**Recursive concept:** *intent that carries itself forward.* Durable per-thread
state (the goal) steers a bounded graph loop that mutates other durable state
(the board), with the graph's `recursion_limit` and the goal's budget as the two
stop conditions — continuation without a heartbeat. See
[Goals and Todos](Goals-and-Todos.md) for the full design.

---

## Recently added capabilities

Copy-pasteable snippets for the newest harness/graph/registry surfaces; each
mirrors its cited test.

### Reject retired models in resolution

Resolution normally skips `ModelStatus::Retired` models; set `allow_retired` to
opt a retired model back in. *(See `src/harness/model/test.rs`
`registry_skips_retired_models_across_every_resolution_path`.)*

```rust
use tinyagents::harness::model::{ModelRegistry, ModelSelection};

// Without allow_retired this returns None; with it the retired model resolves.
let binding = registry.resolve(ModelSelection {
    requested: Some("gpt-legacy".into()),
    allow_retired: true,
    ..ModelSelection::default()
});
```

Emits no events; unresolved selections simply return `None`.

### Recover from unknown tool calls

Turn a model's call to an unregistered tool into a recoverable tool-error
message instead of aborting the run. *(See
`tests/e2e_unknown_tool_policy.rs::return_tool_error_preserves_original_arguments`.)*

```rust
use tinyagents::harness::runtime::{RunPolicy, UnknownToolPolicy};

harness.with_policy(RunPolicy {
    unknown_tool: UnknownToolPolicy::ReturnToolError,
    ..RunPolicy::default()
});
// The next run injects an "unknown tool `..`" message and keeps going.
```

Emits `AgentEvent::UnknownToolCall { requested_name, arguments, recovery, .. }`
(here `recovery == "tool_error"`, with the original arguments preserved).

### Workspace isolation + path enforcement

Prepare an isolated workspace, enforce that every tool path stays inside it, then
clean up — threading the descriptor into a run so tools inherit the sandbox.
*(See `src/harness/workspace/test.rs`.)*

```rust
use std::path::Path;
use tinyagents::harness::context::{RunConfig, RunContext};
use tinyagents::harness::events::EventSink;
use tinyagents::harness::workspace::{
    SharedRootWorkspace, cleanup_workspace, prepare_workspace,
};

let events = EventSink::new();
let provider = SharedRootWorkspace::new("/work");
let ws = prepare_workspace(&provider, &events, "run-7", Some("worker")).await?;
ws.enforce(Path::new("/work/out.txt"), &events)?; // Err for paths outside the root
let ctx = RunContext::new(RunConfig::new("run-7"), ()).with_workspace(ws.clone());
cleanup_workspace(&provider, &events, &ws).await?;
```

Emits `workspace.prepared` and `workspace.cleanup`; a blocked path emits
`workspace.violation` and fails closed.

### Enforce tool policy

Gate tool exposure and execution: sandboxed tools require a sandboxed workspace,
approval-gated tools require an explicit grant, and oversized results are
truncated. *(See `src/harness/middleware/library/test.rs`
`tool_policy_requires_sandbox_for_sandboxed_tool`,
`tool_policy_truncates_oversized_results`.)*

```rust
use tinyagents::harness::middleware::ToolPolicyMiddleware;

let mw = ToolPolicyMiddleware::new(policies) // HashMap<String, ToolPolicy>
    .require_sandbox(true)
    .require_approval(["deploy"])
    .enforce_result_bytes(true);
```

No dedicated event: blocked calls surface as `TinyAgentsError::Validation`, and
oversized results are truncated in place with a `max_result_bytes` note attached.

### Audit + inherit tool exposure

`inheriting` composes a parent allow/deny with a child's so a sub-agent can only
narrow, never widen, the exposed tools. *(See
`src/harness/middleware/library/test.rs`
`contextual_selection_inheriting_narrows_never_widens`,
`contextual_selection_emits_exposure_event`.)*

```rust
use tinyagents::harness::middleware::ContextualToolSelectionMiddleware;

// parent allow ∩ child allow, then union of both deny lists.
let mw = ContextualToolSelectionMiddleware::inheriting(
    Some(["a", "b", "c"]), // parent allow
    ["c"],                  // parent deny
    Some(["b", "c", "d"]),  // child allow
    Vec::<String>::new(),   // child deny
);
```

Emits `AgentEvent::ToolsFiltered { excluded, remaining, .. }` before the model
call.

### Middleware control + precedence

Request a control outcome from middleware; the loop honors it at the checkpoint
after the model call, keeping the highest-precedence request. *(See
`tests/e2e_control_and_steer.rs`, `src/harness/context/test.rs`
`request_control_keeps_highest_precedence`.)*

```rust
use tinyagents::harness::context::MiddlewareControl;

// From inside a middleware's after_model, on `ctx: &mut RunContext`:
ctx.request_control(MiddlewareControl::StopWithFinal("stopped".into()));
// A stronger Interrupt would not be downgraded by a later StopWithFinal.
```

`StopWithFinal` ends the run (no more tools) and still emits `run.completed`
plus `control.applied`; `Interrupt` surfaces as `TinyAgentsError::Interrupted`.

### Budget with reservation + cached-input

Preflight reserves against estimated input tokens (blocking oversized calls) and
reconciles against actual usage; a separate cap bounds cache-read tokens. *(See
`src/harness/middleware/library/test.rs`
`budget_preflight_reserves_and_reconciles`,
`budget_enforces_cached_input_token_limit`.)*

```rust
use tinyagents::harness::middleware::{BudgetLimits, BudgetMiddleware};

let mw = BudgetMiddleware::new(BudgetLimits {
    max_input_tokens: Some(5),
    max_cached_input_tokens: Some(10),
    ..BudgetLimits::default()
});
```

Emits `AgentEvent::BudgetReserved` on preflight and `BudgetReconciled { actual_input_tokens, .. }`
after the call; exhaustion emits `BudgetExceeded { blocked: true, .. }` and errors
with `TinyAgentsError::LimitExceeded`.

### Stable event ids

Seed a sink with a stream id so `(stream_id, offset)` re-mints identical event
ids across restarts/replays. *(See `src/harness/events/test.rs`
`stream_id_prefix_makes_event_ids_stable_and_collision_free`.)*

```rust
use tinyagents::harness::events::EventSink;

let sink = EventSink::with_stream_id("run-42");
let record = sink.emit(tinyagents::harness::events::AgentEvent::StateUpdate);
assert_eq!(record.id.as_str(), "run-42-evt-0"); // stable across restart
```

Ids are `"<stream_id>-evt-<offset>"`; default sinks get process-unique prefixes.

### Parallel map/reduce with timeouts + cancellation

Run a fallible async closure over items with bounded concurrency, per-item and
total timeouts, and a cancellation token — results stay in input order. *(See
`src/graph/parallel/test.rs`.)*

```rust
use std::time::Duration;
use tinyagents::harness::cancel::CancellationToken;
use tinyagents::{ParallelOptions, map_reduce};

let token = CancellationToken::new();
let out = map_reduce(
    vec![1u64, 2, 3],
    ParallelOptions::default()
        .with_item_timeout(Duration::from_millis(50))
        .with_total_timeout(Duration::from_secs(5))
        .with_cancellation(token),
    |_index, n| async move { Ok::<_, tinyagents::TinyAgentsError>(n * 10) },
)
.await?;
let values: Vec<u64> = out.into_successes();
```

No events; a total timeout yields `TinyAgentsError::Timeout`, a cancelled token `TinyAgentsError::Cancelled`.

### Orchestrate list with kind + created window

The `orchestrate_list` tool filters spawned tasks by `kind` and a creation-time
window. *(See `src/graph/orchestration/test.rs`
`list_tool_honors_created_window_and_kind`.)*

```rust
use serde_json::json;
use tinyagents::graph::orchestration::{OrchestrationTool, OrchestrationToolKind};
use tinyagents::harness::tool::{Tool, ToolCall};

let list = OrchestrationTool::new(OrchestrationToolKind::List, store);
let result = list
    .call(&(), ToolCall::new("l1", "orchestrate_list",
        json!({ "kind": "sub_agent", "created_after_ms": 0 })))
    .await?;
// result.raw is a JSON array of matching task snapshots.
```

No events; the tool result's `raw` holds the filtered task list.

### Registry introspection

Snapshot the registry (components + aliases) for audit/UI, and run integrity
diagnostics. *(See `src/registry/capability/test.rs` `snapshot_enumerates_aliases`,
`diagnostics_flag_name_reused_across_kinds`.)*

```rust
use tinyagents::registry::ComponentKind;

let snapshot = registry.snapshot();
for a in &snapshot.aliases {
    println!("{:?} {} -> {}", a.kind, a.alias, a.canonical);
}
let components = snapshot.by_kind(ComponentKind::Model);
let diagnostics = registry.diagnostics(); // e.g. a name reused across kinds
```

No events; `snapshot` serializes for audit logs and `diagnostics` returns any
integrity warnings.

### Storage conformance helpers

Reusable contract suites prove every `TaskStore`/`Checkpointer` backend behaves
interchangeably, including concurrent access and replay-after-restart. *(See
`tests/conformance.rs`.)*

```rust
use tinyagents::graph::orchestration::{InMemoryTaskStore, JsonlTaskStore};
use tinyagents::graph::testkit::conformance::{
    checkpointer_concurrent_contract, taskstore_concurrent_contract, taskstore_replay_contract,
};

taskstore_concurrent_contract(std::sync::Arc::new(InMemoryTaskStore::new()));
taskstore_replay_contract(|| JsonlTaskStore::open(&path).unwrap());
```

No events; each helper panics on a contract violation, so a passing call is the assertion.

---

## Where to go next

- [Harness](Harness.md) — the agent loop, sub-agents, steering, and the surfaces
  these examples drive.
- [Graph Runtime](Graph-Runtime.md) — subgraphs, reducers, and checkpointing.
- [Providers](Providers.md) — configuring `OpenAiModel` and compatible hosts.
