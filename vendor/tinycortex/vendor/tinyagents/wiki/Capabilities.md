# Capabilities

This is the **discovery index** for TinyAgents: one page where an agent (or a
human) can see every functionality the crate exposes and jump straight to the
deep page for it. Each row is a capability, a one-line description of what it
does, and a **Where** link to the wiki page (and section anchor) that documents
it. Start here, find the surface you need, and branch out.

The crate is organized into five surfaces — [Harness](Harness),
[Graph runtime](Graph-Runtime), [Registry](Registry),
[Expressive `.rag`](Expressive-Language-RAG), and [REPL `.ragsh`](REPL-Language-RAGSH) —
plus [Providers](Providers) at the leaves and testkit/conformance for tests.

## Harness — model calls, tools, control

| Capability | What it does | Where |
| --- | --- | --- |
| Agent loop | Default model→tool→model loop with run identity + limits | [Harness](Harness#the-agent-loop-agent_loop) |
| Provider-neutral model calls | One `ChatModel` trait; `ModelRequest`/`ModelResponse` across hosts | [Harness](Harness#provider-neutral-model-calls-model-providers) |
| Model resolution & selection | `ModelRegistry` resolves a call via `ModelSelection`/`ModelHint`/capabilities | [Harness](Harness#provider-neutral-model-calls-model-providers) |
| Typed tools | `Tool` trait with JSON schemas and a `ToolRegistry` | [Harness](Harness#typed-tools-tool) |
| Middleware hooks | before/after hooks around agent, model, and tool calls | [Harness](Harness#middleware-hooks-middleware) |
| Structured output | Extract a typed value via a provider-appropriate strategy | [Harness](Harness#structured-output-structured) |
| Streaming | Model-delta and event streaming through the loop | [Harness](Harness#streaming-stream-model-streaming) |
| Usage & cost | Token accounting and pricing roll-ups across the run tree | [Harness](Harness#usage-and-cost-usage-cost) |
| Limits, retry, fallback | Fail-closed caps, backoff, model fallback, rate limiting | [Harness](Harness#limits-retry-fallback-rate-limiting-limits-retry) |
| Response & prompt cache | Local response cache + provider prompt/KV-cache layout | [Harness](Harness#cache-cache) |
| Memory & embeddings | Thread memory, vector stores, retrievers | [Harness](Harness#memory-embeddings-retrieval-memory-embeddings) |
| Sub-agents (agents as tools) | Wrap an agent as a tool; child runs one level deeper | [Harness](Harness#sub-agents-agents-as-tools-subagent) |
| Steering | Typed runtime control (pause/resume/redirect) of a running agent | [Harness](Harness#steering-steering) |
| Summarization | Context-window-aware transcript compaction | [Harness](Harness#summarization-context-window-aware-summarization) |
| Events & run status | In-memory event spine + `HarnessRunStatus` | [Harness](Harness#events-and-run-status-events) |
| Durable observability | Journals, status stores, sinks, latency metrics | [Harness](Harness#durable-observability-observability) |
| Testkit | Fakes, recorders, trajectory assertions | [Harness](Harness#testkit-testkit) |

## Graph runtime — durable typed workflows

| Capability | What it does | Where |
| --- | --- | --- |
| Nodes & edges | Named nodes wired by static/conditional edges over typed state | [Graph Runtime](Graph-Runtime#building-a-graph-graphbuilderstate-update) |
| Dynamic routing | `Command.goto` + routing precedence resolve next targets | [Graph Runtime](Graph-Runtime#routing-precedence) |
| `Send` fan-out (map-reduce) | Schedule nodes with per-invocation args for the map step | [Graph Runtime](Graph-Runtime#node-results-updates-commands-interrupts) |
| Reducers & channels | Merge branch updates deterministically; channel-per-field state | [Graph Runtime](Graph-Runtime#typed-state-reducers-and-channels) |
| Parallel fan-out | Run multi-node supersteps concurrently, folded in index order | [Graph Runtime](Graph-Runtime#parallel-fan-out) |
| Checkpoints & durability | Persist at superstep boundaries; in-memory/file/SQLite backends | [Graph Runtime](Graph-Runtime#checkpoints-durability-and-time-travel-graphcheckpoint) |
| Time travel | Read/fork/update state history against a checkpointer | [Graph Runtime](Graph-Runtime#the-superstep-executor-graphcompiled) |
| Interrupts & resume | Human-in-the-loop pause and resume with a payload | [Graph Runtime](Graph-Runtime#interrupts-and-resume) |
| Subgraphs | Embed a compiled graph as a node (graph-runs-graph) | [Graph Runtime](Graph-Runtime#subgraphs-graph-level-recursion-graphsubgraph) |
| Sub-agent nodes | Embed a harness agent as a graph node | [Graph Runtime](Graph-Runtime#sub-agent-nodes-graphsubagent_node) |
| Recursion policy | Bound depth, per-node visits, and total steps | [Graph Runtime](Graph-Runtime#recursion-policy-and-the-run-tree-graphrecursion) |
| Orchestration tools | Model-callable child-work supervision (`orchestrate_*`) | [Graph Runtime](Graph-Runtime#orchestration-tools-graphorchestration) |
| Streaming & events | `GraphEvent`s + `StreamMode` projections | [Graph Runtime](Graph-Runtime#streaming-and-events-graphstream) |
| Durable graph observability | Journals, status store, latency metrics | [Graph Runtime](Graph-Runtime#durable-observability-graphobservability) |
| Topology export | JSON/Mermaid export of a graph's structure | [Graph Runtime](Graph-Runtime#topology-export-and-visualization-graphexport) |
| Graph testkit | Deterministic node doubles + graph assertions | [Graph Runtime](Graph-Runtime#testkit-graphtestkit) |

## Registry — named capability catalog

| Capability | What it does | Where |
| --- | --- | --- |
| Component identity | `ComponentId`/`ComponentKind`/`ComponentMetadata` describe capabilities | [Registry](Registry#component-identity-registrycomponent) |
| Capability registry | Register/resolve models, tools, graphs, agents by name | [Registry](Registry#the-capability-registry-registrycapability) |
| Bind `.rag`/`.ragsh` by name | Allow-list validation of agent-authored source before it runs | [Registry](Registry#binding-rag--ragsh-capabilities-by-name) |
| Model catalog | Offline pricing, context windows, capability facts | [Registry](Registry#the-model-catalog-registrycatalog) |

## Language & REPL — authored and interactive workflows

| Capability | What it does | Where |
| --- | --- | --- |
| Expressive `.rag` | Declarative, side-effect-free blueprints that compile to the runtime | [Expressive Language (.rag)](Expressive-Language-RAG) |
| REPL `.ragsh` | Imperative, capability-bound interactive orchestration (RLM loop) | [REPL Language (.ragsh)](REPL-Language-RAGSH) |

## Providers — model backends at the leaves

| Capability | What it does | Where |
| --- | --- | --- |
| Offline mock model | Deterministic, network-free default build | [Providers](Providers#offline-by-default-providers-compiled-in) |
| OpenAI + compatible hosts | One adapter for OpenAI, Anthropic, Ollama, DeepSeek, Groq, xAI, OpenRouter, Together, Mistral | [Providers](Providers#provider-kinds-and-specs) |
| List available models | `OpenAiModel::list_models()` — runtime model discovery via `GET /models` | [Providers](Providers#4-discover-available-models) |
| Provider inference | Resolve a provider from a model string | [Providers](Providers#provider-selection-from-a-model-string) |
| Capability profiles | Reject impossible requests; pick capability-satisfying fallbacks | [Providers](Providers#capability-profiles-and-the-model-registry) |

## Testing — testkit & conformance

| Capability | What it does | Where |
| --- | --- | --- |
| Harness testkit | Scripted models, fake tools, trajectory asserts | [Harness](Harness#testkit-testkit) |
| Graph testkit | Node doubles + fluent graph assertions | [Graph Runtime](Graph-Runtime#testkit-graphtestkit) |
| Storage conformance | Reusable contracts for task stores and checkpointers | [Graph Runtime](Graph-Runtime#testkit-graphtestkit) |

## Recently added (SDK-gap hardening)

New capabilities that harden the SDK surface. Deep docs live on the linked pages.

| Capability | What it does | Where |
| --- | --- | --- |
| Model lifecycle gating | `ModelRegistry::resolve` skips `ModelStatus::Retired`; `ModelSelection.allow_retired = true` opts back in | [Harness](Harness#provider-neutral-model-calls-model-providers) |
| `orchestrate_list` filters | Filter managed tasks by `kind`, `created_after_ms`, `created_before_ms` | [Graph Runtime](Graph-Runtime#orchestration-tools-graphorchestration) |
| Unknown-tool recovery | `UnknownToolCall` carries original `arguments`; `UnknownToolPolicy{Fail,ReturnToolError,Rewrite}` | [Harness](Harness#typed-tools-tool) |
| Stable event ids | `EventSink::with_stream_id`; ids `{stream_id}-evt-{offset}` survive restarts | [Harness](Harness#durable-observability-observability) |
| Workspace isolation | `RunContext::with_workspace`, `prepare/cleanup_workspace`, `WorkspaceDescriptor::enforce`, `AgentEvent::Workspace*` | [Harness](Harness#the-agent-loop-agent_loop) |
| Tool exposure auditing | `AgentEvent::ToolsFiltered`; `ContextualToolSelectionMiddleware::inheriting(...)` | [Harness](Harness#middleware-hooks-middleware) |
| Control outcomes | `AgentEvent::ControlApplied`, `MiddlewareControl::{kind,precedence}`, precedence-based `request_control` | [Harness](Harness#middleware-hooks-middleware) |
| Tool policy enforcement | `ToolPolicyMiddleware::{require_sandbox, require_approval, enforce_result_bytes}` | [Harness](Harness#middleware-hooks-middleware) |
| Model delta attribution | `AgentEvent::ModelDelta` now carries `run_id` | [Harness](Harness#streaming-stream-model-streaming) |
| Thinking-token streaming | `MessageDelta.reasoning`, `ModelDelta.reasoning`, and `Usage.reasoning_tokens` keep thoughts separate from visible text | [Harness](Harness#streaming-stream-model-streaming) |
| Budget reservation & cost | `BudgetLimits.max_cached_input_tokens`, `AgentEvent::{BudgetReserved,BudgetReconciled}`, preflight reservation | [Harness](Harness#usage-and-cost-usage-cost) |
| Parallel map/reduce controls | `ParallelOptions::{with_item_timeout, with_total_timeout, with_cancellation}` | [Graph Runtime](Graph-Runtime#parallel-fan-out) |
| Registry introspection | `ComponentKind` adds Middleware/Checkpointer/TaskStore/Listener; `RegistrySnapshot.aliases`; name-reuse diagnostic | [Registry](Registry#the-capability-registry-registrycapability) |
| Storage conformance | `graph::testkit::conformance::{taskstore_concurrent_contract, checkpointer_concurrent_contract, taskstore_replay_contract}` | [Graph Runtime](Graph-Runtime#testkit-graphtestkit) |

## Agent quick-start: pick your task

- **Want to enforce tool safety (sandbox/approval/output caps)?** →
  [Tool policy enforcement](Harness#middleware-hooks-middleware)
- **Need a token/cost budget with preflight reservation?** →
  [Budget reservation & cost](Harness#usage-and-cost-usage-cost)
- **Got a model call for an unregistered tool?** →
  [Unknown-tool recovery](Harness#typed-tools-tool)
- **Want to stop calling a deprecated model?** →
  [Model lifecycle gating](Harness#provider-neutral-model-calls-model-providers)
- **Running agents in parallel with timeouts/cancellation?** →
  [Parallel map/reduce controls](Graph-Runtime#parallel-fan-out)
- **Need to isolate a run to a workspace?** →
  [Workspace isolation](Harness#the-agent-loop-agent_loop)
- **Want stable, replayable event ids?** →
  [Stable event ids](Harness#durable-observability-observability)
- **Supervising child work from a model?** →
  [Orchestration tools](Graph-Runtime#orchestration-tools-graphorchestration)
- **Pausing for human input mid-graph?** →
  [Interrupts and resume](Graph-Runtime#interrupts-and-resume)
- **Letting a model author its own workflow?** →
  [Bind `.rag`/`.ragsh` by name](Registry#binding-rag--ragsh-capabilities-by-name)
- **Verifying a run's shape in a test?** →
  [Harness testkit](Harness#testkit-testkit) /
  [Graph testkit](Graph-Runtime#testkit-graphtestkit)

## See also

- [Home](Home) — orientation and the five-surface overview.
- [Examples](Examples) — runnable end-to-end demonstrations.
- [Architecture](Architecture) — how the surfaces compose.
