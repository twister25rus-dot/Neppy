import { spawn, spawnSync, execFileSync } from "node:child_process";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPTS = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCRIPT = path.join(SCRIPTS, "neppy-install-local.sh");
const RECOVER = path.join(SCRIPTS, "neppy-recover.sh");

function plist(version) {
  return `<?xml version="1.0"?>\n<plist version="1.0"><dict>\n<key>CFBundleShortVersionString</key>\n<string>${version}</string>\n</dict></plist>\n`;
}

function makeApp(dir, version, marker) {
  fs.mkdirSync(path.join(dir, "Contents", "MacOS"), { recursive: true });
  fs.writeFileSync(path.join(dir, "Contents", "Info.plist"), plist(version));
  fs.writeFileSync(path.join(dir, "Contents", "MacOS", "marker"), marker);
}

/**
 * A sandbox: fake /Applications holding Neppy 1.0.0, a freshly built 2.0.0, and
 * an "open" stub whose behaviour is chosen per test via $MODE:
 *   ok       writes launch-pending.json, then clears it after a moment
 *   hang     writes launch-pending.json and never clears it
 *   failed   writes a new last-failed-launch.json
 *   silent   does nothing (marker never appears)
 * It logs which version it was asked to open.
 */
function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "neppy-install-"));
  const apps = path.join(root, "Applications");
  const data = path.join(root, "data");
  const built = path.join(root, "built", "Neppy.app");
  fs.mkdirSync(apps);
  fs.mkdirSync(data);
  makeApp(path.join(apps, "Neppy.app"), "1.0.0", "old");
  makeApp(built, "2.0.0", "new");
  const open = path.join(root, "open-stub.sh");
  fs.writeFileSync(
    open,
    `#!/bin/bash
app="$1"
ver=$(awk '/CFBundleShortVersionString/{getline; gsub(/<[^>]*>|[ \\t]/,""); print; exit}' "$app/Contents/Info.plist")
echo "$ver" >> "${root}/opened.log"
# A new app only succeeds if it is the new version (the restored 1.0.0 always starts).
mode="$MODE"; [ "$ver" = "1.0.0" ] && mode=ok
case "$mode" in
  ok) echo '{"pid":1}' > "${data}/launch-pending.json"; ( sleep 0.6; rm -f "${data}/launch-pending.json" ) >/dev/null 2>&1 & ;;
  hang) echo '{"pid":1}' > "${data}/launch-pending.json" ;;
  failed) echo '{"pid":9,"detected_at":"now"}' > "${data}/last-failed-launch.json" ;;
  silent) : ;;
esac
`,
    { mode: 0o755 },
  );
  const env = {
    ...process.env,
    HOME: root,
    NEPPY_WORKSPACE: data,
    NEPPY_INSTALL_APPS_DIR: apps,
    NEPPY_INSTALL_BACKUP_DIR: path.join(root, "backups"),
    NEPPY_INSTALL_MARKER_DIR: data,
    NEPPY_INSTALL_STATE_DIR: path.join(root, "state"),
    NEPPY_INSTALL_OPEN_CMD: open,
    NEPPY_INSTALL_EXIT_TIMEOUT: "2",
    NEPPY_INSTALL_READY_TIMEOUT: "2",
    MODE: "ok",
  };
  return { root, apps, data, built, env, state: path.join(root, "state"), backups: path.join(root, "backups") };
}

function run(f, args, extraEnv = {}) {
  return spawnSync("bash", [SCRIPT, ...args], { env: { ...f.env, ...extraEnv }, encoding: "utf8" });
}

const result = (f) => JSON.parse(fs.readFileSync(path.join(f.state, "last-local-install.json"), "utf8"));
const installed = (f) => fs.readFileSync(path.join(f.apps, "Neppy.app", "Contents", "MacOS", "marker"), "utf8");
const opened = (f) => fs.readFileSync(path.join(f.root, "opened.log"), "utf8").trim().split("\n");

test("script is valid bash, executable and under 200 lines", () => {
  execFileSync("bash", ["-n", SCRIPT]);
  assert.ok(fs.readFileSync(SCRIPT, "utf8").split("\n").length < 200);
  assert.ok(fs.statSync(SCRIPT).mode & 0o100, "script must be executable");
});

test("a healthy new app replaces the installed one and the old one is backed up", () => {
  const f = fixture();
  const r = run(f, [f.built, "99999999"]);
  assert.equal(r.status, 0, r.stderr + fs.readFileSync(path.join(f.state, "local-install.log"), "utf8"));
  assert.equal(installed(f), "new");
  const res = result(f);
  assert.equal(res.status, "installed");
  assert.equal(res.version, "2.0.0");
  assert.match(path.basename(res.backup), /^Neppy-1\.0\.0-\d{8}T\d{6}Z\.app$/);
  assert.equal(fs.readFileSync(path.join(res.backup, "Contents", "MacOS", "marker"), "utf8"), "old");
  assert.deepEqual(opened(f), ["2.0.0"]);
  // no temp copies left behind in the apps dir
  assert.deepEqual(fs.readdirSync(f.apps), ["Neppy.app"]);
});

test("waits for the app pid to exit before touching anything", () => {
  const f = fixture();
  const child = spawn("sleep", ["30"], { stdio: "ignore" });
  const pid = String(child.pid);
  const r = run(f, [f.built, pid], { NEPPY_INSTALL_EXIT_TIMEOUT: "1" });
  child.kill();
  assert.equal(r.status, 1);
  assert.equal(installed(f), "old", "nothing was replaced");
  const res = result(f);
  assert.equal(res.status, "failed");
  assert.match(res.reason, /did not quit/);
  assert.equal(fs.existsSync(f.backups), false);
});

test("a launch marker that never clears restores the backup and reopens it", () => {
  const f = fixture();
  const r = run(f, [f.built, "99999999"], { MODE: "hang", NEPPY_INSTALL_READY_TIMEOUT: "1" });
  assert.equal(r.status, 0);
  assert.equal(installed(f), "old", "the previous app is back");
  const res = result(f);
  assert.equal(res.status, "restored");
  assert.equal(res.version, "2.0.0", "reports the version that failed");
  assert.match(res.reason, /never cleared/);
  assert.deepEqual(opened(f), ["2.0.0", "1.0.0"]);
});

test("a new last-failed-launch.json triggers a restore", () => {
  const f = fixture();
  fs.writeFileSync(path.join(f.data, "last-failed-launch.json"), '{"pid":1,"detected_at":"older"}');
  const r = run(f, [f.built, "99999999"], { MODE: "failed" });
  assert.equal(r.status, 0);
  assert.equal(installed(f), "old");
  assert.equal(result(f).status, "restored");
  assert.match(result(f).reason, /failed launch/);
});

test("an app that never writes a launch marker is treated as failed", () => {
  const f = fixture();
  run(f, [f.built, "99999999"], { MODE: "silent", NEPPY_INSTALL_READY_TIMEOUT: "1" });
  assert.equal(installed(f), "old");
  assert.match(result(f).reason, /never wrote a launch marker/);
});

test("only the newest 3 backups are kept", () => {
  const f = fixture();
  fs.mkdirSync(f.backups);
  for (const [i, v] of ["0.1.0", "0.2.0", "0.3.0"].entries()) {
    const d = path.join(f.backups, `Neppy-${v}-2026010${i + 1}T000000Z.app`);
    makeApp(d, v, "b");
    const t = new Date(2026, 0, i + 1);
    fs.utimesSync(d, t, t);
  }
  assert.equal(run(f, [f.built, "99999999"]).status, 0);
  const kept = fs.readdirSync(f.backups).sort();
  assert.equal(kept.length, 3);
  assert.ok(!kept.some((n) => n.includes("0.1.0")), `oldest pruned: ${kept}`);
  assert.ok(kept.some((n) => n.includes("1.0.0")), `fresh backup kept: ${kept}`);
});

test("rejects something that is not an app bundle without touching the install", () => {
  const f = fixture();
  const r = run(f, [path.join(f.root, "nope"), "1"]);
  assert.equal(r.status, 1);
  assert.equal(installed(f), "old");
  assert.equal(result(f).status, "failed");
});

test("restore-app puts the newest backup back", () => {
  const f = fixture();
  assert.equal(run(f, [f.built, "99999999"]).status, 0);
  assert.equal(installed(f), "new");
  const r = spawnSync("bash", [RECOVER, "restore-app"], { env: f.env, input: "y\n", encoding: "utf8" });
  assert.equal(r.status, 0, r.stderr + r.stdout);
  assert.match(r.stdout, /restored .*Neppy-1\.0\.0/);
  assert.equal(installed(f), "old");
  assert.deepEqual(fs.readdirSync(f.apps), ["Neppy.app"]);
});

test("restore-app asks first and reports a missing backup", () => {
  const f = fixture();
  const none = spawnSync("bash", [RECOVER, "restore-app"], { env: f.env, input: "y\n", encoding: "utf8" });
  assert.equal(none.status, 1);
  assert.match(none.stderr, /no app backup/);
  assert.equal(run(f, [f.built, "99999999"]).status, 0);
  const no = spawnSync("bash", [RECOVER, "restore-app"], { env: f.env, input: "n\n", encoding: "utf8" });
  assert.equal(no.status, 1);
  assert.equal(installed(f), "new", "declined: unchanged");
});
