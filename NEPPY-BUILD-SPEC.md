# Neppy — build spec

A personal fork of `tinyhumansai/openhuman`, cut off from the TinyHumans backend
and rebranded. Target directory: `/Users/alex/Neppy`.

Every line below is tagged `observed` (read from the upstream repo's own docs),
`inferred` (follows from something observed), or `guessed` (needs checking before
you rely on it). When the rebuild misbehaves later, the bug is almost always
sitting on a `guessed` line.

---

## 0. Verdict before you start

**This is a fork job, not a reverse-engineering job.** The source is GPL-3.0 and
public, so Phases 1–3 of the teardown method (inventory, probe, infer) do not
apply. Go straight to spec and rebuild.

**The rename is the easy half. The hard half is that the app is gated on their
backend even when inference is local.** From upstream `AGENTS.md`:

> A custom provider is gated on an active app session (`verify_session_active`),
> even for a host that supplied the endpoint and key itself.

> Point `backend_url` somewhere real or stubbed. The core makes non-inference
> backend calls regardless of where inference goes. Signed out of the hosted
> backend, those are rejected, a rejection publishes `SessionExpired`, and the
> *next* turn then fails the provider gate for reasons unrelated to the turn.

`observed`. Meaning: point it at Ollama, sign out, and you get an app that fails
in a way that looks like a model problem and isn't. Solving that is job one.

**Two things will not survive the disconnect at full quality.** Say this out loud
now so it isn't a surprise in week three:

1. The 118 one-click OAuth integrations run through a hosted OAuth proxy app that
   TinyHumans registers and maintains. There is no free local equivalent. They
   die. Replacement is MCP servers with your own OAuth credentials, one per
   service you actually use. `observed` (the proxy), `inferred` (the replacement).
2. The agent harness, prompts and tool-calling dialects are tuned for frontier
   models. A 7B–14B local model will drop tool calls and loop. `inferred`.
   Realistic setup: Qwen3 30B-class or better for the orchestrator if the machine
   takes it, small local models for summarisation and background work.

---

## 1. What you're forking

| Path | Role | Source |
|---|---|---|
| `app/` | pnpm workspace, Vite + React UI, Tauri v2 desktop host | observed |
| `src/` | Rust lib crate + `neppy-core` CLI binary, `src/openhuman/*` domains | observed |
| `vendor/` | git submodules: `tinyhumans-sdk` and the `tiny*-bus` contract crates | observed |
| `gitbooks/`, `docs/` | contributor and internals docs | observed |

Toolchain: Node 24+, pnpm 10.10.0, Rust 1.93.0, CMake. On Apple Silicon prefix
Rust commands with `GGML_NATIVE=OFF`. `observed`.

Licence: GPL-3.0. For personal use you have no distribution obligation at all.
If you ever publish the fork: keep the licence, keep the copyright notices, mark
the files you changed (GPL-3 §5a). The *name* "Neppy" and the TinyHumans
logos are trademarks and are not covered by the GPL, so renaming to Neppy is the
correct thing to do rather than just a preference. `observed`.

---

## 2. The cloud coupling map

This is the actual work list. Each row is a place the app reaches TinyHumans.

### 2.1 Session and auth — do this first

- `src/api/jwt.rs` — session-token retrieval. `observed`
- `src/api/config.rs` — base URL / env resolution. `observed`
- `src/api/rest.rs` — error classification, `classify_sdk_error`, Sentry policy.
  401 maps to `SESSION_EXPIRED`. `observed`
- `desktop::app_state::ops` — hand-rolled client hitting `GET /auth/me`. `observed`
- `verify_session_active` — the gate that refuses your own BYOK provider. `observed`

Fix, in order of increasing surgery:

1. `Session::local("neppy")` satisfies the gate without asserting anything at the
   backend — upstream says so explicitly for the `Harness` embed path. Try this
   first; it may be enough for the CLI/headless path. `observed`
2. Make `verify_session_active` return `Ok` unconditionally in the desktop path.
   `inferred`
3. Run a tiny local stub backend on `127.0.0.1:8788` that answers `/auth/me` and
   the `{success, data}` envelope for whatever else still fires, and set
   `backend_url` to it. ~100 lines of Axum or Express. This is the belt-and-braces
   option and the one that stops surprise `SessionExpired` events. `inferred`

Do **not** skip 3 on the assumption that 1 and 2 cover it. The backend calls come
from ~35 sites across the domains and two of them hand-roll their own reqwest
client, so they inherit nothing from the wrappers. `observed`.

### 2.2 Turn the hosted domains off rather than patching them

`DomainSet` is a runtime axis on `CoreBuilder` with one flag per `DomainGroup`.
Set these false:

- `Hosted` — announcements, billing, orchestration, referral, team. Upstream
  describes all five as "thin proxies to the TinyHumans backend". `observed`
- `Relay` — tiny.place, the agent social network. `observed`

A gated domain's controllers become unknown-method, its agent tools disappear,
its stores and subscribers never initialise. That is cleaner than stubbing.
`observed`.

### 2.3 Inference

`Provider::openai_compatible(base_url, key)` already exists in the `Harness`
builder. `observed`. So:

- Ollama: `http://localhost:11434/v1`, any dummy key.
- LM Studio: `http://localhost:1234/v1`.
- llama.cpp server: `http://localhost:8080/v1`.

Caveat, `observed`: "managed inference and embeddings go out through `tinyagents`'
own clients". So there is a second path that does not go through the config you
just set. Find it and force it onto the BYOK branch before you trust the offline
test.

### 2.4 Embeddings

Same seam (`inference::embeddings`). Local options through Ollama:
`nomic-embed-text` (768d, cheap) or `bge-m3` (1024d, better multilingual).
`guessed` — check what dimension the memory store schema expects before you
switch, because changing it later means reindexing everything.

### 2.5 Voice — the awkward one

Upstream **deleted** the bundled whisper.cpp engine. STT is now always a hosted
HTTP call, chosen by `voice_server.stt_engine` = `backend` / `elevenlabs` /
`openai`. `observed`.

Cheapest route: set `stt_engine = "openai"` and point the base URL at a local
server that speaks the OpenAI `/v1/audio/transcriptions` shape —
`faster-whisper-server` or `whisper.cpp`'s own server both do. `inferred`.

If you don't care about voice on day one, just leave the `voice` feature off. It
compiles out cleanly behind a facade + stub. `observed`.

### 2.6 Telemetry

- Sentry: behind the `crash-reporting` gate, already OFF in the contributor
  default set. Leave it off and never add it to your product feature list.
  `observed`
- Frontend analytics: `app/src/services/analytics.ts` is the consent/provider
  implementation. Make it a no-op. `observed`
- Langfuse: `agent::progress_tracing::langfuse` posts to
  `POST /telemetry/langfuse/ingestion` on their backend with a bare
  `reqwest::Client::new()`. Also `flows::tinyflows::langfuse_export`. Both need
  killing at the call site — they don't share a client with anything. `observed`

### 2.7 Updater

Tauri IPC commands `check_app_update`, `apply_core_update`. Disable the updater
in `app/src-tauri/tauri.conf.json` or repoint it at your own releases. `observed`.

### 2.8 Skills catalog

Default catalog points at `tinyhumansai/openhuman-skills`; override with
`VITE_SKILLS_GITHUB_REPO`. Point it at your fork or a local directory. `observed`.

### 2.9 Native modules — decide your line here

`src/openhuman/modules/registry.rs` is a compiled-in const table pinning module
releases by SHA-256, downloaded from TinyHumans' GitHub releases. `documents`
depends on the `tinydocs` module; memory is half-migrated onto a `tinymemory`
module. `observed`.

These are GitHub release downloads with verified digests, not a live server call.
Two positions:

- **Pragmatic**: keep them. They're first-party signed artifacts fetched once.
- **Purist**: build the modules yourself from the `vendor/` submodule sources and
  load them with `OPENHUMAN_MODULE_PATH` pointing at a local directory. `observed`
  that this env var exists; `guessed` that a self-built artifact passes the
  admission gates, though upstream notes admission is deliberately permissive
  about toolchain mismatch, which is encouraging.

### 2.10 Integrations

Composio-backed OAuth (`src/openhuman/integrations/composio`). This is the one
that has no local replacement. Cut it and rebuild the services you need as MCP
servers — the `mcp` domain already supports both config-declared static servers
and dynamically installed ones. `observed`.

---

## 3. Rename rules

A blind `sed -i 's/openhuman/neppy/g; s/tinyhuman/neppy/g'` will break this build.
Here is the split.

### 3.1 Rename these

- All user-visible strings, including the six locale files (en, zh, ja, ko, de, ur)
- Agent prompts in `src/openhuman/agent/prompts/` — this is where the assistant
  says its own name, so it matters for "feels like the same app"
- README, INSTALL, docs, gitbooks
- `app/src-tauri/tauri.conf.json`: product name, window title, bundle identifier
- Crate and package names: `openhuman` → `neppy`, bin `neppy-core` →
  `neppy-core`, `neppy-fleet` → `neppy-fleet`, `openhuman-app`, `openhuman-repo`
- The directory `src/openhuman/` → `src/neppy/`, and every `crate::openhuman::`
  and `neppy_core::` path with it
- Env vars `OPENHUMAN_*` → `NEPPY_*`, everywhere at once: `.env.example`,
  `app/.env.example`, `scripts/load-dotenv.sh`, CI workflows, docs
- Filesystem roots: `~/.neppy` → `~/.neppy`, `~/Neppy/projects` →
  `~/Neppy/projects`

### 3.2 Do NOT rename these

| Thing | Why |
|---|---|
| Any `tiny*` crate name — `tinyagents`, `tinycortex`, `tinyflows`, `tinychannels`, `tinybus`, `tinymcp`, `tinymemory-api`, `tinymemory-core`, `tinydocs-bus`, `tinyvoice-bus`, `tinyjuice-bus`, `tinyruntime-bus`, `tinywallet-bus`, `tinyhumans-sdk` | Submodules and path dependencies. Rename and cargo cannot resolve them. `observed` |
| Anything under `vendor/`, and `.gitmodules` | Same reason |
| RPC namespace strings in `ControllerSchema` | They are string literals, deliberately decoupled from module paths. Upstream says a move "never changes the wire surface — do not rename namespace strings". Frontend catalog drift tests assert them. `observed` |
| Agent tool names: `tinyjuice_retrieve`, the `tokenjuice_retrieve` alias, `mcp_registry_*`, `media_generate_*`, `whatsapp_data_*`, `node_exec`, `npm_exec` | Matched against the owning crate's constants, and pinned by drift guards (`representative_tool_names_are_real`). `observed` |
| Builtin agent ids in `agent.toml`: `skill_setup`, `skill_executor`, `mcp_agent`, `workflow_builder`, `flow_discovery` | Data files resolved by id, asserted in loader tests. `observed` |
| Cargo feature gate names: `voice`, `inference`, `web3`, `media`, `documents`, `modules`, `skills`, `flows`, `mcp`, `memory-git`, `contacts`, `runtime-node`, `tui` | Forwarded by name from the shell manifest, asserted by `check-feature-forwarding.mjs` and `INFERENCE_COMPILED_IN`. Note `inference` is already a historical misnomer upstream and they kept it for exactly this reason. `observed` |
| Config migration names, e.g. `retire_local_whisper_stt` (9 → 10) | Upgrade path. Harmless on a fresh install, but nothing to gain. `observed` |
| `x-sdk-name` default value `openhuman` | Only reaches their backend. Once you stub the backend it's inert; if you're still talking to theirs during bring-up, changing it makes requests unattributed. `observed` |

### 3.3 Traps

- `openhuman-skills` (the separate skills repo) is a different rename target than
  `openhuman` the crate. Handle it separately or your regex eats the wrong thing.
- The `src/openhuman/` directory rename is the single biggest diff. Upstream
  measured a comparable rename inside `memory/` at ~545 import rewrites. Do it
  with `cargo fix`-style mechanical care and its own commit.
- Many tests hard-assert namespace and agent-id strings. Expect a red suite after
  the rename and fix the assertions, don't fix the code back.

---

## 4. Order of operations

This order is the point. Doing the rename first is the classic way to lose a
weekend.

**Phase A — green build, unmodified.**
Fork on GitHub, then:

```bash
git clone --recurse-submodules git@github.com:<you>/openhuman.git /Users/alex/Neppy
cd /Users/alex/Neppy
git remote add upstream https://github.com/tinyhumansai/openhuman.git
git checkout -b neppy
pnpm install
GGML_NATIVE=OFF cargo check --manifest-path Cargo.toml
pnpm dev:app
```

Stop here until the untouched app builds and launches. If you skip this, every
later error is ambiguous.

**Phase B — de-cloud, still called Neppy.**
Sections 2.1 → 2.10 above. Success condition: turn off wifi, run one full agent
turn against Ollama, get a sane reply with a tool call in it.

**Phase C — slim the build.**
Personal, local, no crypto:

```bash
GGML_NATIVE=OFF cargo build --no-default-features \
  --features "flows,skills,mcp,modules,memory-git,media,channels,runtime-node"
```

Then forward **exactly** that list in `app/src-tauri/Cargo.toml`. The forwarding
gate asserts set equality in both directions — a gate in your product list but
not the shell's is silently compiled out of the shipped app with no build error.
That bug shipped upstream twice. `observed`.

**Phase D — rename.**
One commit per category from §3.1, with `cargo check` and `pnpm typecheck`
between each. Directory rename last.

**Phase E — verify.** See §5.

---

## 5. Verification

Measure it, don't feel it.

- **Offline turn.** Wifi off. Full agent turn against Ollama, including one tool
  call and one memory write. This is the single test that matters.
- **Egress audit.** Run a 10-minute session with Little Snitch, or
  `lsof -i -P | grep -i neppy` on a loop. Only localhost and whatever you
  explicitly allowlisted should appear. Repeat during a *cold* start, because the
  boot-time catalog refresh and module fetch only fire then.
- **Grep for survivors.**
  `rg 'tinyhumans\.ai|api\.tinyhumans|/auth/me|langfuse' src/ app/src`
- **Feature forwarding gate.** `node scripts/ci/check-feature-forwarding.mjs`
- **State matrix.** Fresh install, provider unreachable, provider returns garbage,
  offline mid-turn, two windows open. Upstream's own failure modes were all
  session-expiry-shaped, so specifically test: does a failed backend call still
  publish `SessionExpired` anywhere?
- **Gated-build smoke.** `GGML_NATIVE=OFF cargo test --lib --no-default-features core::`
  — upstream warns CI never runs the disabled-build test suite, so it rots.

---

## 6. Claude Code kickoff

Paste this at the start of a Claude Code session in `/Users/alex/Neppy`:

> Read `NEPPY-BUILD-SPEC.md` in the repo root. We are working through it in order:
> Phase A, then B, then C, then D, then E. Do not start the rename (Phase D)
> until Phase B's offline-turn test passes.
>
> Standing rules for this repo:
> - Never rename anything in §3.2 of the spec.
> - Every change to a Cargo feature gate must be made in both `Cargo.toml` and
>   `app/src-tauri/Cargo.toml`.
> - Run `GGML_NATIVE=OFF cargo check` before claiming a Rust change works.
> - When you touch the voice, web3 or mcp surface, also run
>   `--no-default-features` — it's the only thing that catches stub drift.
> - Keep `upstream` as a fetch-only remote so we can rebase onto their fixes.
>
> Start with Phase A and report what breaks.

---

## 7. Open questions

Things I could not settle from outside the code. Check these early.

1. Where exactly does `tinyagents` build its inference client, and is there a
   documented seam for injecting a base URL? If not, that's a vendored-crate
   patch and it changes the effort estimate.
2. Does the embedding dimension live in a migration, or is it inferred at write
   time? Decides whether swapping to `nomic-embed-text` is a config change or a
   reindex.
3. Which routes does the app actually hit on a cold boot when signed out? Capture
   them once against a local proxy — that list *is* the spec for your stub
   backend, and guessing it is how the stub ends up half-right.
4. Does the memory module load from `MODULE_PATH` with a self-built artifact, or
   does the digest check refuse it?
