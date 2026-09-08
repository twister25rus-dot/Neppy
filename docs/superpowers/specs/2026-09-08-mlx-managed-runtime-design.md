# MLX as a first-class managed runtime

**Status:** approved design, ready for implementation planning
**Date:** 2026-09-08

## 1. Problem

Neppy can talk to an MLX server but cannot run one. `MLX_PROFILE`
(`src/neppy/inference/local/profile.rs:151`) and the `mlx:`/`omlx:` branches in
`src/neppy/inference/provider/factory.rs:1816` build an OpenAI-compatible client
against a URL the user is expected to have started by hand. Nothing spawns,
supervises, configures or stops a process.

Three specific defects follow from that:

1. `LocalAiProvider` (`src/neppy/inference/local/provider.rs:6`) has only
   `Ollama` and `LmStudio`, and `provider_from_config` maps everything else to
   `Ollama` (`provider.rs:79`). Selecting MLX therefore still requires Ollama to
   be running, because bootstrap falls through to `ensure_ollama_server`
   (`src/neppy/inference/local/service/bootstrap.rs:284`).
2. The settings UI never offers MLX at all —
   `LOCAL_RUNTIME_SLUGS = ['ollama','lmstudio','omlx']`
   (`app/src/components/settings/panels/aiRouting.ts:11`).
3. `mlx`, `omlx` and vision are modelled as three separate providers when they
   are one runtime with different flags.

## 2. Goal

Neppy owns the MLX process: start it, stop it, configure every parameter,
manage its models, and keep it inside a memory budget — with one unified "MLX"
provider replacing the `mlx`/`omlx`/vision sprawl.

## 3. Key finding: `mlx_vlm.server` is the unified runtime

`mlx-vlm` 0.7.0 is installed (`~/.local/bin/mlx_vlm.server`). It is not a
vision-only sibling of `mlx_lm.server`; it is a superset that serves six model
slots from one process:

`--model` (LLM or VLM) · `--embedding-model` · `--reranker-model` ·
`--stt-model` · `--tts-model` · `--image-model`

Its OpenAI-compatible surface covers every role Neppy has:

| Route | Role |
| --- | --- |
| `/v1/chat/completions`, `/v1/responses` | chat, reasoning, vision |
| `/v1/embeddings` | embeddings |
| `/v1/audio/transcriptions`, `/v1/audio/speech` | STT, TTS |
| `/v1/rerank`, `/v1/images/generations` | new capabilities |
| `/health`, `/metrics`, `/settings`, `/unload`, `/cache/stats`, `/cache/reset` | live admin without restart |

Consequently the "merge MLX services" requirement is a **config collapse, not an
orchestration problem**:

- `omlx` → `--api-key`
- vision (`vmlx`) → `--model` pointed at a VLM checkpoint
- reasoning → `--enable-thinking`, `--thinking-budget`,
  `--thinking-start-token`, `--thinking-end-token`

It also natively provides most of what would otherwise be a bespoke optimizer:
KV-cache quantization (`--kv-bits` including TurboQuant, `--kv-group-size`,
`--max-kv-size`, `--quantized-kv-start`), `--max-num-seqs` backpressure,
`--expert-cache-gb` MoE offload, and speculative decoding
(`--draft-model`, `--draft-kind {dflash,eagle3,mtp}`).

Two current profile facts are wrong and must be corrected: `mlx_vlm.server`
does expose `/v1/responses`, and it does native tool calling
(`tools`/`tool_choice`/`tool_calls`, with per-model parsers in
`mlx_vlm/tool_parsers/`) rather than prompt-guided.

## 4. Constraint: the vendored config is not ours

`LocalAiConfig` lives in
`vendor/tinymemory/crates/tinymemory-api/src/host/local_ai.rs`, and
`vendor/tinymemory` is a git submodule tracking upstream
`tinyhumansai/tinymemory`. Its docstring fixes the provider set to
`ollama | lm_studio | omlx`.

**Decision:** all new MLX configuration lives in a new Neppy-owned `[mlx]`
config section. The vendored struct is not modified, so the fork carries no
submodule diff.

## 5. Design

### 5.1 Config — new `[mlx]` section

New module `src/neppy/config/schema/mlx.rs`.

```toml
[mlx]
enabled = true
bin_dir = "~/.local/bin"
memory_budget_gib = 26.0
embeddings_backend = "ollama"   # "ollama" | "mlx"; default ollama

[[mlx.server]]
id = "primary"
kind = "vlm"                    # "vlm" (default) | "lm"
host = "127.0.0.1"              # never the upstream 0.0.0.0 default
port = 0                        # 0 = auto-assign
auth = "none"                   # "none" | "bearer"
model = "mlx-community/Qwen3.8-27B-nvfp4"
embedding_model = ""
reranker_model = ""
stt_model = ""
tts_model = ""
image_model = ""
# sampling / generation
max_tokens = 512
enable_thinking = false
thinking_budget = 0
# performance
kv_bits = 0
max_kv_size = 0
max_num_seqs = 0
prefill_step_size = 2048
draft_model = ""
draft_kind = ""                 # dflash | eagle3 | mtp
expert_cache_gb = 0
vision_cache_size = 20
model_discovery = "hf-cache"    # served | hf-cache
adapter_path = ""
trust_remote_code = false
log_level = "INFO"
```

A `[[mlx.server]]` block is the unit of everything. The default configuration is
**one block**; multiple blocks are supported for isolation but are not the
normal case.

### 5.2 Supervisor — `src/neppy/inference/local/service/mlx_admin/`

Mirrors the existing `ollama_admin/` module layout so it reads like its
neighbour.

| File | Responsibility |
| --- | --- |
| `binary.rs` | Locate `mlx_vlm.server` / `mlx_lm.server`, probe version, actionable error when absent (`uv tool install mlx-vlm`) |
| `process.rs` | Spawn/kill one server, owned-child tracking, argv construction from a server block |
| `pool.rs` | Desired-state reconciliation: make running processes match the configured blocks |
| `health.rs` | Poll `/health` and `/v1/models`; state machine `stopped/starting/ready/degraded/crashed` |
| `memory.rs` | Admission control: estimate resident size, enforce `memory_budget_gib`, prefer `/unload` over kill to reclaim |
| `models.rs` | HF cache scan with sizes, download with progress, delete, disk totals, adapter paths |
| `logs.rs` | Bounded per-process stdout/stderr ring buffer, tailed to the UI |

`spawn_marker.rs` is extended to be keyed by server id so orphan reclamation
works per-process, reusing the existing marker/PID-liveness logic rather than
duplicating it.

**Role routing.** Neppy already resolves providers per role —
`chat_provider`, `reasoning_provider`, `vision_provider`, `embeddings_provider`
(`src/neppy/config/schema/types.rs:674`). The pool reads which distinct MLX
models those roles reference; roles naming the same model share one process.

### 5.3 Runtime integration

- Add `Mlx` to `LocalAiProvider`, plus an `ensure_mlx_available` branch in
  `bootstrap.rs` so MLX no longer falls through to `ensure_ollama_server`.
- Replace `provider_from_config`'s `_ => Ollama` catch-all with an explicit
  match, so an unknown provider degrades loudly instead of silently booting
  Ollama.
- Keep parsing the `mlx:` and `omlx:` model prefixes for back-compat, but route
  both through the one unified provider path; retire `OMLX_PROFILE` as a
  separate kind.
- Correct `MLX_PROFILE`: `supports_responses_api: true`, `tool_support: Native`.
- When `embeddings_backend = "ollama"`, Ollama is ensured for embeddings and TTS
  only — never as a precondition for chat.

### 5.4 Control surface

New RPC ops, exposed as Tauri commands and driven by a dedicated MLX panel in
Settings (`mlx` added to `LOCAL_RUNTIME_SLUGS`):

`mlx_status` · `mlx_start` / `mlx_stop` / `mlx_restart` (per server id) ·
`mlx_apply_params` · `mlx_unload` · `mlx_logs_tail` ·
`mlx_models_list` / `mlx_models_download` / `mlx_models_delete`

The panel renders one card per block: state pill, port, resident RAM, live
metrics, start/stop, a full parameter form, and a log tail.

### 5.5 Error handling

- Crash: capture exit code and the tail of stderr, surface both, and do **not**
  auto-restart in a loop — restart is an explicit action, consistent with the
  existing reasoning at `bootstrap.rs:140`.
- Port conflict: auto-reassign and record the chosen port.
- Admission refusal is a first-class UI state naming the shortfall and the
  process to stop, not an error toast.
- Missing binary is a distinct, actionable state, not a generic failure.

### 5.6 Security

- Always pass `--host 127.0.0.1`. Upstream's default is `0.0.0.0`, which would
  expose a local model on the LAN.
- Set `--allowed-origins` to Neppy's own origin rather than the `*` default.
- `trust_remote_code` executes arbitrary model code: off by default, behind an
  explicit per-server confirmation.
- Hugging Face download hosts require an egress allowlist entry
  (`src/neppy/security/egress`). Downloads are user-initiated only, preserving
  the zero-egress offline guarantee established in commit `97aa11a`.

### 5.7 Testing

- Unit: argv construction covering every flag, memory estimation, port
  assignment, health and `/v1/models` parsing, marker reclamation.
- Integration: a fake server script standing in for `mlx_vlm.server`, so
  spawn/kill/orphan-reclaim are tested without loading model weights — the same
  approach as `ollama_admin_tests.rs`.
- New code carries real behavioural tests; this work does not touch the untested
  security-policy surfaces catalogued in `plan.md`.

## 6. Phasing

1. **Config + single-process lifecycle.** `[mlx]` section, `binary.rs`,
   `process.rs`, `health.rs`, `LocalAiProvider::Mlx`, bootstrap branch, profile
   corrections. Outcome: start/stop the primary server from Settings with all
   parameters.
2. **Pool, admission control, provider collapse.** `pool.rs`, `memory.rs`,
   multi-block support, role sharing, `omlx`/vision folded into the unified
   provider.
3. **Model management.** `models.rs`, HF browse/download/delete, adapters.

Deferred: an auto-optimizer that tunes context and KV bits under memory
pressure. `mlx_vlm.server`'s native KV-quantization and `--max-num-seqs`
backpressure cover most of that need.

## 7. Out of scope

- Routing Neppy's STT/TTS through MLX, though the runtime supports it.
- Removing Ollama. `embeddings_backend` makes that a later config change once a
  1024-dim MLX embedder is chosen; the memory tree is fixed at 1024 dims.
- Any change to `vendor/tinymemory`.
