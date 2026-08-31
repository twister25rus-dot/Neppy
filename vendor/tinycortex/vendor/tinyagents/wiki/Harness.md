# Harness

The **harness** (`src/harness/`) is the orchestration layer around language-model
calls — and, in TinyAgents' recursive-language-model (RLM) framing, it is the
*substrate that makes recursion observable*. Every model call, tool call,
sub-agent run, and graph node ultimately bottoms out in a harness agent loop, so
the harness is where parent/child run identity, usage roll-ups, depth limits, and
event streams are tracked. When an agent calls another agent (a model calling a
model), it is one harness invoking another harness one level deeper in the
recursion tree.

This page is a developer deep-dive. Each section is grounded in real module and
type names under `src/harness/` and links to the matching design note under
[`docs/modules/harness/`](../docs/modules/harness/README.md).

> **Background on the RLM lineage** — see the
> [Recursive Language Models](https://alexzhang13.github.io/blog/2025/rlm/) blog
> and paper (Zhang, Kraska, Khattab, MIT CSAIL, 2025,
> [arXiv:2512.24601](https://arxiv.org/abs/2512.24601)). TinyAgents is
> *inspired by and architected around* that execution model — sub-model /
> sub-agent / sub-graph calls as functions, persistent session values, depth
> tracking, and trajectory logging — not a reimplementation of the paper.

## Module map

| Module (`src/harness/…`) | Role | Design note |
| --- | --- | --- |
| `agent_loop` | Default model→tool→model loop | — |
| `runtime` | `AgentHarness` facade + `RunPolicy` | — |
| `context` | `RunConfig`, `RunContext`, depth/limit tracking | [context.md](../docs/modules/harness/context.md) |
| `message` | Typed `Message` / `ContentBlock` chat shapes | — |
| `ids` | Typed `RunId`/`ThreadId`/`CallId`/… identifiers | — |
| `prompt` | Prompt templates + cache-segmented `PromptBuilder` | [prompt.md](../docs/modules/harness/prompt.md) |
| `model` | Provider-neutral `ChatModel`, requests, responses, streams | [model.md](../docs/modules/harness/model.md) |
| `providers` | Feature-gated provider adapters (`MockModel`, `OpenAiModel`) | — |
| `tool` | Typed tool trait, schemas, registry | [tool.md](../docs/modules/harness/tool.md) |
| `middleware` | before/after hooks around agent, model, tool | [middleware.md](../docs/modules/harness/middleware.md) |
| `structured` | Typed/JSON-schema response extraction | [structured-output.md](../docs/modules/harness/structured-output.md) |
| `stream` / model streaming | Token & event streaming | [streaming.md](../docs/modules/harness/streaming.md) |
| `usage` | Token accounting | [usage.md](../docs/modules/harness/usage.md) |
| `cost` | Pricing + cost roll-ups | [cost.md](../docs/modules/harness/cost.md) |
| `limits` / `retry` | Caps, timeouts, backoff, fallback, rate limit | [limits-retry.md](../docs/modules/harness/limits-retry.md) |
| `cache` | Local response cache + prompt-cache layout | [cache.md](../docs/modules/harness/cache.md) |
| `memory` | Short-term thread memory / chat history | — |
| `embeddings` | Embedding models, vector stores, retrievers | [embeddings.md](../docs/modules/harness/embeddings.md) |
| `store` | Pluggable persistence backends | [store.md](../docs/modules/harness/store.md) |
| `events` | Typed event stream + run status | [observability.md](../docs/modules/harness/observability.md) |
| `observability` | Durable journals, status stores, sinks, latency metrics | [observability.md](../docs/modules/harness/observability.md) |
| `subagent` | Agents-as-tools, reusable sessions | [subagent-steering.md](../docs/modules/harness/subagent-steering.md) |
| `steering` | Typed runtime control of running agents | [subagent-steering.md](../docs/modules/harness/subagent-steering.md) |
| `summarization` | Context-window-aware compaction | [summarization.md](../docs/modules/harness/summarization.md) |
| `cancel` | Cooperative `CancellationToken` | — |
| `testkit` | Fakes, recorders, trajectory asserts | [testkit.md](../docs/modules/harness/testkit.md) |

The harness deliberately does **not** depend on the graph module: you can call a
model or run a tool loop without constructing a graph. The graph runtime depends
on harness traits, not the other way around.

## The agent loop (`agent_loop`)

The default loop is implemented as inherent methods on
`AgentHarness<State, Ctx>` (`src/harness/agent_loop/mod.rs`). The canonical entry
points are:

- `invoke(state, ctx_data, config, input)` — full control over run identity and
  context data.
- `invoke_default(state, input)` — convenience wrapper that builds a default
  `RunConfig`.
- `invoke_in_context(state, ctx, input)` — run *inside an existing*
  `RunContext`, which is how nested/child runs inherit parent identity.
- `invoke_streaming*` variants — same loop, emitting `ModelStreamItem`s.
- `*_with_status` variants — return the `AgentRun` plus a `HarnessRunStatus`.

The lifecycle (model→tool→model) is:

```text
input messages
  -> build RunContext, emit RunStarted
  -> before_agent middleware
  -> loop:
       enforce model-call cap + wall-clock deadline (fail-closed)
       build ModelRequest (messages + tool schemas + default response format)
       before_model middleware; emit ModelStarted
       resolve + invoke model (with retry + fallback)
       after_model middleware; emit ModelCompleted; fold Usage into AgentRun
       append assistant message
       if tool calls -> enforce tool cap, before_tool, run tools,
                        after_tool, append tool results, continue
       else -> extract structured output (if configured) and break
  -> after_agent middleware; emit RunCompleted
```

On error the loop emits `RunFailed`, fans the error through `on_error`
middleware, and returns it. The loop is intentionally **sleep-free**: retry
backoff durations are *computed* via `RetryPolicy::backoff_for_attempt` but the
loop itself does not block, keeping tests deterministic.

The accumulated result is `AgentRun` (`harness::middleware::AgentRun`), holding
the final messages, folded `Usage`, and any extracted structured response.

## Provider-neutral model calls (`model`, `providers`)

All model invocation goes through one trait (`harness::model::ChatModel<State>`)
with `invoke` (unary) and `stream` (incremental) methods. A call is described by
`ModelRequest` (built fluently with `ModelRequest::new(messages).with_tools(…)
.with_tool_choice(…).with_response_format(…).with_model_hint(…)
.with_required_capabilities(…)`) and answered by `ModelResponse`, which carries
the assistant message, `Usage`, finish reason, and a `ResolvedModel` recording
*which* concrete provider/model actually served the call.

Model selection is explicit and reusable. `ModelRegistry` resolves a call through
`ModelSelection` / `ModelHint` against a `CapabilitySet` (see `ModelProfile`,
`Modalities`, `ModelStatus`), and records the choice as a `ResolvedModel` with a
`ModelResolutionSource`. This lets a parent and its sub-agents reason about model
identity consistently across the recursion tree.

**Model lifecycle gating.** `ModelRegistry::resolve` skips any candidate whose
profile reports `ModelStatus::Retired` across *every* resolution path (override,
state reuse, hint, agent default, registry default), so a provider-retired model
is never selected by accident — a higher-priority retired hint is passed over for
a live one. `ModelProfile::is_usable()` is `true` for any status except
`Retired`; `is_deprecated()` covers both `Deprecated` and `Retired` (deprecated
models stay selectable but flagged). Opt back in for historical replay with
`ModelSelection { allow_retired: true, .. }`. See
[model.md](../docs/modules/harness/model.md).

`providers` holds the adapters:

- `MockModel` / the `providers` test models (`echo`, `constant`, scripted
  responses) — the default **offline** build.
- `OpenAiModel` (behind the `openai` Cargo feature). Despite the name it speaks
  the OpenAI Chat Completions wire format to *many* hosts via `ProviderSpec` /
  `ProviderKind`: constructors include `deepseek`, `anthropic`, `groq`, `xai`,
  `openrouter`, `together`, `mistral`, `ollama`, and `compatible(...)` for any
  OpenAI-compatible endpoint, plus `from_env` / `from_spec_env`.

## Typed tools (`tool`)

Tools implement `harness::tool::Tool<State>` with `name`, `description`,
`schema() -> ToolSchema`, and an async `call(state, ctx, call) -> Result<ToolResult>`.
`ToolCall` carries an id, name, and JSON `arguments`; `ToolResult` is built with
`ToolResult::text(...)` / `ToolResult::error(...)` and exposes `is_error()`.
`ToolRegistry` keys tools by name and is consulted by the loop when an assistant
message requests a tool. `ToolDelta` carries incremental tool progress for
streaming.

Crucially, a **whole agent can be a tool** — see [Sub-agents](#sub-agents-agents-as-tools-subagent).

## Middleware hooks (`middleware`)

`harness::middleware::Middleware<State, Ctx>` exposes the cross-cutting hooks the
loop fans out to: `before_agent` / `after_agent`, `before_model` /
`on_model_delta` / `after_model`, `before_tool` / `on_tool_delta` /
`after_tool`, and `on_error`. They are ordered through a `MiddlewareStack`
(`before_*` in registration order, `after_*` in reverse). `HookCounts` records
how often each hook fired. Built-in middleware lives here too:
`LoggingMiddleware`, `MessageTrimMiddleware`, `ContextCompressionMiddleware`,
`PromptCacheGuardMiddleware`, and `UsageAccountingMiddleware`.

## Structured output (`structured`)

`StructuredExtractor` extracts a typed value from the final `ModelResponse`
according to a `StructuredStrategy` (`StructuredStrategy::for_profile(...)`
chooses a provider-appropriate strategy). `response_format_for_strategy(...)`
maps a strategy onto a `ResponseFormat` (`Text`, `json_schema`, provider-native,
or a tool-call strategy). The parsed value lands in `StructuredOutput`, which
exposes `as_value()` and `parse::<T>()`. Set
`RunPolicy::default_response_format` to attach a schema to every model request in
a run; the loop runs extraction automatically before completing.

## Streaming (`stream`, model streaming)

Two layers cooperate. At the provider edge, `ChatModel::stream` yields a
`ModelStream` of `ModelStreamItem`s; `StreamAccumulator` (or the
`collect_model_stream` helper) folds them back into a `ModelResponse`. At the
harness edge, `harness::stream` exposes a `StreamSink` over `StreamChunk`s with
selectable `StreamMode`s (messages, tools, updates, events, final). The
`invoke_streaming*` agent-loop methods surface model deltas as the loop runs.

Thinking output is a first-class side channel. `MessageDelta` carries visible
`text` separately from `reasoning`, and `ModelDelta.reasoning` forwards the same
field through middleware. Provider adapters map exposed thinking fragments
(for example OpenAI-compatible `reasoning_content` / `reasoning` stream fields)
there, so UIs can render thoughts without merging them into final assistant
text.

## Usage and cost (`usage`, `cost`)

`Usage` records `input_tokens` / `output_tokens`, cached-token bookkeeping,
`reasoning_tokens` when the provider reports them, and `effective_total()`. The
loop folds each call's `Usage` into the `AgentRun`, and `UsageTotals` aggregates
across calls. `cost::estimate_cost(pricing, usage)` turns a `ModelPricing` +
`Usage` into `CostTotals`. **This is where recursion becomes measurable:**
because a child run executes through the parent's `RunContext`/event sink, child
usage and cost roll up into the parent's totals, so an orchestrator can see the
full cost of the subtree it spawned.

## Limits, retry, fallback, rate limiting (`limits`, `retry`)

`RunLimits` (built via `with_max_model_calls`, `with_max_tool_calls`,
`with_max_wall_clock_ms`, `with_max_retries_per_call`, `with_max_concurrency`,
and **`with_max_depth`**) is enforced fail-closed by a `LimitTracker` and by the
loop itself. Reaching a cap returns `TinyAgentsError::LimitExceeded`; the
deadline returns `TinyAgentsError::Timeout`.

`max_depth` is the **recursion limit**: it bounds how deep nested sub-agents (or
subgraphs) may recurse. An invocation whose child depth would exceed the cap
fails with `TinyAgentsError::SubAgentDepth`.

`retry` provides `RetryPolicy` (attempts, backoff, multiplier, jitter), the
`is_retryable(err)` classifier (only network/timeout/rate-limit/5xx by default),
`FallbackPolicy` (an ordered model fallback chain — `next_after(current)`), and a
token-bucket `RateLimiter`.

## Cache (`cache`)

`cache` separates two distinct ideas:

- **Local response cache** — `ResponseCache` (with `InMemoryResponseCache`),
  keyed by `cache_key(request)`. Attach it with
  `AgentHarness::with_response_cache`; the loop then checks it before each
  provider call and stores successful responses, emitting `cache.hit` /
  `cache.miss` events. Because the cache is owned by the harness, a repeated
  identical request — even across separate runs — can be served from an earlier
  result.
- **Provider prompt/KV-cache layout** — `PromptCacheLayout` (and
  `CacheLayoutEvent`) make the stable prompt prefix explicit so middleware that
  edits model-visible prompt segments can be detected
  (`is_prefix_stable_against`). `CachePolicy` gates both behaviors.
  `thread_id` should be propagated through parent agents, sub-agents, subgraphs,
  and nested harness calls so provider caches see one stable logical
  conversation. Provider adapters may map that stable identity into required
  cache/user headers when a backend, such as Fireworks-style prefix caching,
  requires explicit cache attribution.

## Memory, embeddings, retrieval (`memory`, `embeddings`)

`memory` owns conversation continuity: the `ChatHistory` trait
(`InMemoryChatHistory`, `StoreChatHistory<S>`) and `ShortTermMemory<H>`, which is
loaded before a loop and saved after, optionally trimmed. `MemoryScope`
distinguishes short- vs long-term data.

`embeddings` provides retrieval-augmented context: the `EmbeddingModel` trait
(`MockEmbeddingModel`; `OpenAiEmbeddingModel` behind the feature), a `VectorStore`
trait (`InMemoryVectorStore`), `cosine_similarity`, and a `Retriever` whose
`index(docs)` / `retrieve(query, top_k) -> Vec<ScoredDoc>` close the loop. In the
RLM framing, retrieval is how an agent pulls *snippets* of a large external
environment into context instead of stuffing the whole thing into one window.

## Sub-agents (agents as tools) (`subagent`)

This is the core recursive surface. A `SubAgent<State, Ctx>` wraps an
`Arc<AgentHarness>` plus a stable `name`, `description`, and optional
`system_prompt`. Invoking it always produces a **child run one level deeper** in
the recursion tree:

- `SubAgent::invoke` / `invoke_with_events` run a fresh child loop.
- `SubAgent::invoke_in_parent(state, ctx_data, parent, input)` threads the *live*
  parent `RunContext`, so the child inherits the parent's depth and event sink —
  the child runs at `parent.depth() + 1` and its events (and usage) surface on
  the parent's stream. It emits `SubAgentStarted` / `SubAgentCompleted` around
  the child loop, making the recursion observable.
- `SubAgentTool` adapts a sub-agent into a `Tool`, so a parent model can call an
  entire agent the same way it calls any tool — **a model calling a model**. The
  child input is read from the `SUBAGENT_INPUT_FIELD` (`"input"`) argument. Depth
  is fixed at construction (`with_parent_depth`) because the `Tool` trait gives
  `call` no live parent context.
- When a sub-agent invoked through `SubAgentTool` hits a child run limit
  (`max_model_calls`, `max_tool_calls`, wall-clock timeout, or depth), the tool
  returns an error `ToolResult` telling the parent orchestrator the delegated
  agent hit its limit. The parent run can continue and decide whether to narrow,
  split, retry, or report the task instead of mistaking the child limit for a
  completed answer.
- `SubAgentSession` keeps the *same* sub-agent alive across turns, accumulating a
  transcript (post-completion reuse, e.g. human-in-the-loop). Each reuse emits
  `SubAgentReused`.

Depth is bounded by `RunLimits::max_depth`; overrunning it yields
`TinyAgentsError::SubAgentDepth`. The graph analogue is `graph::subgraph`
(`SubAgentNode`), where a node embeds another compiled graph with the same depth
tracking.

## Steering (`steering`)

Steering is typed runtime control of an *already-running* agent (distinct from
sub-agent reuse, which acts between runs). An orchestrator — a parent agent, a
human UI, a graph supervisor, or a test — holds a cloneable `SteeringHandle`,
calls `send(command)`, and the loop `drain`s pending commands at a **safe
checkpoint** (before each model call). Commands are `SteeringCommand`: `Pause`,
`Resume`, `Cancel`, `InjectMessage`, `Redirect { instruction }`, and
`SetMetadata`. Each command reports its `SteeringCommandKind` (via `kind()`),
which a `SteeringPolicy` allowlist checks
(`SteeringPolicy::new` permits nothing by default; `allow_all` for tests), and a
disallowed command is rejected with `TinyAgentsError::Steering`. Applying a batch
yields a `SteeringOutcome` (`Continue` / `Pause` / `Cancel`). Commands are
`Serialize`/`Deserialize`, so steering can be logged, transported, and replayed.

## Summarization (context-window-aware) (`summarization`)

`summarization` keeps the working transcript inside the model's context window.
`estimate_tokens(text)` and `TokenEstimate` provide a cheap budget;
`trim_messages(messages, strategy)` applies a `TrimStrategy`; and
`SummarizationPolicy` decides *when* to compact. A policy can be derived from the
model's own profile — `SummarizationPolicy::from_profile(profile, threshold)` and
`with_context_window(max_input_tokens)` — so the trigger budget scales with the
target model. `should_summarize(messages)` and `plan(messages)` split the
transcript into a "summarize these / keep these" partition, the `Summarizer`
trait (with `ConcatSummarizer`) produces the summary, and `SummaryRecord` /
`CompressionProvenance` record what was compressed so the compaction is
auditable. This matters for recursion: deep sub-agent transcripts stay bounded
instead of blowing the parent's budget.

## Events and run status (`events`)

`events` is the observability spine. `AgentEvent` enumerates lifecycle
boundaries — run, model (incl. `ModelDelta`), tool, middleware, sub-agent,
cache hit/miss, retry/rate-limit/fallback, route selection, usage/cost,
limit-reached (`LimitKind`), memory, and compression. Events flow
through an `EventSink` to `EventListener`s (e.g. `RecordingListener`), are
journaled in an `EventJournal` (with `replay_from(offset)`), and the run's
overall state is `HarnessRunStatus`. Because child runs share the parent's
`EventSink`, a single stream shows the *entire recursion tree* — parent and child
model/tool/sub-agent events interleaved with correct `depth` annotations.

## Durable observability (`observability`)

While `events` is the *in-memory* spine, `harness::observability` is the
**durable** side that a journal or external trace needs without a live broadcast.
`AgentObservation` wraps each `AgentEvent` with the correlation metadata a trace
backend needs — `event_id`, `run_id`, `parent_run_id` / `root_run_id` lineage,
the stream `offset`, and a `ts_ms` wall-clock timestamp — so the whole recursion
tree can be reconstructed off-process. Persistence is pluggable: the
`HarnessEventJournal` trait (`InMemoryEventJournal`, `StoreEventJournal<A>` over
an `AppendStore`) durably records observations, and the `HarnessStatusStore`
trait (`InMemoryStatusStore`) snapshots `HarnessRunStatus`. Sinks compose over an
`EventSink`: `FanOutSink` (multiplex), `RedactingSink` (scrub sensitive fields),
`JournalSink` (write through to a journal), and `JsonlSink` (append JSONL).
`AgentLatencyMetrics::from_record(...)` summarizes a run's timing —
`run_elapsed_ms`, per-call `AgentCallLatency` lists, and `total_model_ms` /
`max_model_ms` / `total_tool_ms` / `max_tool_ms` roll-ups — turning the event
record into measurable latency across the subtree.

## Messages and typed identifiers (`message`, `ids`)

`harness::message` owns the chat shapes the loop carries: `Message`
(`SystemMessage`, `UserMessage`, `AssistantMessage`, `ToolMessage`), the
`ContentBlock` enum (with `ImageRef` for multimodal content), and `MessageDelta`
for streaming. `harness::ids` defines the newtype identifiers that thread
parent/child lineage through every layer — `RunId`, `ThreadId`, `CallId`,
`EventId`, `ComponentId`, `GraphId`, `NodeId`, `TaskId`, `SessionId`, `CellId`,
`CheckpointId`, and `InterruptId` — plus the `ExecutionStatus` and `HarnessPhase`
enums. Stable typed ids are what let usage, events, and caches all agree on
*which* run in the recursion tree they describe.

## Prompt templates and cache-segmented prompts (`prompt`)

`harness::prompt` builds model-visible prompts deterministically. `PromptTemplate`
renders `{{var}}` substitutions (`render`, `render_system`, `render_user`,
`render_assistant`) with a `TemplateRole`, and `MessagesTemplate` renders an
ordered multi-message conversation. `PromptBuilder` assembles a request out of
named, ordered segments — `push_system`, `push_tools_segment`,
`push_instructions`, `push_history`, and `push_volatile` — then `build(tail)`
emits a `ModelRequest`. Because the stable segments come before the volatile
ones, `fingerprint()` identifies the cacheable prefix, which is exactly what the
`PromptCacheLayout` machinery in [`cache`](#cache-cache) needs to keep a
provider's prompt/KV cache stable across turns.

## Testkit (`testkit`)

`harness::testkit` makes the loop deterministically testable: `ScriptedModel` /
`StreamingMock` / `SlowModel` fake providers, `FakeTool`, `DeterministicClock`,
`DeterministicIds`, `EventRecorder`, and a `Trajectory` assertion helper
(`from_events(...).assert_tool_called(...)`, `assert_model_called_times(n)`,
`assert_order(&[...])`, `assert_completed()`). Trajectory assertions are how you
verify the *shape* of a recursive run — that the orchestrator called the model,
then a sub-agent tool, then the model again, and completed.

## Unknown-tool recovery

When a model calls an unregistered tool, `RunPolicy::unknown_tool:
UnknownToolPolicy` decides what the loop does: `Fail` (default) aborts with
`TinyAgentsError::ToolNotFound`; `ReturnToolError` injects a tool-error result
(naming the requested tool and echoing its arguments) and continues so the model
can retry; `Rewrite { tool_name }` retargets the call to a fixed compatibility
tool and retries the lookup once (falling back to `ReturnToolError` if that
target is also missing). Every recovery consumes a tool-call slot (bounded by
`max_tool_calls`) and emits `AgentEvent::UnknownToolCall { call_id,
requested_name, arguments, recovery }`, preserving the original `arguments` for
repair/analysis. See [tool.md](../docs/modules/harness/tool.md#unknown-tool-recovery).

## Workspace isolation

`harness::workspace` owns the SDK-neutral hooks for tools that run over real
files. A `WorkspaceDescriptor` (root + trusted roots + `policy_id` +
`SandboxMode`) tells a tool which paths it may touch;
`RunContext::with_workspace(descriptor)` threads it into every
`ToolExecutionContext` so a tool reads its allowed root from context, not a
global. `WorkspaceIsolation` providers prepare/tear down per-agent environments
(`SharedRootWorkspace` is the built-in), driven by `prepare_workspace` /
`cleanup_workspace` which emit `WorkspacePrepared` / `WorkspaceCleanup`.
`WorkspaceDescriptor::enforce(path, &events)` is a fail-closed lexical gate: an
out-of-root path emits `WorkspaceViolation` and returns
`TinyAgentsError::Validation`. See
[workspace.md](../docs/modules/harness/workspace.md).

## Tool policy enforcement

Each tool advertises a serializable `ToolPolicy` (`side_effects`, `runtime`,
`access`, and a `classified` flag) via `Tool::policy()`. `ToolPolicyMiddleware`
enforces it at both exposure (`before_model` — hide the tool) and execution
(`before_tool` — reject with `Validation`). Beyond `strict` /
`deny_side_effects` / `require_classification` / `require_background_safe`, the
enforcement builders add `require_sandbox(true)` (block a
`SandboxMode::Required` tool unless the run carries a `Required`-sandbox
[workspace](#workspace-isolation)), `require_approval([names])` (block
`approval_required` tools not on the list), and `enforce_result_bytes(true)`
(truncate + flag results over `runtime.max_result_bytes` in `after_tool`). See
[middleware.md](../docs/modules/harness/middleware.md#tool-policy-enforcement).

## Tool exposure

`ContextualToolSelectionMiddleware` filters model-visible tools per call using a
predicate that sees a `ToolSelectionContext { run_id, depth, tags,
requested_model }`, so exposure can vary by recursion depth or run tags. It emits
`AgentEvent::ToolsFiltered { by, excluded, remaining }` whenever it withholds
tools. `ContextualToolSelectionMiddleware::inheriting(parent_allow, parent_deny,
child_allow, child_deny)` composes a child policy against an inherited parent
one so a sub-agent can only *narrow*: deny is additive (parent ∪ child), allow is
intersective. See
[middleware.md](../docs/modules/harness/middleware.md#tool-exposure).

## Middleware control

A middleware can steer the loop out-of-band with
`RunContext::request_control(MiddlewareControl)`: `StopWithFinal(text)` ends the
run with a final answer, `Interrupt { node, message }` pauses at the next safe
checkpoint (surfacing `TinyAgentsError::Interrupted`). `request_control` keeps
the highest-`precedence()` pending request (`Interrupt` > `StopWithFinal`),
never last-writer-wins, so a pause is never downgraded to a stop. The loop drains
it via `take_control` and emits `AgentEvent::ControlApplied { control, detail }`
when it honors one. See
[context.md](../docs/modules/harness/context.md#middleware-control-outcomes).

## Cost & budget

`BudgetMiddleware` enforces `BudgetLimits` across a run — or a whole recursive
tree, when a shared `BudgetTracker` is handed to every sub-agent. Alongside the
token/cost limits it adds `max_cached_input_tokens` (enforced against
`Usage.cache_read_tokens`) and a **preflight reservation**: `before_model`
estimates the upcoming call's input tokens and blocks *before dispatch* if it
would breach the input budget (`BudgetReserved { estimated_input_tokens }` on
success, `BudgetExceeded { blocked: true }` on block), then `after_model`
reconciles the estimate against provider-reported usage
(`BudgetReconciled { estimated_input_tokens, actual_input_tokens }`). The
existing `BudgetWarning` / `BudgetExceeded { blocked: false }` post-spend signals
are unchanged. See [cost.md](../docs/modules/harness/cost.md#budget-middleware).

## Stable event ids and delta attribution

Emitted `EventId`s are `{stream_id}-evt-{offset}`. `RunContext` seeds the
`stream_id` from the run id (`EventSink::with_stream_id`), so ids are stable
across process restarts and never collide between runs; default sinks
(`EventSink::new`) get a process-unique prefix instead. Because a sink is shared
across the recursion tree, `AgentEvent::ModelDelta` now carries `run_id` (next to
`call_id` and the `MessageDelta`) so a streamed delta can be routed to its
run/thread lineage regardless of which sink delivered it. See
[observability.md](../docs/modules/harness/observability.md#stable-event-ids-and-delta-attribution).

## See also

- [Graph Runtime](Graph-Runtime.md) — durable typed state graphs and subgraphs.
- [Providers](Providers.md) — configuring hosted providers behind `openai`.
- [Examples](Examples.md) — annotated, runnable catalog.
- [`docs/modules/harness/README.md`](../docs/modules/harness/README.md) — the
  full module specification.
