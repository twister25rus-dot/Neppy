# 2. A provider-agnostic hosting model, with credentials that never come back

- **Status:** Accepted
- **Date:** 2026-08-18

## Context

TinyHosts exists so a workspace can become a live website. The obvious
implementation is a Vercel client: OpenCompany takes a token, this crate calls
`api.vercel.com`, a site appears. That implementation is also the one that has to
be thrown away the first time somebody wants Cloudflare, or a self-hosted box, or
a database Vercel does not sell.

Two further facts shaped the design. Hosting is not one call — a working Next.js
application needs a site, a database wired into it, an environment, a domain and
a build, in that order. And the credential is a user's, pasted into a form,
crossing a process boundary on its way here.

## Decision

**One trait, `Host`, is the whole contract**, spoken in a provider-independent
vocabulary; `ProviderKind` plus `connect_to` makes the provider a configuration
value. Vercel is an adapter behind it, not the API.

**`launch` owns the order**, because the order is the part that is easy to get
wrong and impossible to debug: a database attached after the build is invisible
to the built pages.

**Secrets travel one way.** `Credentials` implements `Deserialize` but not
`Serialize`, and renders as `<redacted>`. `EnvVar` carries a value; the record
returned by a list call does not. A `Database` reports the *names* of the
variables the provider injects and never a connection string — the values are
injected provider-side, so this crate has no reason to hold one and therefore no
way to leak one.

**A capability a provider lacks is an error.** `Error::Unsupported` names the
provider and the capability rather than returning `Ok`.

**A state the model does not know is carried through.** `DeploymentStatus::Other`
and `Framework::Other` keep the provider's own word rather than being mapped onto
the nearest known value.

## Consequences

- A second provider is a new module and a `ProviderKind` variant. Nothing above
  the trait changes. `docs/specs/unified-hosting-api.md` maps six candidates.
- The model is deliberately narrow: it covers shipping and running a Next.js
  application and nothing else. Anything further is reached through a provider's
  own client.
- Vercel-shaped assumptions may still be hiding in the vocabulary. The cheapest
  way to find them is a second adapter over a provider with first-party
  databases; Cloudflare's *bindings* are the known stress test, because a binding
  is not an environment variable.
- `attach_database` returning names rather than values means a caller cannot run
  a migration from the connection string it just created. That is the intended
  trade: the provider injects it, the application reads it, and nothing in
  between has to be trusted with it.
- A launch is not transactional, so a failed build can leave a paid database
  behind. Deleting and recreating it on every retry was judged the worse failure.
