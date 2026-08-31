# TinyHosts TinyBus Module

This package contains the native `tinyhosts` module for TinyBus module ABI v1.
Install only the archive matching the host operating system and architecture.

The module claims `ai.tinyhumans.tinyhosts.Hosting`, serves the object at
`/ai/tinyhumans/tinyhosts/Hosting`, and provides two methods:

- **`Execute`** takes one JSON hosting request and returns one JSON result. The
  request names a provider, carries the account's API key, and names an
  operation — `launch`, `deploy`, `provision_database`, `set_env`, `analytics`
  and the rest of the `Host` surface. A request without a credential falls back
  to the environment. See the repository README for the envelope.
- **`Providers`** takes nothing and returns the provider slugs this build can
  connect to, as a JSON array.

The archive contains one `.so`, `.dylib`, or `.dll` plus `modules.toml`. Keep
those files together when copying them into a TinyBus module directory. The
allowlist binds the native library filename to its SHA-256 digest so TinyBus can
reject a missing, renamed, or modified artifact before initialization.

The GitHub release also publishes `checksum.toml` as a separate asset. TinyBus
checks that manifest before downloading and extracting the selected platform
archive. Install directly from a tagged release with:

```sh
tinybus modules load-github \
  https://github.com/tinyhumansai/tinyhosts/releases/tag/v0.1.5 \
  tinyhosts-0.1.5-ubuntu-24.04-x86_64.tar.gz \
  <archive-sha256>
```

TinyBus modules are trusted in-process code, and this one is handed live hosting
credentials by its callers. Install release artifacts only from a trusted source,
and restart the host after replacing a loaded module.
