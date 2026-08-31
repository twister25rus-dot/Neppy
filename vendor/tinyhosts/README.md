# TinyHosts

One API for putting a Next.js application — and the database behind it — on a
real hosting provider.

TinyHosts is the hosting category of the TinyHumans stack. OpenHuman vendors it,
OpenCompany inherits it from there, and a user who pastes a provider API key into
OpenCompany gets a live site out of a workspace. It ships both as an ordinary
Rust library and as an installable TinyBus module.

Vercel is the first provider. Adding the next one means implementing one trait.

## What it does

The unit of work is the whole thing, because that is what "host this" means:

| Step | Method | On Vercel |
| --- | --- | --- |
| Site | `create_site`, `find_site`, `list_sites` | a project |
| Database | `provision_database`, `attach_database` | a marketplace store, connected to the project |
| Environment | `set_env`, `list_env` | project environment variables |
| Domain | `add_domain`, `list_domains` | a project domain |
| Deployment | `deploy`, `deployment`, `list_deployments`, `promote` | file upload, then a build |
| Traffic | `analytics` | the web analytics query API |

[`launch`](src/launch/mod.rs) runs the five hosting steps in the one order that
works — the database is connected *before* the build, because a Next.js build
reads its environment at build time.

## Using it

```rust
use tinyhosts::{Bundle, DatabaseSpec, LaunchPlan, ProviderKind, SiteSpec, launch};

let host = tinyhosts::connect_from_env(ProviderKind::Vercel)?;

let plan = LaunchPlan::new(SiteSpec::new("shop"), Bundle::from_dir("./shop")?)
    .with_database(DatabaseSpec::new("shop-db"))
    .into_production();

let result = launch(host.as_ref(), &plan).await?;
println!("building at {:?}", result.url());
```

`launch` returns while the build is still running. Poll `Host::deployment` until
its status `is_terminal()`; how long to wait is the caller's policy, so this
crate owns no timer.

Credentials come from `TINYHOSTS_VERCEL_TOKEN`, falling back to `VERCEL_TOKEN`,
with `TINYHOSTS_VERCEL_TEAM_ID` / `VERCEL_TEAM_ID` for a team account. See
[`.env.example`](.env.example). A credential can equally be passed in from a
form, which is what OpenCompany does.

### From another process

`tinyhosts::execute_json` is the same surface as one JSON request and one JSON
result, and the TinyBus `Execute` method is a thin wrapper over it:

```json
{
  "provider": "vercel",
  "credentials": { "api_key": "...", "team": null },
  "operation": "launch",
  "plan": {
    "site": { "name": "shop" },
    "bundle": [{ "path": "package.json", "contents": "e30=" }],
    "database": { "name": "shop-db", "kind": "postgres" },
    "target": "production"
  }
}
```

Bundle file contents are base64. Results are `{"result": "...", "value": ...}`.

## What it will not do

- **Hold a secret.** A database's connection string is injected by the provider
  into the site's environment; this crate only ever learns the *names* of the
  variables. `Credentials` has no `Serialize` and a redacting `Debug`.
- **Pretend.** A capability a provider lacks is an `Unsupported` error naming the
  provider and the capability, never a silent success.
- **Wait, retry, or schedule.** Those are the caller's policy.

## Databases

Vercel does not run databases; its marketplace partners do. `provision_database`
therefore searches the installed integrations on the account for a product that
serves the requested kind — `postgres` matches Neon, Supabase, Prisma Postgres
and friends — creates a store from it, and connects it to the project. If nothing
on the account can serve the kind, the error says exactly that rather than
failing later with a missing `DATABASE_URL`.

Pin a specific product with `DatabaseSpec::with_product` when an account has more
than one that would match.

## Features

`vercel` (default) is the provider. `module` (default) is the TinyBus module —
the bus interface, the ABI exports and the `cdylib` a TinyBus host loads. A
downstream that links the library directly takes
`default-features = false, features = ["vercel"]` and gets no TinyBus in its
graph, which is what OpenHuman does: it vendors its own TinyBus, and two path
copies of one package cannot both be written to a lockfile.

## Adding a provider

Implement `Host`, add a `ProviderKind` variant, and wire it into `connect_to`.
[`docs/specs/unified-hosting-api.md`](docs/specs/unified-hosting-api.md) maps the
model onto Netlify, Cloudflare, Railway, Render, Fly.io and a self-hosted target,
and says where each one does not fit.

## Layout

```text
src/
├── lib.rs              # crate docs + the public re-export surface
├── error/              # crate-wide `Error` and `Result<T>`
├── credentials/        # the API key, redacted and write-only
├── host/               # the `Host` trait and the unified vocabulary
│   ├── mod.rs
│   └── types.rs
├── bundle/             # an application's files, and reading them off disk
├── launch/             # the whole flow, in the order that works
├── providers/
│   ├── mod.rs          # `ProviderKind`, `connect`, `connect_to`
│   └── vercel/         # the Vercel adapter: `mod.rs`, `http.rs`, `wire.rs`
├── rpc/                # one JSON request in, one JSON result out
└── tinybus_module/     # bus interface, setup, and ABI v1 exports
tests/public_api.rs     # integration tests against the public API only
examples/               # runnable, compiled-in-CI usage examples
vendor/tinybus/         # pinned TinyBus git submodule
docs/{specs,plans,adr}/
```

## Development

Clone with submodules, or initialize them before building:

```sh
git submodule update --init --recursive
```

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
```

Those four are exactly what CI runs. Optional extras:

```sh
cargo run --example basic            # build a launch plan, send nothing
cargo doc --no-deps --all-features   # CI builds this with RUSTDOCFLAGS="-D warnings"
cargo deny check all                 # supply-chain check; see deny.toml
cargo install cargo-llvm-cov         # once, before running the coverage gate
.github/scripts/check-file-coverage.sh 90 coverage.json
```

The provider tests run the real adapter against a local mock of the REST API, so
the suite is offline, deterministic, and needs no token.

## Releasing

Run the **Release** workflow from the Actions tab with a `patch`, `minor`, or
`major` bump. Use `current` only to resume an interrupted release whose version
commit and tag already exist. The workflow revalidates the crate, versions and
tags it, builds this crate as a TinyBus `cdylib`, and creates a GitHub release.
Assets follow `tinyhosts-<version>-<platform>.<tar.gz|zip>` and contain the
native module, its SHA-256 `modules.toml`, license, and
[`MODULE.md`](MODULE.md). Every release also publishes `checksum.toml`, which
TinyBus uses to verify an archive before extraction. The workflow loads the
published Ubuntu archive through TinyBus's GitHub release API and calls its
`Providers` method before declaring the release successful. TinyBus itself is not
shipped by this repository; the pinned submodule is the build-time SDK. Do not
hand-edit the version in `Cargo.toml`.

## Documentation

- [`AGENTS.md`](AGENTS.md) — repository guidelines for humans and agents
- [`docs/specs/unified-hosting-api.md`](docs/specs/unified-hosting-api.md) — the
  model, and how it maps onto other providers
- [`docs/adr/`](docs/adr/0001-record-architecture-decisions.md) — architecture
  decision records
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to propose a change
- [`SECURITY.md`](SECURITY.md) — how to report a vulnerability

## License

GPL-3.0-only. See [LICENSE](LICENSE).
