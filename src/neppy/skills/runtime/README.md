# Skill Runtime

`skill_runtime` owns execution of installed `SKILL.md` workflows.

Responsibilities:

- Start and cancel skill runs.
- Read recent run metadata and run logs.
- Resolve reusable language runtimes before script-backed skills run.
- Host the built-in `skill_executor` agent.

It deliberately reuses:

- `runtime_node` for Node.js, npm, npx, and PATH injection.
- `runtime_python` for Python interpreter resolution and process launching.
- `workflows` for installed skill discovery, metadata, resources, and run logs.

Production smoke examples:

```bash
neppy skill_runtime schemas
neppy skill_runtime resolve_runtimes --runtime all
neppy skill_runtime run --skill_id git-helper --inputs '{}'
neppy skill_runtime recent_runs --limit 10
```

Compatibility:

- Existing `neppy workflows run`, `workflows cancel`, and run-log RPCs remain available.
- New scripts should prefer the `skill_runtime` namespace for execution.

