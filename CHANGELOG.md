# Changelog

All notable changes to Neppy are recorded here. Each release also carries these notes on its GitHub Release page.

## [Unreleased]

## [0.66.3] - 2026-09-09

### What's Changed

- Redesigned LLM settings into an at-a-glance provider grid with direct connection toggles, Codex and Claude Code controls, custom providers, and routing on the same page.
- Opened desktop Settings as a large, self-contained window over the current page, with its own navigation, close button, Escape handling, and safe deep-link behavior.
- Added a maintained Git changelog and made releases commit their version and notes before tagging the exact release commit.
- Limited explicit release uploads to the signed Neppy updater bundle and signature. GitHub's automatic source snapshots remain available as part of every GitHub Release.

## [0.66.2] - 2026-09-09

### Changed

- Managed workload roles now route through the local runtime that is actually configured.

## [0.66.1] - 2026-09-09

### Fixed

- The Composio settings panel keeps the API-key field visible in direct/local mode.
