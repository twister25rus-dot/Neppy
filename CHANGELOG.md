# Changelog

All notable changes to Neppy are recorded here. Each release also carries these notes on its GitHub Release page.

## [Unreleased]

## [0.66.5] - 2026-09-11

### What's Changed

- Give the release build a target directory of its own ([`f5e72c39`](https://github.com/twister25rus-dot/Neppy/commit/f5e72c39dd76905a26bd4aa2ea73e9198f1e1335))
- Fit the settings window on screen, and give connector pages a measure ([`8d1a75b2`](https://github.com/twister25rus-dot/Neppy/commit/8d1a75b2c0dc78f0169af03b0a8cd432e1b97cd3))
- Let a turn ask for sampling and reasoning effort ([`2b647cee`](https://github.com/twister25rus-dot/Neppy/commit/2b647cee88eb5a98c19f907b115a23f9f2a21ad6))
- Keep every answer to a question, and let one of them be in effect ([`620bd6e7`](https://github.com/twister25rus-dot/Neppy/commit/620bd6e7fa8c03003c686cb56b1824cc7f0594ff))
- Regenerate an answer without losing the one before it ([`ccd0a2af`](https://github.com/twister25rus-dot/Neppy/commit/ccd0a2afae4bf89a464bb994508b03c692eae5bb))
- Switch between a question's answers ([`70fc32a9`](https://github.com/twister25rus-dot/Neppy/commit/70fc32a9bd8ad0c252b5de55ba998f681ed7d469))
- Set sampling for a turn from the composer ([`0bde427e`](https://github.com/twister25rus-dot/Neppy/commit/0bde427e154c7e3f1874a18cd82d19eb891e1c03))
- Format the chat and settings work to house style ([`5666a3ac`](https://github.com/twister25rus-dot/Neppy/commit/5666a3ac32f8d855d4ec5e68cc1e0be6dd35351b))
- Ignore the release target dir where the rule actually applies ([`ddfe70ac`](https://github.com/twister25rus-dot/Neppy/commit/ddfe70ac3aff9ee5db4a402d872bf5411142eda2))

**Full Changelog:** https://github.com/twister25rus-dot/Neppy/compare/6b9f9491feef65bbcc6ff2a5e8773f66d086fb26...ddfe70ac3aff9ee5db4a402d872bf5411142eda2

## [0.66.4] - 2026-09-09

### What's Changed

- Chat reaches the MLX server the app manages, and keeps the model you picked ([`4cc609fe`](https://github.com/twister25rus-dot/Neppy/commit/4cc609fee7f22f664fae9010e3fba53ea3d4da72))

**Full Changelog:** https://github.com/twister25rus-dot/Neppy/compare/6e6407fa6f12c7bd12202973e3be711054a0787c...4cc609fee7f22f664fae9010e3fba53ea3d4da72

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
