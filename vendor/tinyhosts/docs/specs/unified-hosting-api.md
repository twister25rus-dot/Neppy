# The unified hosting API

**Status:** accepted. Implemented for Vercel.

This is the standard TinyHosts holds every provider to: what a hosting provider
must be able to do for a Next.js application with a database behind it, said in
one vocabulary, so the provider is a configuration value rather than a rewrite.

## The problem it solves

Shipping an application is six things, not one:

1. a **site** to deploy to;
2. a **database** it can reach;
3. the **environment** it reads;
4. a **domain** in front of it;
5. a **deployment** that builds and serves it;
6. the **traffic** it served afterwards.

Providers expose different subsets of these six concerns, and no two name them
the same. Unsupported capabilities must return `Error::Unsupported`. A caller
that integrates against one provider's API has hard-coded not just its
endpoints but its shape — Vercel's "project" against Netlify's "site", Vercel's
marketplace store against Railway's first-party Postgres. The unified model is
the smallest vocabulary all of them can be said in.

## The model

| Concept | Type | What it is |
| --- | --- | --- |
| Site | `Site` / `SiteSpec` | A named application on an account. |
| Bundle | `Bundle` | The application's files, relative paths, contents in memory. |
| Deployment | `Deployment` | One build of a bundle, with a URL and a status. |
| Target | `DeploymentTarget` | `preview` or `production`. |
| Environment | `EnvVar` / `EnvVarRecord` | A name and, outbound only, a value. |
| Database | `DatabaseSpec` / `Database` | A managed store of a `DatabaseKind`. |
| Domain | `Domain` | A custom hostname on a site. |
| Analytics | `AnalyticsQuery` / `AnalyticsSummary` | Traffic over a window. |

Four rules hold the model honest:

- **A value never comes back.** `EnvVar` carries a value outbound; `EnvVarRecord`
  has none. A database reports `secret_keys` — the *names* of the variables the
  provider injects — never a connection string. If the model could return a
  secret, every caller and every log line would become a place one could leak.
- **A status this crate does not model is reported, not mapped.**
  `DeploymentStatus::Other` and `Framework::Other` carry the provider's own word.
  Mapping an unknown state onto a known one is how a poller decides a live site
  failed.
- **A missing capability is an error, not a no-op.** `Error::Unsupported` names
  the provider and the capability. A stub returning `Ok` for a database it never
  created is discovered by an application whose `DATABASE_URL` is missing.
- **A missing site is `Ok(None)`.** "Create it if it is not there" is the common
  path and must not be written by catching an error.

## The order a launch runs in

`launch` is the standard. The steps do not commute:

1. **Site** — created only if `find_site` does not find it, so a relaunch
   redeploys rather than failing on a name already taken.
2. **Database** — provisioned, then *connected to the site*. Connecting is what
   puts the connection variables into the site's environment.
3. **Environment** — after the database, so an explicit variable overrides an
   injected one rather than the reverse.
4. **Domains** — before the deployment, so a production build is aliased to them
   as it goes live.
5. **Deployment** — last. A Next.js build reads the environment at build time; a
   database attached afterwards is one the built pages cannot see.

A launch is not transactional. A database provisioned before a failing build
stays provisioned, because deleting and recreating it is the more expensive
mistake.

## Databases are a protocol, not a product

`DatabaseKind` names a protocol — `postgres`, `redis`, `blob` — and each kind
carries `product_hints`, the fragments a vendor's naming actually uses. A managed
Postgres is rarely called "postgres": it is Neon, Supabase, Prisma, Timescale.
Matching a kind to a product means matching against those names, on the product
slug, the product name, and the installation slug together.

`DatabaseSpec::product` pins an exact product and overrules the hints, for the
account that has several that would match.

## Vercel

| Concept | Endpoint |
| --- | --- |
| Create site | `POST /v11/projects` |
| Find / list sites | `GET /v9/projects/{idOrName}`, `GET /v10/projects` |
| Environment | `POST /v10/projects/{id}/env?upsert=true`, `GET /v10/projects/{id}/env` |
| Database: find product | `GET /v1/integrations/configurations?view=account`, then `GET /v1/integrations/configuration/{id}/products` |
| Database: provision | `POST /v1/storage/stores/integration/direct` |
| Database: attach | `POST /v1/integrations/installations/{icfg}/resources/{id}/connections` |
| Domains | `POST /v10/projects/{id}/domains`, `GET /v9/projects/{id}/domains` |
| Deploy | `POST /v2/files` per file, then `POST /v13/deployments` |
| Deployment status | `GET /v13/deployments/{id}`, `GET /v7/deployments` |
| Promote | `POST /v10/projects/{projectId}/promote/{deploymentId}` |
| Analytics | `GET /v1/query/web-analytics/visits/{count,aggregate}` |

Three details are Vercel's, and are the reason the adapter exists:

- **Deployment is upload-then-build, not Git.** Each file is uploaded to
  `POST /v2/files` keyed by its **SHA-1** digest (`x-vercel-digest` defines the
  algorithm), and the deployment references the digests. This is what makes a
  workspace with no repository behind it deployable.
- **A preview omits the target.** Vercel reads an absent `target` as a preview
  and rejects the literal string.
- **Databases are marketplace resources.** Vercel runs none itself, which is why
  provisioning is a three-request search-and-create rather than one call, and why
  `Error::NoDatabaseProduct` exists.

## Other providers

Suggestions for what to implement next, and where each one does not fit the model
cleanly. Nothing below is implemented.

### Netlify — closest fit

Sites, deploys, env vars and domains map almost one to one, and the deploy API is
the same shape: `POST /api/v1/sites/{id}/deploys` with a `files` map of
**SHA-1** digests, then upload the missing ones. Next.js runs through the Next
Runtime. **No first-party database** — a Postgres comes from Neon or Supabase
directly, so `provision_database` would either call those vendors' own APIs or
return `Unsupported`. Analytics are a paid add-on with a narrower API.

### Cloudflare (Workers / Pages) — best database story

`Workers` with `OpenNext` runs Next.js, and Cloudflare *does* run the databases:
D1 (SQLite), Hyperdrive (pooled Postgres), KV, R2 — all first-party REST APIs, so
`provision_database` is a single call and `attach_database` is a binding rather
than an environment variable. That is the mismatch worth knowing about: a binding
is not a `DATABASE_URL`, so `attach_database` would return binding names and the
application code has to be written for them. Deployment is a Wrangler-style
upload of a built worker, so the bundle would be build output rather than source.

### Railway — one provider, everything included

Postgres, MySQL, Redis and Mongo are first-party one-call provisions that *do*
produce a `DATABASE_URL`, which makes it the best fit for the model after Vercel.
The API is GraphQL rather than REST, so the adapter carries a query document
instead of paths. Deployment normally comes from a repository; the upload path
exists but is less travelled than Vercel's.

### Render — closest to a traditional host

First-party managed Postgres and Redis with connection strings, blueprint-driven
services, a straightforward REST API. Deploys are Git- or image-driven, so a
source bundle would need an intermediate repository or registry push — the one
place the model would have to grow.

### Fly.io — the escape hatch

Machines running a container, Fly Postgres or a Managed Postgres alongside.
Everything the model needs exists, but a deployment is a container image build and
push, not a file upload, so `Bundle` would have to mean "build context" and the
adapter would own a builder. Worth doing when someone needs a region or a runtime
the platforms above do not offer.

### AWS Amplify / Azure Static Web Apps — enterprise fit

Both host Next.js and both have managed databases nearby (RDS, Cosmos), but
neither pairs them: provisioning a database is a separate service with its own
IAM story. `provision_database` would be a substantially larger piece of work
than the rest of the adapter combined.

### Self-hosted — the one worth building second

A Docker or Kubernetes target where `deploy` builds an image and rolls it out, and
`provision_database` runs a Postgres container or claims one from an operator. It
is the only target that answers "what if the user does not want a hosting bill",
and implementing it is what would prove the model is not shaped around Vercel.

## Recommendation

Vercel first (done), then **Railway** — the same model with a genuinely
first-party database, which is the cheapest way to find out which parts of the
vocabulary are Vercel-shaped. Then **Cloudflare**, whose bindings are the model's
real stress test, then a **self-hosted** target.
