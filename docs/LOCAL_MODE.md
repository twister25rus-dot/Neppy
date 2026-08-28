# Local Mode

Run OpenHuman with no dependency on the project's hosted backend
(`api.tinyhumans.ai`).

```toml
# ~/.openhuman/config.toml
[local_mode]
enabled = true
```

or, without editing config:

```sh
OPENHUMAN_LOCAL_MODE=1 openhuman-core
```

Then restart the core. Settings → Privacy shows the switch and the full
inventory of what each hosted service becomes.

---

## What it is, and what it is not

Local Mode is about **service topology**: which services the app depends on.
It is *not* Privacy Mode, which is about **data egress**: how much of your data
may leave the device. The two are orthogonal and compose:

| `privacy.mode` | `local_mode.enabled` | Result |
| --- | --- | --- |
| `standard` | `false` | Default install. The hosted backend serves auth, model routing and search. |
| `local_only` | `false` | Inference stays on-device, but sign-in and usage still call the hosted backend. |
| `standard` | `true` | Self-hosted install. No hosted backend; a cloud LLM on **your** key is still allowed. |
| `local_only` | `true` | Fully offline. Nothing leaves the machine. |

The third row is the one people usually want and the one Privacy Mode alone
could not express. `PrivacyMode::LocalOnly` deliberately exempts the backend
**control plane** — blocking sign-in buys no privacy — and that exemption is
exactly what kept a local install tied to `api.tinyhumans.ai`. Local Mode
closes it by *replacing* the control plane rather than restricting it.

---

## How it works

Every hosted round-trip in the core and the renderer funnels through one
function: `api::config::effective_backend_api_url`. Local Mode repoints that
function at a loopback service the core starts at boot
(`src/openhuman/local_mode/backend/`), so the rest of the app keeps working
unchanged against a backend that happens to be local.

```
                     effective_backend_api_url()
                                │
              local mode? ──────┴────── no ──► https://api.tinyhumans.ai
                    │
                   yes
                    │
                    ▼
        http://127.0.0.1:43117   (loopback only, bearer-guarded)
                    │
   ┌────────────────┼────────────────────────────┐
   ▼                ▼                            ▼
/auth/*         /openai/v1/*              everything else
device session  → local runtime           → 501 local_mode_unsupported
/teams/*        (Ollama, LM Studio…)         + the local alternative
on-device ledger
```

Two enforcement chokepoints back it up, so a call cannot escape to the hosted
backend even if some future code path forgets to resolve its URL through the
resolver:

- **`security::egress::local_mode_blocks`** refuses any managed-backend
  round-trip. This is the mirror image of the privacy-mode rule: where
  `LocalOnly` exempts the control plane, Local Mode blocks precisely it.
- **`inference::provider::factory::enforce_local_mode_inference`** refuses to
  construct the managed (`openhuman`) chat provider, which also cuts the
  recursion that would otherwise occur — a managed call would dial the local
  backend, whose handler resolves a provider, which would be managed again.

Third-party providers are untouched by both. Local Mode drops *our* backend, not
your own vendor accounts.

---

## What replaces what

The table below is generated from the same inventory the app and the local
backend use (`src/openhuman/local_mode/services.rs`); a test asserts this file
lists every entry, so it cannot drift.

### Replaced — works locally, no setup

| Hosted | Locally |
| --- | --- |
| **`auth.session`** — hosted sign-in and the session token | A device-local session issued by the local backend and stored in the OS keyring. The app is single-user on this machine, so there is no account to sign in to. |
| **`memory.storage`** — nothing; the memory tree was always on-device | Unchanged. SQLite under the workspace directory, mirrored to the Obsidian vault. |
| **`telemetry.tracing`** — Langfuse ingestion proxied through the backend | Off by default. Run counts and per-call costs are still recorded locally and replayable in the UI; point agent tracing at your own Langfuse if you want traces off-device. |
| **`sync.realtime`** — Socket.IO push from the backend | The core's own event bus over loopback Socket.IO and SSE, which the desktop UI already consumes. Cross-*device* sync has no local equivalent — there is no server in the middle. |
| **`account.usage`** — team membership and metered usage | A single-member local team backed by the on-device cost ledger, so per-model spend still shows up for your own provider keys. |

### Needs setup — works locally once you stand something up

| Hosted | Locally | Setup |
| --- | --- | --- |
| **`inference.chat`** — managed model routing | The local backend forwards `/openai/v1/*` to the runtime configured under `[local_ai]`. | `ollama pull llama3.1`, or point `inference_url` at your own provider key. |
| **`inference.embeddings`** — managed embeddings (Voyage) | The `ollama` embedding provider, selected automatically. Memory search falls back to keyword-only if no model is available, so the memory tree still works. | `ollama pull nomic-embed-text` |
| **`search.web`** — managed web search (Exa) | The `searxng` engine: `web_search_tool` is served by your own SearXNG instance. | `docker run -p 8080:8080 searxng/searxng`, then set `[searxng] enabled = true` and `base_url`. |
| **`voice.speech`** — hosted realtime voice and transcription proxy | In-process Whisper for speech-to-text and Piper for speech output, both already bundled. | Download a Whisper model in Settings → Voice; set `PIPER_BIN`. |
| **`channels.messaging`** — backend relay for messaging channels | Direct connections. Telegram, Discord, Slack, IRC, Matrix and email all speak to their own servers from your machine with your own bot token — the relay was a convenience, never a requirement. | Paste each channel's bot token in Settings → Channels. |

### No local equivalent

These are thin wrappers over a third-party SaaS. We may not reimplement their
service, proxy around their billing, or redistribute their model weights, so
Local Mode does not pretend they work — it names what the app offers instead
and leaves the feature off.

| Hosted | Why not | Instead |
| --- | --- | --- |
| **`integrations.composio`** — 100+ OAuth integrations | Composio is a third-party SaaS; its catalogue and OAuth broker cannot be run locally. | MCP servers, which cover the same ground with your own credentials. If you have your own Composio account, set the integration mode to `direct` and it calls Composio with your key instead of ours. |
| **`integrations.research`** — Parallel, TinyFish, Apify | Each is a metered third-party API. | The built-in browser and fetch tools, plus any MCP server pointed at these vendors with your own key. |
| **`media.generation`** — Seedream/SeedEdit, Seedance, Veo | Proprietary weights behind a hosted API. | A local OpenAI-compatible image endpoint (ComfyUI, Automatic1111, or anything exposing `/v1/images/generations`) under `[media]`. The hosted video models have no local equivalent. |

### Not applicable

| Hosted | Why |
| --- | --- |
| **`account.billing`** — subscription, credits, top-ups | Nothing to bill. You pay your model provider directly, or nothing at all when everything runs on local weights. |
| **`account.growth`** — referrals, invites, rewards, announcements | All four are properties of the hosted account system. |
| **`agents.marketplace`** — tiny.place handles, agent-to-agent, x402 bounties | A handle is an identity on someone else's network. Local subagents and fleets are unaffected. |

---

## Settings

```toml
[local_mode]
# Run without the project's hosted backend. Default false.
enabled = true

# Loopback port for the local backend. 0 asks the OS for an ephemeral port.
# Deliberately not 11434 / 8000 / 8080 / 1234 / 8888 — a loopback URL on one of
# those is classified as a local *model runner* by `api::config` and would be
# skipped as a backend override, silently undoing local mode.
backend_port = 43117

# Re-resolve managed provider defaults to local ones (embeddings → Ollama,
# search → SearXNG). Providers you chose yourself are never rewritten.
# Turn off to keep the topology change but pick every provider by hand.
apply_local_defaults = true

# Serve /openai/v1/* by forwarding to the local model runtime. Turn off when
# `inference_url` already points somewhere the app should call directly.
proxy_inference = true
```

Environment overrides:

| Variable | Effect |
| --- | --- |
| `OPENHUMAN_LOCAL_MODE` | `1`/`true`/`yes`/`on` forces Local Mode on; `0`/`false` forces it **off** for a config that enables it. Unset or empty defers to config. |
| `OPENHUMAN_LOCAL_BACKEND_PORT` | Overrides `backend_port`. |

The env var overrides in both directions on purpose: it is the escape hatch when
a local install needs one hosted round-trip to recover — re-authenticating
against the hosted backend after migrating back, say — without editing
`config.toml` to get it.

---

## Errors you will see

A hosted route with no local implementation answers `501 Not Implemented` with a
machine-readable body:

```json
{
  "code": "local_mode_unsupported",
  "error": "`100+ OAuth integrations brokered through the backend's Composio account` is served by the hosted backend, which local mode does not use. Use the local alternative instead.",
  "path": "/agent-integrations/composio/execute",
  "service_id": "integrations.composio",
  "local_alternative": "MCP servers, which cover the same ground and run on your machine with your own credentials. …"
}
```

`501` rather than `404` because the route exists and the *service behind it*
does not, and because clients treat `404` as retryable and `501` as terminal.

It is never a plausible-looking empty success. A `200 {"results": []}` for a
search route would be a lie the agent cannot detect: an empty result set is
indistinguishable from "the web had nothing", so the model would conclude the
latter and reason on.

---

## Limitations

- **Turning Local Mode on or off needs a restart.** Enforcement (refusing
  hosted round-trips) applies immediately, but binding or releasing the loopback
  listener is boot work — it competes for a port and can fail, and a
  half-applied switch would leave the app unable to reach either control plane.
  The settings panel says so via `restartRequired`.
- **Cross-device sync is gone**, not replaced. It requires a server in the
  middle by definition.
- **A slim build without the `http-server` feature** has no listener to bind.
  The policy still applies — hosted round-trips are refused — but nothing serves
  the local control plane, and the core logs a warning at boot saying so.
- **The local session token is not signed, and is not the security boundary.**
  It is a JWT-shaped triple whose signature segment is the literal string
  `local`. There is nothing for a signature to prove — the issuer and the
  verifier are the same process on the same machine — and the sentinel is what
  makes it impossible for a verifier to mistake it for a signature it should
  have checked.

  What actually protects the local backend is the pair of checks in its request
  guard: the listener binds loopback only, and every request's `Origin` is
  checked against the same allow-list the core's RPC server uses (the Tauri
  webview, loopback, and anything an operator opted in through
  `OPENHUMAN_CORE_ALLOWED_ORIGINS`). That is what stops a page on a site you
  visited from POSTing to the inference proxy and spending your provider key —
  CORS alone would not, because such a POST does its damage on the way in. The
  bearer arm accepts the per-launch core RPC token (a real secret, compared in
  constant time) or any device-local token (a shape check, since the renderer
  mints its own and the format is in this repository).
