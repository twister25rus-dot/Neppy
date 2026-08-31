#!/usr/bin/env node
// Thin executable entry. All CLI logic lives in the `./cli/` module so it can be
// split into richer tools over time; this file only wires the bin + re-exports.
import { realpathSync } from "node:fs";
import { basename } from "node:path";
import { fileURLToPath } from "node:url";

import { runTinyPlaceCli } from "./cli/index.js";

export { runTinyPlaceCli } from "./cli/index.js";
export { CLI_GUIDES, HARNESS_CLI_COMMANDS } from "./cli/commands.js";
export type {
  TinyPlaceCliCommand,
  TinyPlaceCliGuide,
  TinyPlaceCliOptions,
  TinyPlaceCliResult,
} from "./cli/types.js";

export function normalizeTinyVerseArgv(
  argv: Array<string>,
  binName: string | undefined,
): Array<string> {
  if (basename(binName ?? "") !== "tinyverse") {
    return argv;
  }
  const [command, ...rest] = argv;
  if (command === "codex" || command === "claude") {
    return ["tui", command, ...rest];
  }
  return argv;
}

// Run the CLI only when this file is the process entry point. We compare the
// real (symlink-resolved) path of argv[1] against this module's own path so the
// check holds whether invoked as `node dist/cli.js`, via a globally-installed
// `bin` symlink (`npm install -g` points bin/tinyplace -> dist/cli.js), or via
// `npm link` — and stays false when the file is merely imported as a module.
function isCliEntryPoint(): boolean {
  const entry = process.argv[1];
  if (typeof process === "undefined" || !entry) {
    return false;
  }
  try {
    return realpathSync(entry) === fileURLToPath(import.meta.url);
  } catch {
    return false;
  }
}

if (isCliEntryPoint()) {
  runTinyPlaceCli(normalizeTinyVerseArgv(process.argv.slice(2), process.argv[1])).then((result) => {
    if (result.stdout) {
      process.stdout.write(result.stdout);
    }
    if (result.stderr) {
      process.stderr.write(result.stderr);
    }
    process.exit(result.code);
  });
}
