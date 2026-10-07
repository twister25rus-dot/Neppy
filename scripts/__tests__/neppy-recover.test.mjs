import { execFileSync, spawnSync } from "node:child_process";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPT = path.join(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  "neppy-recover.sh",
);

const GIT_ID = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"];

function git(repo, ...args) {
  return execFileSync("git", ["-C", repo, ...GIT_ID, ...args], {
    encoding: "utf8",
    env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1" },
  }).trim();
}

/** Temp repo + temp workspace + temp data dir; a checkpoint of "one". */
function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "neppy-recover-"));
  const repo = path.join(root, "repo");
  const ws = path.join(root, "ws");
  fs.mkdirSync(repo);
  fs.mkdirSync(path.join(ws, "debug_mode", "known_good"), { recursive: true });
  git(repo, "init", "-q");
  git(repo, "symbolic-ref", "HEAD", "refs/heads/main");
  fs.writeFileSync(path.join(repo, "a.txt"), "one\n");
  git(repo, "add", ".");
  git(repo, "commit", "-q", "-m", "init");
  // A checkpoint the way Debug Mode pins one: commit + `-index` tree ref.
  const tree = git(repo, "write-tree");
  const commit = git(repo, "commit-tree", tree, "-p", "HEAD", "-m", "cp");
  git(repo, "update-ref", "refs/neppy-debug/checkpoints/cp-1", commit);
  git(repo, "update-ref", "refs/neppy-debug/checkpoints/cp-1-index", tree);
  const env = {
    ...process.env,
    HOME: root,
    NEPPY_WORKSPACE: ws,
    NEPPY_RECOVER_REPO: repo,
  };
  return { root, repo, ws, env };
}

function run(f, args, input = "") {
  return spawnSync("bash", [SCRIPT, ...args], { env: f.env, input, encoding: "utf8" });
}

test("script is syntactically valid bash and under 250 lines", () => {
  execFileSync("bash", ["-n", SCRIPT]);
  assert.ok(fs.readFileSync(SCRIPT, "utf8").split("\n").length < 250);
});

test("status reports markers, recent tasks and checkpoints", () => {
  const f = fixture();
  fs.writeFileSync(
    path.join(f.ws, "launch-pending.json"),
    JSON.stringify({ pid: 4242, version: "0.67.0", started_at: "2026-10-07T00:00:00Z" }, null, 2),
  );
  fs.writeFileSync(
    path.join(f.ws, "last-failed-launch.json"),
    JSON.stringify(
      { pid: 7, version: "0.66.0", started_at: "s", detected_at: "2026-10-06T01:02:03Z" },
      null,
      2,
    ),
  );
  fs.writeFileSync(
    path.join(f.ws, "debug_mode", "history.json"),
    JSON.stringify(
      [
        { id: "task-1", request: "fix the tray icon", status: "pass", validation: [{ check_id: "x" }] },
        { id: "task-2", request: "rewrite the updater", status: "failed", validation: [] },
      ],
      null,
      2,
    ),
  );
  const r = run(f, ["status"]);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /pid=4242 version=0\.67\.0/);
  assert.match(r.stdout, /detected_at=2026-10-06T01:02:03Z/);
  assert.match(r.stdout, /task-1\s+pass\s+fix the tray icon/);
  assert.match(r.stdout, /task-2\s+failed\s+rewrite the updater/);
  assert.match(r.stdout, /cp-1\n/);
  assert.doesNotMatch(r.stdout, /cp-1-index/);
});

test("status copes with an empty workspace", () => {
  const f = fixture();
  const r = run(f, ["status"]);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /launch pending .*: none/);
  assert.match(r.stdout, /no history/);
});

test("restore confirms, saves a safety ref, restores files, keeps untracked ones", () => {
  const f = fixture();
  fs.writeFileSync(path.join(f.repo, "a.txt"), "two\n");
  fs.writeFileSync(path.join(f.repo, "b.txt"), "untracked\n");
  const r = run(f, ["restore", "cp-1"], "y\n");
  assert.equal(r.status, 0, r.stderr + r.stdout);
  assert.equal(fs.readFileSync(path.join(f.repo, "a.txt"), "utf8"), "one\n");
  assert.ok(fs.existsSync(path.join(f.repo, "b.txt")), "untracked file is never deleted");
  const refs = git(f.repo, "for-each-ref", "--format=%(refname)", "refs/neppy-debug/recovery/")
    .split("\n")
    .filter((x) => x && !x.endsWith("-index"));
  assert.equal(refs.length, 1);
  // The safety snapshot holds the pre-restore edit, including the untracked file.
  assert.equal(git(f.repo, "show", `${refs[0]}:a.txt`), "two");
  assert.equal(git(f.repo, "show", `${refs[0]}:b.txt`), "untracked");
  assert.equal(git(f.repo, "rev-parse", "--abbrev-ref", "HEAD"), "main");
});

test("restore aborts on anything but y and changes nothing", () => {
  const f = fixture();
  fs.writeFileSync(path.join(f.repo, "a.txt"), "two\n");
  const r = run(f, ["restore", "cp-1"], "n\n");
  assert.notEqual(r.status, 0);
  assert.equal(fs.readFileSync(path.join(f.repo, "a.txt"), "utf8"), "two\n");
  assert.equal(git(f.repo, "for-each-ref", "refs/neppy-debug/recovery/"), "");
});

test("restore rejects unknown or malformed checkpoint ids", () => {
  const f = fixture();
  assert.notEqual(run(f, ["restore", "nope"], "y\n").status, 0);
  assert.notEqual(run(f, ["restore", "../x"], "y\n").status, 0);
  assert.notEqual(run(f, ["restore", "cp-1; rm -rf /"], "y\n").status, 0);
  assert.notEqual(run(f, ["restore"], "y\n").status, 0);
});

test("known-good lists saved binaries and run-known-good starts the newest with serve", () => {
  const f = fixture();
  const dir = path.join(f.ws, "debug_mode", "known_good");
  assert.match(run(f, ["known-good"]).stdout, /no saved known-good/);
  const old = path.join(dir, "neppy-core-20260101-000000-000");
  const fresh = path.join(dir, "neppy-core-20260102-000000-000");
  fs.writeFileSync(old, "#!/bin/sh\necho old $@\n", { mode: 0o755 });
  fs.writeFileSync(fresh, "#!/bin/sh\necho fresh $@\n", { mode: 0o755 });
  fs.utimesSync(old, new Date(1000), new Date(1000));
  const list = run(f, ["known-good"]).stdout;
  assert.ok(list.indexOf("20260102") < list.indexOf("20260101"), "newest first");
  const r = run(f, ["run-known-good"]);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /fresh serve/);
  assert.match(run(f, ["run-known-good", old]).stdout, /old serve/);
});

test("logs tails the newest log files", () => {
  const f = fixture();
  const logs = path.join(f.ws, "logs");
  fs.mkdirSync(logs);
  fs.writeFileSync(path.join(logs, "neppy-2026-10-07.log"), "line1\nline2\nline3\n");
  const r = run(f, ["logs", "2"]);
  assert.equal(r.status, 0, r.stderr);
  assert.match(r.stdout, /line3/);
  assert.doesNotMatch(r.stdout, /line1/);
  assert.notEqual(run(f, ["logs", "abc"]).status, 0);
});

test("unknown commands fail and help succeeds", () => {
  const f = fixture();
  assert.notEqual(run(f, ["bogus"]).status, 0);
  assert.equal(run(f, ["--help"]).status, 0);
});
