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

test("the release build compiles into a target directory of its own", () => {
  // A shared target directory let a second checkout's artifacts into a release
  // build, which then linked a mix of two source roots and failed to compile a
  // tree that was clean on its own.
  assert.match(
    script,
    /RELEASE_TARGET_DIR="\$\{NEPPY_RELEASE_TARGET_DIR:-\$ROOT\/app\/src-tauri\/target-release\}"/,
  );
  assert.match(script, /export CARGO_TARGET_DIR="\$RELEASE_TARGET_DIR"/);

  // The bundle is collected from where that build actually wrote it.
  assert.match(script, /BUNDLE_DIR="\$RELEASE_TARGET_DIR\/release\/bundle"/);
  assert.doesNotMatch(
    script,
    /BUNDLE_DIR="\$ROOT\/app\/src-tauri\/target\/release\/bundle"/,
  );

  // And a symlinked directory is refused: it is the same sharing wearing a
  // private directory's clothes, which is how this went unnoticed.
  assert.match(script, /! -L "\$RELEASE_TARGET_DIR"/);

  // The export has to precede the build it governs.
  const exported = script.indexOf('export CARGO_TARGET_DIR="$RELEASE_TARGET_DIR"');
  const built = script.indexOf("tauri build");
  assert.ok(exported >= 0);
  assert.ok(exported < built);
});
