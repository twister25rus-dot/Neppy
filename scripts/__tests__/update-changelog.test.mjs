import assert from "node:assert/strict";
import { test } from "node:test";

import {
  normalizeNotesForChangelog,
  updateChangelog,
} from "../release/update-changelog.mjs";

test("release-note headings are nested below the version heading", () => {
  assert.equal(
    normalizeNotesForChangelog("# Headline\n\n## Changes\n\n### Detail"),
    "## Headline\n\n### Changes\n\n#### Detail",
  );
});

test("a release is inserted below Unreleased and above older versions", () => {
  const current = `# Changelog

## [Unreleased]

## [0.66.2] - 2026-09-09

Old notes.
`;
  const updated = updateChangelog({
    current,
    version: "0.66.3",
    date: "2026-09-10",
    notes: "## What changed\n\n- A useful fix.",
  });

  assert.ok(updated.indexOf("[Unreleased]") < updated.indexOf("[0.66.3]"));
  assert.ok(updated.indexOf("[0.66.3]") < updated.indexOf("[0.66.2]"));
  assert.match(updated, /## \[0\.66\.3\] - 2026-09-10\n\n### What changed/);
  assert.match(updated, /- A useful fix\.\n\n## \[0\.66\.2\]/);
});

test("rerunning the same version replaces its entry without duplication", () => {
  const first = updateChangelog({
    current: "",
    version: "1.2.3",
    date: "2026-09-10",
    notes: "## First notes",
  });
  const second = updateChangelog({
    current: first,
    version: "1.2.3",
    date: "2026-09-11",
    notes: "## Corrected notes",
  });

  assert.equal(second.match(/## \[1\.2\.3\]/g)?.length, 1);
  assert.doesNotMatch(second, /First notes/);
  assert.match(second, /## \[1\.2\.3\] - 2026-09-11/);
  assert.match(second, /Corrected notes/);
});

test("invalid or empty release metadata is rejected", () => {
  assert.throws(
    () =>
      updateChangelog({ current: "", version: "v1", date: "today", notes: "" }),
    /Version must be X\.Y\.Z/,
  );
  assert.throws(
    () => normalizeNotesForChangelog("  "),
    /Release notes are empty/,
  );
});
