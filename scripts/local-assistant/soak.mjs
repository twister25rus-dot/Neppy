#!/usr/bin/env node
/**
 * Local-assistant soak driver (macOS). Node built-ins only.
 *
 * Starts its OWN `neppy-core serve` against an isolated workspace and HOME,
 * with the MLX worker on a non-default port, then runs the protocol:
 *   A baseline (worker stopped)      B context scaling (2k/8k/14k prompts)
 *   C work/idle cycles, idle_policy=always (worker unloads, then stops)
 *   D work/idle cycles, idle_policy=pressure_only (model stays resident),
 *     then a release probe: /unload, stop, respawn
 *   E kill -9 of the worker mid-generation in one C cycle
 *   F fallback model cycles
 * The task runs on a disposable `git clone --local` of the repo and the clone
 * is reset between cycles. Nothing under ~/.neppy, no Ollama model and no MLX
 * server this script did not start is touched. HF_HUB_OFFLINE=1: no downloads.
 *
 * Writes run-<ts>/{samples.jsonl, marks.jsonl, driver.json, report.json,
 * report.md, core.log}. See README.md.
 *
 * Usage: node soak.mjs --scratch DIR [--core-bin PATH] [--source-repo PATH]
 *   [--phases A,B,C,D,F] [--c-cycles 10] [--d-cycles 6] [--f-cycles 3]
 *   [--idle-secs 150] [--d-idle-secs 60] [--a-secs 60] [--kill-cycle 5]
 *   [--max-steps 6] [--unload-secs 45] [--stop-secs 120] [--cycles N (dry run)]
 *   [--model ID] [--fallback-model ID] [--core-port 17790] [--worker-port 18764]
 */

import { execFileSync, spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { analyze, evaluateKillTest, renderMarkdown } from './analyze-cycles.mjs';
import { createSampler } from './sampler-macos.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const sh = (cmd, args, opt = {}) => execFileSync(cmd, args, { encoding: 'utf8', ...opt });

function parseArgs(argv) {
  const o = {
    scratch: null, coreBin: path.join(REPO, 'target/release/neppy-core'), sourceRepo: '/Users/alex/Neppy',
    phases: 'A,B,C,D,F', cCycles: 10, dCycles: 6, fCycles: 3, idleSecs: 150, dIdleSecs: 60, aSecs: 60,
    killCycle: 5, maxSteps: 6, unloadSecs: 45, stopSecs: 120, cycles: null,
    model: 'ornith-ai/Ornith-1.5-9B-MLX-8bit', fallbackModel: 'LiquidAI/LFM2.5-1.2B-Instruct-MLX-4bit',
    corePort: 17790, workerPort: 18764, binDir: path.join(process.env.HOME ?? '', '.local/bin'),
    hfHome: path.join(process.env.HOME ?? '', '.cache/huggingface'),
  };
  const map = {
    '--scratch': ['scratch', String], '--core-bin': ['coreBin', String], '--source-repo': ['sourceRepo', String],
    '--phases': ['phases', String], '--c-cycles': ['cCycles', Number], '--d-cycles': ['dCycles', Number],
    '--f-cycles': ['fCycles', Number], '--idle-secs': ['idleSecs', Number], '--d-idle-secs': ['dIdleSecs', Number],
    '--a-secs': ['aSecs', Number], '--kill-cycle': ['killCycle', Number], '--max-steps': ['maxSteps', Number],
    '--unload-secs': ['unloadSecs', Number], '--stop-secs': ['stopSecs', Number], '--cycles': ['cycles', Number],
    '--model': ['model', String], '--fallback-model': ['fallbackModel', String], '--core-port': ['corePort', Number],
    '--worker-port': ['workerPort', Number],
  };
  for (let i = 2; i < argv.length; i += 2) {
    const e = map[argv[i]];
    if (!e || argv[i + 1] === undefined) throw new Error(`bad argument: ${argv[i]}`);
    o[e[0]] = e[1](argv[i + 1]);
  }
  if (!o.scratch) throw new Error('--scratch DIR is required (a disk-backed directory, not the repo)');
  if (o.cycles) { o.cCycles = o.dCycles = o.fCycles = o.cycles; }
  return o;
}

const opts = parseArgs(process.argv);
const SCRATCH = path.resolve(opts.scratch);
const stamp = new Date().toISOString().replace(/[-:]/g, '').slice(0, 15);
const RUN = path.join(SCRATCH, 'runs', `run-${stamp}`);
const HOME = path.join(SCRATCH, 'home');
const WS = path.join(SCRATCH, 'ws');
const CLONE = path.join(SCRATCH, 'neppy-soak');
// Its own directory: the test command writes here, and the command policy only lets a
// path argument through when it is inside a trusted root (see renderConfig).
const TEST_LOG_DIR = path.join(SCRATCH, 'test-log');
const TEST_LOG = path.join(TEST_LOG_DIR, 'test-invocations.log');
const TOKEN = `soak-${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
const RPC = `http://127.0.0.1:${opts.corePort}/rpc`;
const WORKER = `http://127.0.0.1:${opts.workerPort}`;
const GOAL = 'Find every code path that spawns, stops, or reclaims an MLX server process and add a one-line `// NOTE(soak):` comment above each spawn call site; list file:line references in the summary.';
const TEST_FILE = 'src/neppy/inference/local/service/mlx_admin/process.rs';
const TEST_CMD = `echo run >> ${TEST_LOG}; git diff --stat && cargo fmt --check -- ${TEST_FILE}`;
const TERMINAL = new Set(['done', 'failed', 'budget_exhausted', 'cancelled', 'interrupted', 'paused']);

fs.mkdirSync(RUN, { recursive: true });
fs.mkdirSync(path.join(WS, 'workspace'), { recursive: true });
fs.mkdirSync(path.join(WS, 'projects'), { recursive: true });
fs.mkdirSync(HOME, { recursive: true });
fs.mkdirSync(TEST_LOG_DIR, { recursive: true });
const marksOut = fs.createWriteStream(path.join(RUN, 'marks.jsonl'));
const samplesOut = fs.createWriteStream(path.join(RUN, 'samples.jsonl'));
const driver = { args: opts, started: new Date().toISOString(), events: [], cycles: [], notes: [] };
const log = (...a) => { const l = `[soak ${new Date().toISOString().slice(11, 19)}] ${a.join(' ')}`; console.log(l); };
const mark = (kind, f = {}) => { const m = { t: Date.now(), kind, ...f }; marksOut.write(`${JSON.stringify(m)}\n`); return m; };

// ---------------------------------------------------------------- config
function renderConfig({ idlePolicy, model, localModel = '', fallback = '' }) {
  return `# generated by soak.mjs; isolated from ~/.neppy
[autonomy]
level = "full"
trusted_roots = [{ path = "${CLONE}", access = "readwrite" }, { path = "${TEST_LOG_DIR}", access = "readwrite" }]

[update]
enabled = false

[modules]
allow_download = false

[local_assistant]
model = "${localModel}"
max_steps = ${opts.maxSteps}

[mlx]
enabled = true
bin_dir = "${opts.binDir}"
embeddings_backend = "ollama"

[mlx.worker]
idle_policy = "${idlePolicy}"
idle_unload_secs = ${opts.unloadSecs}
idle_stop_secs = ${opts.stopSecs}
fallback_model = "${fallback}"

[[mlx.server]]
id = "primary"
kind = "vlm"
host = "127.0.0.1"
port = ${opts.workerPort}
model = "${model}"
model_discovery = "hf-cache"
max_kv_size = 16384
max_num_seqs = 1
kv_bits = 0.0
autostart = false
log_level = "INFO"
`;
}
function writeConfig(c) {
  for (const f of [path.join(WS, 'config.toml'), path.join(WS, 'workspace', 'config.toml')]) fs.writeFileSync(f, renderConfig(c));
  log(`config: idle_policy=${c.idlePolicy} model=${c.model}`);
}

// ---------------------------------------------------------------- rpc
async function rpc(method, params = {}, ms = 30000) {
  const res = await fetch(RPC, {
    method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${TOKEN}` },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }), signal: AbortSignal.timeout(ms),
  });
  const j = await res.json();
  if (j.error) throw new Error(`${method}: ${j.error.message}`);
  let v = j.result;
  if (v && typeof v === 'object' && 'result' in v && 'logs' in v) v = v.result;
  return v;
}

// ---------------------------------------------------------------- core lifecycle
let core = null;
const samplerOpts = { corePid: null, workerPort: opts.workerPort, intervalMs: 1000, rpcUrl: RPC };
const liveSampler = createSampler(samplerOpts);
let lastSample = null;
let lowAvail = 0;
let aborting = false;

process.env.NEPPY_CORE_TOKEN = TOKEN; // the sampler reads it for its status RPC

async function startCore() {
  const env = {
    PATH: `${opts.binDir}:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin`, HOME,
    HF_HOME: opts.hfHome, HF_HUB_OFFLINE: '1', TRANSFORMERS_OFFLINE: '1',
    NEPPY_WORKSPACE: WS, NEPPY_ACTION_DIR: path.join(WS, 'projects'),
    NEPPY_CORE_HOST: '127.0.0.1', NEPPY_CORE_PORT: String(opts.corePort), NEPPY_CORE_TOKEN: TOKEN,
    NEPPY_KEYRING_BACKEND: 'file', NEPPY_APPROVAL_GATE: '0', RUST_LOG: 'info,neppy=debug',
  };
  const out = fs.openSync(path.join(RUN, 'core.log'), 'a');
  core = spawn(opts.coreBin, ['serve'], { env, stdio: ['ignore', out, out], detached: false });
  for (let i = 0; i < 100; i += 1) {
    await sleep(300);
    try { if ((await fetch(`http://127.0.0.1:${opts.corePort}/health`)).ok) break; } catch { /* not up yet */ }
    if (core.exitCode !== null) throw new Error('core exited during startup');
  }
  samplerOpts.corePid = core.pid;
  await rpc('neppy.auth_store_session', { token: 'soak.session.local', user: { name: 'soak', email: 'soak@localhost' } });
  await rpc('neppy.mlx_worker_status'); // brings the watchdog up
  log(`core pid ${core.pid} on :${opts.corePort}`);
  mark('core_start', { pid: core.pid });
}

function workerPids() {
  try {
    const ps = sh('ps', ['-axo', 'pid=,command=']);
    return ps.split('\n').filter((l) => /mlx_(vlm|lm)[._]server/.test(l) && l.includes(`--port ${opts.workerPort}`)).map((l) => Number(l.trim().split(/\s+/)[0]));
  } catch { return []; }
}

async function stopWorker() {
  try { await rpc('neppy.mlx_stop', { id: 'primary' }, 60000); } catch (e) { log(`mlx_stop: ${e.message}`); }
  for (let i = 0; i < 20 && workerPids().length; i += 1) await sleep(500);
}

async function stopCore() {
  if (!core) return;
  await stopWorker();
  core.kill('SIGTERM');
  for (let i = 0; i < 20 && core.exitCode === null; i += 1) await sleep(250);
  if (core.exitCode === null) core.kill('SIGKILL');
  samplerOpts.corePid = null;
  core = null;
  // SIGTERM to the core does not run its drop handlers, so a worker can outlive it.
  for (const pid of workerPids()) { log(`reaping leftover worker ${pid}`); try { process.kill(pid, 'SIGTERM'); } catch { /* gone */ } }
  await sleep(1500);
}

void liveSampler.start((s) => {
  samplesOut.write(`${JSON.stringify(s)}\n`);
  lastSample = s;
  if (s.sys?.avail_pct != null && s.sys.avail_pct < 12) lowAvail += 1; else lowAvail = 0;
  if (lowAvail >= 5 && !aborting) { aborting = true; driver.notes.push('aborted: available memory below 12% for 5 samples'); log('ABORT: available memory < 12%'); void shutdown(2); }
});

async function shutdown(code = 0) {
  try { liveSampler.stop(); await stopCore(); } catch (e) { log(`cleanup: ${e.message}`); }
  fs.writeFileSync(path.join(RUN, 'driver.json'), JSON.stringify(driver, null, 2));
  process.exit(code);
}
process.on('SIGINT', () => { void shutdown(130); });
process.on('SIGTERM', () => { void shutdown(143); });

// ---------------------------------------------------------------- helpers
const dirMib = (p) => { try { return Number(sh('du', ['-sk', p]).split('\t')[0]) / 1024; } catch { return null; } };
const fileMib = (p) => { try { return fs.statSync(p).size / 1048576; } catch { return 0; } };

function resetClone() {
  sh('git', ['-C', CLONE, 'checkout', '-q', '--', '.']);
  sh('git', ['-C', CLONE, 'clean', '-fdq']);
  fs.writeFileSync(TEST_LOG, '');
}

async function runTask(phase, cycle, candidate, { killWhenGenerating = false } = {}) {
  resetClone();
  mark('work_start', { phase, cycle, candidate });
  const task = await rpc('neppy.local_assistant_start_task', {
    project_root: CLONE, goal: GOAL, allow_edits: true, test_command: TEST_CMD, max_steps: opts.maxSteps,
  });
  let killMs = null;
  let killedPid = null;
  let rec = task;
  let resumes = 0;
  const began = Date.now();
  while (Date.now() - began < 20 * 60000) {
    await sleep(2000);
    const st = await rpc('neppy.local_assistant_status', { task_id: task.id }).catch(() => null);
    if (st) rec = st.task;
    if (killWhenGenerating && killMs === null && lastSample?.worker?.phase === 'busy' && lastSample.worker.pid) {
      killedPid = lastSample.worker.pid;
      killMs = Date.now();
      log(`KILL -9 worker ${killedPid} mid-generation`);
      mark('kill', { phase, cycle, pid: killedPid });
      process.kill(killedPid, 'SIGKILL');
    }
    // A 9B model sometimes returns unparseable JSON and the step fails with its
    // checkpoint kept. Resume it (the designed recovery path), at most twice.
    if (rec.status === 'failed' && /model error/.test(rec.error ?? '') && resumes < 2) {
      resumes += 1;
      driver.notes.push(`${phase}${cycle}: resumed after "${rec.error.slice(0, 80)}"`);
      mark('task_resume', { phase, cycle, n: resumes });
      await rpc('neppy.local_assistant_resume', { task_id: task.id }).catch((e) => log(`resume: ${e.message}`));
      continue;
    }
    if (TERMINAL.has(rec.status)) break;
  }
  mark('work_end', { phase, cycle, status: rec.status, steps: rec.steps_done, task_id: task.id });
  log(`${phase}${cycle}: task ${rec.status} (resumed ${resumes}x) after ${rec.steps_done} steps in ${((Date.now() - began) / 1000).toFixed(0)}s`);
  return { rec, killMs, killedPid };
}

async function idleFor(secs, label) {
  log(`${label}: idle ${secs}s`);
  await sleep(secs * 1000);
}

async function runCycles(phase, n, idleSecs, candidate, { killCycle = null } = {}) {
  mark('phase_start', { phase, candidate });
  let kill = null;
  let killNext = killCycle;
  let attempts = 0;
  for (let c = 1; c <= n; c += 1) {
    const isKill = killNext === c;
    const { rec, killMs, killedPid } = await runTask(phase, c, candidate, { killWhenGenerating: isKill });
    const afterWork = Date.now();
    if (isKill) {
      kill = await evaluateKill(rec, killMs, killedPid);
      kill.cycle = c;
      attempts += 1;
      // A 9B model sometimes returns unparseable JSON, which fails the task
      // for reasons unrelated to the kill. Try again in the next cycle.
      const modelFailure = /model error/.test(rec.error ?? '');
      if ((kill.skipped || (!kill.pass && modelFailure)) && attempts < 3) { killNext = c + 1; driver.notes.push(`kill attempt in cycle ${c} inconclusive (${kill.skipped ? 'no generation seen' : rec.error}); retrying`); }
    }
    const wait = Math.max(0, idleSecs * 1000 - (Date.now() - afterWork));
    await sleep(wait);
    const ws = dirMib(WS);
    mark('idle_end', { phase, cycle: c, workspace_mib: ws });
    driver.cycles.push({ phase, cycle: c, status: rec.status, steps: rec.steps_done, tokens: rec.completion_tokens_used, workspace_mib: ws,
      state_db_mib: fileMib(path.join(WS, 'workspace/local_assistant/state.db')) });
  }
  mark('phase_end', { phase });
  return kill;
}

function sqlite(dbPath, sql) {
  try { return sh('sqlite3', ['-separator', '|', dbPath, sql]).trim(); } catch { return ''; }
}

async function evaluateKill(rec, killMs, killedPid) {
  if (killMs === null) return { skipped: true, reason: 'the worker was never observed generating, so no kill was made' };
  await sleep(1000);
  const db = path.join(WS, 'workspace/local_assistant/state.db');
  const ledgerRows = Number(sqlite(db, `select count(*) from effects where task_id='${rec.id}' and kind='test'`) || 0);
  const invocations = fs.existsSync(TEST_LOG) ? fs.readFileSync(TEST_LOG, 'utf8').split('\n').filter(Boolean).length : 0;
  let stacked = 0;
  for (const f of sh('git', ['-C', CLONE, 'diff', '--name-only']).split('\n').filter(Boolean)) {
    const lines = fs.readFileSync(path.join(CLONE, f), 'utf8').split('\n');
    for (let i = 0; i + 1 < lines.length; i += 1) if (lines[i].includes('NOTE(soak)') && lines[i + 1].includes('NOTE(soak)')) stacked += 1;
  }
  const editsApplied = Number(sqlite(db, `select count(*) from effects where task_id='${rec.id}' and kind='edit' and status in ('applied','done')`) || 0);
  const win = await rpc('neppy.mlx_worker_metrics', { since_ms: killMs - 5000, limit: 500, events_only: true });
  const orphans = workerPids().filter((p) => p === killedPid);
  const effects = sqlite(db, `select kind||'/'||status||'='||count(*) from effects where task_id='${rec.id}' group by kind,status`);
  const result = evaluateKillTest({ killMs, events: win.events, taskStatus: rec.status, testRuns: { invocations, ledgerRows }, stackedNoteLines: stacked, editsApplied, orphanWorkerPids: orphans });
  return { ...result, killMs, killedPid, taskId: rec.id, effects, events: win.events.map((e) => `${e.ts_ms - killMs}ms ${e.event} ${e.detail}`) };
}

// ---------------------------------------------------------------- phases
async function waitLoaded(ms = 180000) {
  const t0 = Date.now();
  while (Date.now() - t0 < ms) {
    try { const h = await (await fetch(`${WORKER}/health`, { signal: AbortSignal.timeout(2000) })).json(); if (h.loaded_model) return true; } catch { /* starting */ }
    await sleep(1500);
  }
  return false;
}

function fillerText(chars) {
  const files = sh('git', ['-C', CLONE, 'ls-files', 'src/neppy/inference/local']).split('\n').filter((f) => f.endsWith('.rs'));
  let out = '';
  for (const f of files) { out += `// ${f}\n${fs.readFileSync(path.join(CLONE, f), 'utf8')}\n`; if (out.length >= chars) break; }
  return out.slice(0, chars);
}

async function phaseB() {
  mark('phase_start', { phase: 'B' });
  writeConfig({ idlePolicy: 'pressure_only', model: opts.model });
  await rpc('neppy.mlx_start', { id: 'primary' }, 240000);
  if (!(await waitLoaded())) { driver.notes.push('B: model never reported loaded'); return; }
  await sleep(12000);
  for (const target of [2000, 8000, 14000]) {
    const text = fillerText(Math.floor(target * 3.85));
    mark('ctx_start', { target });
    let usage = null; let ok = true;
    try {
      const res = await fetch(`${WORKER}/v1/chat/completions`, {
        method: 'POST', headers: { 'content-type': 'application/json' }, signal: AbortSignal.timeout(300000),
        body: JSON.stringify({ model: opts.model, max_tokens: 32, temperature: 0, stream: false,
          chat_template_kwargs: { enable_thinking: false },
          messages: [{ role: 'user', content: `Summarize in one line what this code is:\n${text}` }] }),
      });
      const j = await res.json(); usage = j.usage ?? null; ok = res.ok;
      if (!res.ok) driver.notes.push(`B ${target}: HTTP ${res.status} ${JSON.stringify(j).slice(0, 200)}`);
    } catch (e) { ok = false; driver.notes.push(`B ${target}: ${e.message}`); }
    mark('ctx_end', { target, prompt_tokens: usage?.prompt_tokens ?? null, ok });
    log(`B ${target}: prompt_tokens=${usage?.prompt_tokens} ok=${ok}`);
    await sleep(20000);
  }
  await stopWorker();
  mark('phase_end', { phase: 'B' });
}

async function phaseD() {
  writeConfig({ idlePolicy: 'pressure_only', model: opts.model });
  await runCycles('D', opts.dCycles, opts.dIdleSecs, opts.model);
  // Release probe: where did any growth go when asked to give memory back?
  const fp = async (name) => {
    await sleep(10000);
    const w = lastSample?.worker;
    mark('probe', { phase: 'D', name, fp_mib: w?.fp_mib ?? null, rss_mib: w?.rss_mib ?? null, gpu_alloc_mib: lastSample?.gpu_alloc_mib ?? null });
    log(`D probe ${name}: worker fp ${w?.fp_mib?.toFixed(0)} MiB`);
  };
  await fp('end_loaded');
  await rpc('neppy.mlx_unload', { id: 'primary' });
  await fp('end_after_unload');
  await stopWorker();
  await sleep(3000);
  await rpc('neppy.mlx_start', { id: 'primary' }, 240000);
  await waitLoaded();
  await fp('end_after_respawn_loaded');
  await stopWorker();
}

function versions() {
  const v = (cmd, args) => { try { return sh(cmd, args, { stdio: ['ignore', 'pipe', 'ignore'] }).trim().split('\n')[0]; } catch { return 'n/a'; } };
  const py = path.join(process.env.HOME ?? '', '.local/share/uv/tools/mlx-vlm/bin/python');
  return {
    macos: v('sw_vers', ['-productVersion']), machine: v('sysctl', ['-n', 'hw.model']), memBytes: v('sysctl', ['-n', 'hw.memsize']),
    neppyCoreGit: v('git', ['-C', REPO, 'rev-parse', '--short', 'HEAD']),
    mlxVlm: v(py, ['-c', 'import importlib.metadata as m;print(m.version("mlx-vlm"))']),
    mlx: v(py, ['-c', 'import importlib.metadata as m;print(m.version("mlx"))']),
    node: process.version, ollama: v('ollama', ['--version']),
  };
}

function phaseASummary(t0, t1) {
  const rows = fs.readFileSync(path.join(RUN, 'samples.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)).filter((s) => s.t >= t0 && s.t <= t1);
  const med = (fn) => { const v = rows.map(fn).filter((x) => typeof x === 'number').sort((a, b) => a - b); return v.length ? v[v.length >> 1] : null; };
  return { samples: rows.length, core_fp_mib: med((s) => s.core?.fp_mib), core_rss_mib: med((s) => s.core?.rss_mib), sys_used_mib: med((s) => s.sys?.used), avail_pct: med((s) => s.sys?.avail_pct), gpu_alloc_mib: med((s) => s.gpu_alloc_mib), worker_present: rows.some((s) => s.worker) };
}

// ---------------------------------------------------------------- main
(async () => {
  log(`run dir ${RUN}`);
  if (!fs.existsSync(opts.coreBin)) throw new Error(`core binary not found: ${opts.coreBin}`);
  if (!fs.existsSync(CLONE)) { log('cloning disposable project'); sh('git', ['clone', '--local', '-q', opts.sourceRepo, CLONE]); }
  const phases = new Set(opts.phases.split(','));
  const extra = { kill: null, phaseA: null };
  driver.versions = versions();
  writeConfig({ idlePolicy: 'always', model: opts.model });
  await startCore();

  if (phases.has('A')) {
    mark('phase_start', { phase: 'A' }); const t0 = Date.now();
    await sleep(opts.aSecs * 1000);
    extra.phaseA = phaseASummary(t0, Date.now()); mark('phase_end', { phase: 'A' }); log('A done');
  }
  if (phases.has('B')) await phaseB();
  if (phases.has('C')) {
    writeConfig({ idlePolicy: 'always', model: opts.model });
    await stopWorker();
    extra.kill = await runCycles('C', opts.cCycles, opts.idleSecs, opts.model, { killCycle: phases.has('E') || phases.has('C') ? opts.killCycle : null });
  }
  if (phases.has('D')) await phaseD();
  if (phases.has('F')) {
    writeConfig({ idlePolicy: 'always', model: opts.fallbackModel, localModel: opts.fallbackModel });
    await stopWorker();
    await runCycles('F', opts.fCycles, opts.idleSecs, opts.fallbackModel);
  }

  await stopCore();
  liveSampler.stop();
  await sleep(1500);
  const read = (f) => fs.readFileSync(path.join(RUN, f), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
  const report = analyze({ samples: read('samples.jsonl'), marks: read('marks.jsonl') });
  report.driver = driver; report.kill = extra.kill; report.phaseA = extra.phaseA; report.orphans = workerPids();
  fs.writeFileSync(path.join(RUN, 'report.json'), JSON.stringify(report, null, 2));
  const md = [`# Local assistant soak ${stamp}`, '', '```json', JSON.stringify(driver.versions, null, 2), '```', '',
    `Phase A (worker stopped, ${opts.aSecs}s): ${JSON.stringify(extra.phaseA)}`, '',
    extra.kill ? `## Kill test (phase C cycle ${opts.killCycle})\n\n${JSON.stringify(extra.kill, null, 2)}\n` : '',
    renderMarkdown(report), '', `Orphan mlx workers at exit: ${JSON.stringify(report.orphans)}`, '',
    `Driver notes: ${JSON.stringify(driver.notes)}`, '', 'Cycle outcomes:', '', '| phase | cycle | status | steps | tokens | workspace MiB |', '|---|---|---|---|---|---|',
    ...driver.cycles.map((c) => `| ${c.phase} | ${c.cycle} | ${c.status} | ${c.steps} | ${c.tokens} | ${c.workspace_mib?.toFixed(1)} |`)].join('\n');
  fs.writeFileSync(path.join(RUN, 'report.md'), md);
  fs.writeFileSync(path.join(RUN, 'driver.json'), JSON.stringify(driver, null, 2));
  log(`report: ${path.join(RUN, 'report.md')}`);
  process.exit(0);
})().catch(async (e) => { log(`FATAL: ${e.stack ?? e}`); driver.notes.push(`fatal: ${e.message}`); await shutdown(1); });
