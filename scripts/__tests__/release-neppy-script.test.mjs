import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const script = readFileSync(
  new URL("../release-neppy.sh", import.meta.url),
  "utf8",
);

test("release metadata is committed and tagged before GitHub release creation", () => {
  const releaseCommit = script.indexOf('git commit -q -m "Release $VERSION"');
  const releasePush = script.indexOf('git push origin "HEAD:$RELEASE_BRANCH"');
  const tag = script.indexOf('git tag -a "$TAG"');
  const createRelease = script.indexOf('gh release create "$TAG"');

  assert.ok(releaseCommit >= 0);
  assert.ok(releaseCommit < releasePush);
  assert.ok(releasePush < tag);
  assert.ok(tag < createRelease);
});

test("the GitHub release explicitly uploads only the signed app artifact pair", () => {
  const releaseCommand = script.slice(
    script.indexOf('gh release create "$TAG"'),
    script.indexOf("# A private repo"),
  );

  assert.match(releaseCommand, /--notes-file "\$NOTES_FILE"/);
  assert.match(releaseCommand, /"\$TARBALL" "\$SIGFILE"/);
  assert.doesNotMatch(releaseCommand, /\.zip|source/i);
  assert.match(script, /GitHub adds "Source code \(zip\)"[\s\S]*automatically/);
});

test("release staging is explicit and dry-run restores the changelog", () => {
  assert.doesNotMatch(script, /git add -A/);
  assert.match(script, /git add -- "\$\{RELEASE_METADATA\[@\]\}"/);
  assert.match(script, /CHANGELOG\.md/);
  assert.match(script, /dry run: release metadata restored/);
});

test("notes fall back to filtered commit bullets and accept a curated override", () => {
  assert.match(script, /--notes-file/);
  assert.match(script, /render_commit_bullets/);
  assert.match(script, /Release\\ \[0-9\]\*/);
  assert.match(script, /Update\\ updater\\ feed/);
  assert.match(script, /grep -Eq '\^\[\*-\]\[\[:space:\]\]\+'/);
  assert.match(script, /matching release commit is after that tag/);
});
