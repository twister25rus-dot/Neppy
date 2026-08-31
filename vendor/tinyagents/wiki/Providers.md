# Providers

TinyAgents is a recursive language-model (RLM) harness: models call models,
agents call agents, and graphs run graphs. Every one of those recursive calls
eventually reaches a concrete chat model. This page explains how to wire up the
model providers that sit at the leaves of that recursion.

The harness keeps model calls behind provider-neutral traits. A provider adapter
translates a `ModelRequest` into the provider wire format and normalizes the
response, usage, streaming chunks, and errors back into TinyAgents types, so the
rest of the recursive runtime never sees provider-specific shapes.

## Offline by default, providers compiled in

The default build is **fully offline at run time**: no provider call happens
unless you make one. Two chat models ship compiled by default — `MockModel`
(deterministic, network-free, for tests/examples/harness development) and
`OpenAiModel` (the hosted OpenAI + OpenAI-compatible adapter, which pulls no
extra dependencies).

The hosted provider ships with the crate — no feature flag required:

```sh
cargo add tinyagents
```

`harness::providers::openai`, the hosted **OpenAI Chat Completions** adapter, is
compiled in by default (it pulls no extra dependencies — `reqwest` is always
present). This single adapter (`OpenAiModel`) is also the OpenAI-compatible
client used to reach every other hosted provider listed below, because they all
speak the same Chat Completions wire protocol.

> Accuracy note: `OpenAiModel` is the one concrete HTTP-backed provider that is
> implemented. The other providers below exist as **configuration specs and
> convenience constructors** that point `OpenAiModel` at the right base URL,
> default model, and API-key environment variable. There are no separate
> `anthropic`/`ollama`/etc. Cargo features yet — the placeholder module
> declarations in `harness::providers` are reserved for future native adapters.

## Provider kinds and specs

`ProviderKind` enumerates the provider families the factory understands, and
`ProviderSpec` is the portable configuration the harness binds by name.

`ProviderKind` covers:

| Kind | `provider` id | Default model | Base URL | API key env | Key required |
|---|---|---|---|---|---|
| `OpenAi` | `openai` | `gpt-4.1-mini` | `https://api.openai.com/v1` | `OPENAI_API_KEY` | yes |
| `Anthropic` | `anthropic` | `claude-3-5-sonnet-latest` | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` | yes |
| `Ollama` | `ollama` | `llama3.2` | `http://localhost:11434/v1` | _(none)_ | no |
| `DeepSeek` | `deepseek` | `deepseek-chat` | `https://api.deepseek.com/v1` | `DEEPSEEK_API_KEY` | yes |
| `Groq` | `groq` | `llama-3.3-70b-versatile` | `https://api.groq.com/openai/v1` | `GROQ_API_KEY` | yes |
| `Xai` | `xai` | `grok-2-latest` | `https://api.x.ai/v1` | `XAI_API_KEY` | yes |
| `OpenRouter` | `openrouter` | `openai/gpt-4o-mini` | `https://openrouter.ai/api/v1` | `OPENROUTER_API_KEY` | yes |
| `Together` | `together` | `meta-llama/Llama-3.3-70B-Instruct-Turbo` | `https://api.together.xyz/v1` | `TOGETHER_API_KEY` | yes |
| `Mistral` | `mistral` | `mistral-small-latest` | `https://api.mistral.ai/v1` | `MISTRAL_API_KEY` | yes |
| `Compatible` | `compatible` | _(unset)_ | _(unset)_ | _(none)_ | yes |

`ProviderSpec::for_kind(kind)` returns the default spec above. `Compatible` is
the escape hatch for any endpoint that implements the OpenAI Chat Completions
protocol but does not need a named preset — supply your own base URL, model, and
key env.

```rust
use tinyagents::harness::providers::{ProviderKind, ProviderSpec};

let spec = ProviderSpec::for_kind(ProviderKind::Groq)
    .with_model("llama-3.3-70b-versatile");
```

Each spec records the provider kind, the `provider` id (used in profiles and
normalized errors), the default model id, the base URL, the API-key environment
variable when one is required, and whether a real API key is required at all.
Builder methods let you override any of these: `with_model`, `with_base_url`,
`with_provider`, and `with_api_key_env`.

## Quick start: OpenAI

### 1. Configure credentials

Export the key directly:

```sh
export OPENAI_API_KEY=sk-...
export OPENAI_MODEL=gpt-4.1-mini        # optional, defaults to gpt-4.1-mini
export OPENAI_BASE_URL=https://api.openai.com/v1   # optional override
```

Or keep them in a `.env` file at the repo root — the runnable examples load it
via `dotenvy`:

```dotenv
# .env
OPENAI_API_KEY=sk-...
OPENAI_MODEL=gpt-4.1-mini
```

### 2. Run an example

```sh
cargo run --example openai_chat
```

OpenAI-backed examples (`openai_chat`, `openai_tools`, `openai_structured`,
`openai_graph_agent`, `openai_self_blueprint`) require a valid `OPENAI_API_KEY`
at run time.

### 3. Build a model in code

```rust
use tinyagents::harness::providers::openai::OpenAiModel;

let model = OpenAiModel::from_env()?;
```

`OpenAiModel::from_env()` reads `OPENAI_API_KEY` (required), `OPENAI_MODEL`
(optional), and `OPENAI_BASE_URL` (optional). Use `OpenAiModel::new(api_key)`
when you want to pass the key yourself.

### 4. Discover available models

`OpenAiModel::list_models()` calls the provider's `GET {base_url}/models`
endpoint and returns a typed `Vec<ModelListing>` (`id`, optional `created` /
`owned_by`). It's a provider/account-level call — independent of which model the
handle is bound to — and works for every OpenAI-compatible endpoint, so it
doubles as runtime model discovery for local/self-hosted providers (Ollama,
vLLM, …). Any returned `id` can be passed to `with_model`.

```rust
let provider = OpenAiModel::from_env()?;
for listing in provider.list_models().await? {
    println!("{}", listing.id);
}
```

## Reaching other providers through the compatible adapter

Because every provider in the table speaks the OpenAI Chat Completions protocol,
`OpenAiModel` ships named constructors that preset the provider id, base URL, and
default model. Each constructor (except `ollama`) takes the API key:

```rust
use tinyagents::harness::providers::openai::OpenAiModel;

let anthropic  = OpenAiModel::anthropic(std::env::var("ANTHROPIC_API_KEY")?);
let deepseek   = OpenAiModel::deepseek(std::env::var("DEEPSEEK_API_KEY")?);
let groq       = OpenAiModel::groq(std::env::var("GROQ_API_KEY")?);
let xai        = OpenAiModel::xai(std::env::var("XAI_API_KEY")?);
let openrouter = OpenAiModel::openrouter(std::env::var("OPENROUTER_API_KEY")?);
let together   = OpenAiModel::together(std::env::var("TOGETHER_API_KEY")?);
let mistral    = OpenAiModel::mistral(std::env::var("MISTRAL_API_KEY")?);
let ollama     = OpenAiModel::ollama();   // local, no key
```

Override the default model with `.with_model(...)`:

```rust
let groq = OpenAiModel::groq(std::env::var("GROQ_API_KEY")?)
    .with_model("llama-3.1-8b-instant");
```

### From a `ProviderSpec`

To drive the choice from configuration, build a spec and construct the model
from it:

```rust
use tinyagents::harness::providers::{ProviderKind, ProviderSpec};
use tinyagents::harness::providers::openai::OpenAiModel;

let spec = ProviderSpec::for_kind(ProviderKind::Mistral)
    .with_model("mistral-small-latest");

let model = OpenAiModel::from_spec_env(spec)?;
```

`from_spec_env` reads the spec's configured environment variable. Use
`from_spec(spec, api_key)` when credentials come from another secret source, and
`compatible(...)` for a fully custom OpenAI-compatible endpoint.

## Ollama (local)

Start Ollama with its OpenAI-compatible endpoint at
`http://localhost:11434/v1`, then:

```rust
use tinyagents::harness::providers::openai::OpenAiModel;

let model = OpenAiModel::ollama().with_model("llama3.2");
```

Ollama ignores the API key, so TinyAgents supplies a placeholder and the spec
marks `requires_api_key = false`.

## Error and stream normalization

Providers report failures as `ProviderError`, carrying the provider id, the
model id when known, the HTTP status when available, the provider error code
when available, a human-readable message, a retryability hint, and the raw
provider payload when useful.

Streaming providers emit `ModelStreamItem` values: normal chunks become deltas,
usage updates, tool-call chunks, or a final message. Provider-side stream errors
become `ModelStreamItem::ProviderFailed`, so the accumulator and the harness see
the same normalized failure shape as non-streaming calls. This uniform shape is
what lets sub-model, sub-agent, and sub-graph calls roll their usage, cost, and
errors up to the parent run.

Reasoning/thinking output is normalized separately from visible assistant text.
OpenAI-compatible streams that expose `delta.reasoning_content` or
`delta.reasoning` become `MessageDelta.reasoning`; `delta.content` remains the
visible text channel. When usage includes
`completion_tokens_details.reasoning_tokens`, the adapter records it as
`Usage.reasoning_tokens` so cost and budget middleware can account for hidden
thinking tokens without rendering them as final text.

## Provider selection from a model string

`ProviderKind::infer(...)` supports LangChain-style explicit prefixes such as
`openai:gpt-4.1-mini`, `anthropic:claude-...`, and `ollama:llama3.2`, plus
conservative bare-model inference for common families (`gpt-`/`o1`/`o3`/`o4` →
OpenAI, `claude` → Anthropic, `deepseek` → DeepSeek, `grok` → xAI,
`mistral`/`mixtral` → Mistral).

Prefer explicit `ProviderSpec` values in production. Inference is convenient for
configuration files, examples, `.rag` blueprints, and interactive `.ragsh`
sessions where the model string is the only user input.

## Capability profiles and the model registry

Provider constructors produce executable models; the *resolution* layer in
`harness::model` decides which model a run actually uses. A `ModelProfile`
records what a model can do (modalities, tool calling, native structured output,
streaming tool-call chunks), letting the harness reject impossible requests
before a network call and pick capability-satisfying fallbacks — it is not a
pricing table. A `ModelRegistry` binds named models for a `State`, and a
`ResolvedModel` is the durable record of which registry name was selected for a
call (carrying the originally requested value and a `ModelResolutionSource`).
See [Harness](Harness.md) and [Registry](Registry.md) for how profiles, the
registry, and resolution drive model selection across a recursive run.

## See also

- [Harness](Harness.md) — the provider-neutral model traits these adapters
  implement.
- [Development](Development.md) — how to build and test with the `openai`
  feature enabled.
