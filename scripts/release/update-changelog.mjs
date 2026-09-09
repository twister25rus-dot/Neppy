#!/usr/bin/env node
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const DEFAULT_PREAMBLE = `# Changelog

All notable changes to Neppy are recorded here.

## [Unreleased]
`;

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function normalizeNotesForChangelog(notes) {
  const body = String(notes || "").trim();
  if (!body) {
    throw new Error("Release notes are empty");
  }

  // A version entry is an H2, so release-note headings must sit below it.
  return body.replace(/^(#{1,5})(?=\s)/gm, "$1#");
}

export function updateChangelog({ current, version, date, notes }) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) {
    throw new Error(`Version must be X.Y.Z, got "${version}"`);
  }
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) {
    throw new Error(`Date must be YYYY-MM-DD, got "${date}"`);
  }

  const normalizedNotes = normalizeNotesForChangelog(notes);
  const heading = `## [${version}] - ${date}`;
  const entry = `${heading}\n\n${normalizedNotes}`;
  let changelog = String(current || "").trimEnd();
  if (!changelog) {
    changelog = DEFAULT_PREAMBLE.trimEnd();
  }

  // Make reruns safe: replace an existing entry for this version instead of
  // duplicating it. The next version heading (or EOF) bounds the entry.
  const existingEntry = new RegExp(
    `(?:\\n|^)## \\[${escapeRegExp(version)}\\] - \\d{4}-\\d{2}-\\d{2}\\n[\\s\\S]*?(?=\\n## \\[|$)`,
  );
  changelog = changelog.replace(existingEntry, "").trimEnd();

  const unreleased = /^## \[Unreleased\][^\n]*$/m;
  const match = unreleased.exec(changelog);
  if (match) {
    const insertAt = match.index + match[0].length;
    const remainder = changelog.slice(insertAt).trimStart();
    return `${changelog.slice(0, insertAt)}\n\n${entry}${remainder ? `\n\n${remainder}` : ""}\n`;
  }

  const firstVersion = /^## \[\d+\.\d+\.\d+\][^\n]*$/m.exec(changelog);
  if (firstVersion) {
    return `${changelog.slice(0, firstVersion.index).trimEnd()}\n\n${entry}\n\n${changelog.slice(firstVersion.index)}\n`;
  }

  return `${changelog}\n\n${entry}\n`;
}

function parseArgs(argv) {
  const options = {
    version: "",
    date: "",
    notesFile: "",
    changelog: "CHANGELOG.md",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!value || value.startsWith("--")) {
      throw new Error(`${key} requires a value`);
    }
    index += 1;
    if (key === "--version") options.version = value;
    else if (key === "--date") options.date = value;
    else if (key === "--notes-file") options.notesFile = value;
    else if (key === "--changelog") options.changelog = value;
    else throw new Error(`Unknown option: ${key}`);
  }
  if (!options.version || !options.date || !options.notesFile) {
    throw new Error("--version, --date, and --notes-file are required");
  }
  return options;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  let current = "";
  try {
    current = readFileSync(options.changelog, "utf8");
  } catch (error) {
    if (error?.code !== "ENOENT") throw error;
  }
  const notes = readFileSync(options.notesFile, "utf8");
  const updated = updateChangelog({
    current,
    version: options.version,
    date: options.date,
    notes,
  });
  writeFileSync(options.changelog, updated);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(`[changelog] ${error.message}`);
    process.exit(1);
  }
}
