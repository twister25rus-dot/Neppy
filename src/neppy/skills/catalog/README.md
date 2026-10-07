# Skill Registry

`skill_registry` owns remote skill catalogs and installed-skill lifecycle.

Responsibilities:

- Fetch and cache registry catalogs.
- Refresh the remote catalog asynchronously on core load.
- Browse/search registry entries.
- Derive install URLs for Hermes bundled and optional skills.
- Install catalog entries into the user skills directory.
- Uninstall user-scope skills.
- Host the built-in `skill_setup` agent.

Default catalog:

```text
https://hermes-agent.nousresearch.com/docs/api/skills.json
```

Useful environment overrides for prod scripts and deterministic tests:

```bash
NEPPY_SKILL_REGISTRY_CATALOG_URL=https://example.com/skills.json
NEPPY_SKILL_REGISTRY_DOWNLOAD_BASE_URL=https://example.com/skills
NEPPY_SKILL_REGISTRY_REFRESH_ON_BOOT=0
```

`NEPPY_SKILL_REGISTRY_REFRESH_ON_BOOT=0` disables the best-effort startup
refresh. By default, core startup spawns a background task that force-refreshes
the remote catalog and updates the local cache without blocking core readiness.

Production smoke examples:

```bash
neppy skill_registry schemas
neppy skill_registry browse --force_refresh true
neppy skill_registry search --query git
neppy skill_registry sources
neppy skill_registry install --entry_id git-helper
neppy skill_registry uninstall --name git-helper
```

Security notes:

- Production installs still go through the hardened `workflows` URL installer.
- HTTP localhost installs require `NEPPY_SKILL_INSTALL_ALLOW_LOCAL_HTTP=1` and are intended for local fixtures only.
