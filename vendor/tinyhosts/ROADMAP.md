# Roadmap

What exists, what is next, and what is deliberately out of scope.

## Shipped

- The unified hosting model: `Host`, its vocabulary, `Bundle`, and `launch`.
- The Vercel adapter: projects, non-Git deployments, environment variables,
  marketplace databases, domains, promotion, and web analytics.
- `rpc`: one JSON request in, one JSON result out, and the TinyBus module over
  it.

## Next

- A second provider, to find the Vercel-shaped assumptions in the vocabulary.
  Railway first — the same model with a first-party database — then Cloudflare,
  whose bindings are the model's real stress test. See
  [`docs/specs/unified-hosting-api.md`](docs/specs/unified-hosting-api.md).
- A self-hosted target, which is what proves the model is not shaped around a
  platform.
- Deployment log streaming: a failed build currently reports a message, not the
  build output that explains it.

## Out Of Scope

- Waiting, retrying, or scheduling. How long a caller will wait for a build is
  the caller's policy, and a hidden one cannot be cancelled or reported on.
- Holding a connection string, or any secret the provider injects itself.
- Wrapping a provider's whole API. The model covers shipping and running an
  application; anything beyond that is reached through the provider's own client.
