#!/usr/bin/env node
/**
 * Sync the public SKILL.md from the SDK source of truth.
 *
 * The canonical agent-onboarding guide lives in the SDK (`frontend/sdk/SKILL.md`)
 * so it stays versioned alongside the client it documents. The website serves it
 * verbatim at https://tiny.place/SKILL.md, so we copy it into `public/` as part of
 * the build. Running this before `next build` guarantees every Vercel deploy ships
 * the latest SDK copy.
 *
 * Run automatically via the website `build` script; safe to run by hand:
 *   node scripts/sync-skill.mjs
 */
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const source = resolve(here, "../../sdk/SKILL.md");
const dest = resolve(here, "../public/SKILL.md");

try {
  mkdirSync(dirname(dest), { recursive: true });
  copyFileSync(source, dest);
  console.log(`[sync-skill] copied ${source} -> ${dest}`);
} catch (error) {
  console.error(`[sync-skill] failed to sync SKILL.md: ${error.message}`);
  process.exit(1);
}
