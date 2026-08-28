---
description: >-
  Run OpenHuman with no dependency on the hosted backend. Sign-in, model
  routing, embeddings, search and usage are served from a loopback service on
  your own machine.
icon: house-laptop
---

# Local Mode

[Privacy Mode](privacy-mode.md) answers "how much of my data may leave this device?". Local Mode answers a different question: **"which services does this app depend on?"**

They are orthogonal, and the difference is not academic. Privacy Mode's `local_only` deliberately lets the backend **control plane** through — sign-in, session refresh, team usage — because blocking those buys no privacy and only breaks the app. That exemption is exactly what kept a local-only install still talking to `api.tinyhumans.ai`. Local Mode closes it by *replacing* the control plane rather than restricting it.

| Privacy Mode | Local Mode | What you get |
| --- | --- | --- |
| `standard` | off | The default install. |
| `local_only` | off | Inference stays on-device; sign-in and usage still call the hosted backend. |
| `standard` | **on** | Self-hosted. No hosted backend — but a cloud LLM on **your own key** still works. |
| `local_only` | **on** | Fully offline. Nothing leaves the machine. |

The third row is the one most self-hosters want, and it is the one Privacy Mode alone could not express.

## Turning it on

Settings → Privacy → **Local Mode**, then restart. The panel lists every hosted service and what it becomes, so you can see the trade before you take it.

From a config file or a shell:

```toml
[local_mode]
enabled = true
```

```sh
OPENHUMAN_LOCAL_MODE=1 openhuman-core
```

The environment variable overrides in **both** directions — `OPENHUMAN_LOCAL_MODE=0` forces it off for a config that enables it. That is the escape hatch for the one hosted round-trip a local install occasionally needs (re-authenticating after migrating back), without editing `config.toml` to get it.

## How it is enforced

Every hosted round-trip in the core and the UI resolves its base URL through one function. Local Mode repoints that function at a loopback service the core starts at boot, so the rest of the app keeps working unchanged against a backend that happens to be local — the features and the UI are preserved by construction rather than by auditing several hundred call sites.

Two chokepoints back it up, in the same style as Privacy Mode's:

- The **egress spine** (`src/openhuman/security/egress/`) refuses any managed-backend round-trip. This is the mirror image of the privacy rule: where `local_only` exempts the control plane, Local Mode blocks precisely it.
- The **inference provider factory** refuses to build the managed provider at all, so a workload routed to it fails with a message naming the local runtimes rather than dialling a backend that is not there.

Third-party providers are untouched by both. Local Mode drops *our* backend, not your own vendor accounts.

## What replaces what

**Works locally, no setup:** sign-in and the session (a device-local identity), the memory tree and Obsidian mirror (always on-device), realtime events (the core's own bus), team usage (the on-device cost ledger), tracing (off by default, or point it at your own Langfuse).

**Works locally once you stand something up:** chat and reasoning via Ollama / LM Studio / vLLM, embeddings via Ollama, web search via your own [SearXNG](https://docs.searxng.org/) instance, speech via the bundled Whisper and Piper, messaging channels over their own APIs with your own bot tokens.

**No local equivalent** — these wrap a third-party SaaS whose service cannot be reproduced or proxied around: Composio's integration catalogue (use MCP servers, or your own Composio key in `direct` mode), the research vendors (Parallel, TinyFish, Apify), and the hosted image/video models (point `[media]` at a local ComfyUI or Automatic1111 for images).

**Not applicable:** billing, referrals and rewards, and tiny.place handles. There is nothing to bill and no network to hold an identity on.

A hosted route with no local implementation answers `501` with the local alternative attached, rather than a fabricated empty success — an agent that receives invented search results acts on them.

The full table, with the exact setup command for each row, is in [`docs/LOCAL_MODE.md`](https://github.com/tinyhumansai/openhuman/blob/main/docs/LOCAL_MODE.md).

## Limitations

- Turning it on or off needs a restart. Enforcement applies immediately; binding the loopback listener is boot work.
- Cross-device sync is gone, not replaced — it needs a server in the middle by definition.

## See also

- [Privacy Mode](privacy-mode.md): the data-egress posture, and the switch to pair this with.
- [Local AI](model-routing/local-ai.md): the on-device model runtimes Local Mode routes to.
- [Use OpenHuman with a local model](../guides/local-model.md): the full local setup walkthrough.
