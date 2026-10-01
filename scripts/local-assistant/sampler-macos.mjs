#!/usr/bin/env node
/**
 * macOS resource sampler for the local-assistant soak. Node built-ins only.
 *
 * Samples at ~1 Hz and writes one JSON object per line. It measures the core
 * and the MLX worker FROM THE OUTSIDE, so it does not depend on the numbers
 * the code under test reports about itself; those are recorded next to it
 * (`neppy.*`) so the analyzer can show how far they diverge.
 *
 * Per sample (all memory in MiB unless the name says otherwise):
 *   t                      epoch ms
 *   core   {pid, rss_mib, fp_mib, fp_peak_mib, threads, fds}
 *   worker {pid, rss_mib, fp_mib, fp_peak_mib, phase, loaded_model, in_flight,
 *           neppy_fp_mib} | null   phase: starting|busy|loaded|unloaded
 *   sys    {avail_pct, pressure, swap_used_mib, free, active, inactive, wired,
 *           compressor, file_backed, used}   used = active + wired + compressor
 *   gpu_in_use_mib, gpu_alloc_mib           IOAccelerator, sudo-free
 *   ollama {loaded: [{name, size_mib}]}     observed, never managed
 *   neppy  {pressure_state, gate_active, gate_waiting, restarts, worker_fp_mib,
 *           core_fp_mib, gpu_in_use_mib}   mlx.worker_status (the code's own numbers)
 *
 * phys_footprint (`footprint -p`) is the per-process number used for verdicts:
 * it counts compressed and Metal-wired allocations that RSS leaves out. RSS is
 * kept because it is what `ps` and Activity Monitor users compare against.
 *
 * Slow probes (threads, fds, ollama, status RPC) run every few ticks and the
 * last value is carried forward in between.
 *
 * Usage:
 *   node sampler-macos.mjs --core-pid N --worker-port P [--interval-ms 1000]
 *        [--rpc-url http://127.0.0.1:17790/rpc] [--out samples.jsonl]
 *   (bearer for the status RPC comes from OPENHUMAN_CORE_TOKEN)
 */

import { execFile } from 'node:child_process';
import fs from 'node:fs';
import { promisify } from 'node:util';

const run = promisify(execFile);
const MIB = 1024 * 1024;

async function sh(cmd, args, timeout = 4000) {
  try {
    const { stdout } = await run(cmd, args, { timeout, maxBuffer: 8 * MIB });
    return stdout;
  } catch {
    return null;
  }
}

const num = (s) => {
  const n = Number(s);
  return Number.isFinite(n) ? n : null;
};

/** Parse `vm_stat` into MiB per bucket. Exported for tests. */
export function parseVmStat(text) {
  if (!text) return null;
  const pageSize = Number(/page size of (\d+) bytes/.exec(text)?.[1] ?? 16384);
  const pages = (label) => {
    const m = new RegExp(`^${label}:\\s+(\\d+)\\.`, 'm').exec(text);
    return m ? (Number(m[1]) * pageSize) / MIB : null;
  };
  const free = pages('Pages free');
  const active = pages('Pages active');
  const inactive = pages('Pages inactive');
  const wired = pages('Pages wired down');
  const compressor = pages('Pages occupied by compressor');
  const fileBacked = pages('File-backed pages');
  const anon = pages('Anonymous pages');
  if ([active, wired, compressor].some((v) => v === null)) return null;
  return {
    free,
    active,
    inactive,
    wired,
    compressor,
    file_backed: fileBacked,
    anon,
    used: active + wired + compressor,
  };
}

/** Parse `vm.swapusage`: "total = 0.00M  used = 12.50M  free = ...". */
export function parseSwapUsedMib(text) {
  const m = /used = ([\d.]+)([MGK])/.exec(text ?? '');
  if (!m) return null;
  const v = Number(m[1]);
  return m[2] === 'G' ? v * 1024 : m[2] === 'K' ? v / 1024 : v;
}

/** Parse `footprint -p PID -f bytes` into MiB. */
export function parseFootprint(text) {
  if (!text) return null;
  const cur = /phys_footprint:\s+(\d+) B/.exec(text)?.[1];
  const peak = /phys_footprint_peak:\s+(\d+) B/.exec(text)?.[1];
  if (!cur) return null;
  return { fp_mib: Number(cur) / MIB, fp_peak_mib: peak ? Number(peak) / MIB : null };
}

/** Parse ioreg's IOAccelerator counters. */
export function parseIoreg(text) {
  if (!text) return null;
  const used = /"In use system memory"=(\d+)/.exec(text)?.[1];
  const alloc = /"Alloc system memory"=(\d+)/.exec(text)?.[1];
  return {
    gpu_in_use_mib: used ? Number(used) / MIB : null,
    gpu_alloc_mib: alloc ? Number(alloc) / MIB : null,
  };
}

/** Find the worker pid in `ps -axo pid=,rss=,command=` output. */
export function findWorker(psText, port) {
  if (!psText) return null;
  const portRe = new RegExp(`--port[ =]${port}(\\s|$)`);
  for (const line of psText.split('\n')) {
    const m = /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line);
    if (!m) continue;
    if (/mlx_(vlm|lm)[._]server|mlx_(vlm|lm)\.server/.test(m[3]) && portRe.test(m[3])) {
      return { pid: Number(m[1]), rss_mib: Number(m[2]) / 1024 };
    }
  }
  return null;
}

function rssOf(psText, pid) {
  for (const line of (psText ?? '').split('\n')) {
    const m = /^\s*(\d+)\s+(\d+)\s/.exec(line);
    if (m && Number(m[1]) === pid) return Number(m[2]) / 1024;
  }
  return null;
}

async function fetchJson(url, headers = {}, body = null, ms = 2500) {
  try {
    const res = await fetch(url, {
      method: body ? 'POST' : 'GET',
      headers: body ? { 'content-type': 'application/json', ...headers } : headers,
      body: body ? JSON.stringify(body) : undefined,
      signal: AbortSignal.timeout(ms),
    });
    if (!res.ok) return null;
    return await res.json();
  } catch {
    return null;
  }
}

export function createSampler(opts) {
  const interval = opts.intervalMs ?? 1000;
  const token = process.env.OPENHUMAN_CORE_TOKEN ?? '';
  const slow = { threads: null, fds: null, ollama: null, neppy: null, gpu: null, health: null };
  let tick = 0;
  let stopped = false;

  async function probeThreads(pid) {
    const out = await sh('ps', ['-M', '-p', String(pid)]);
    if (!out) return null;
    return Math.max(0, out.trim().split('\n').length - 1);
  }
  async function probeFds(pid) {
    const out = await sh('lsof', ['-n', '-P', '-p', String(pid)], 8000);
    if (!out) return null;
    return Math.max(0, out.trim().split('\n').length - 1);
  }

  async function sample() {
    tick += 1;
    const [sysctl, vm, ps] = await Promise.all([
      sh('sysctl', ['-n', 'kern.memorystatus_level', 'kern.memorystatus_vm_pressure_level', 'vm.swapusage']),
      sh('vm_stat', []),
      sh('ps', ['-axo', 'pid=,rss=,command=']),
    ]);
    const lines = (sysctl ?? '').trim().split('\n');
    const sys = {
      avail_pct: num(lines[0]),
      pressure: num(lines[1]),
      swap_used_mib: parseSwapUsedMib(lines[2]),
      ...(parseVmStat(vm) ?? {}),
    };

    const workerHit = findWorker(ps, opts.workerPort);
    const corePid = opts.corePid;
    const [coreFp, workerFp] = await Promise.all([
      corePid ? sh('footprint', ['-p', String(corePid), '-f', 'bytes', '--noCategories']) : null,
      workerHit ? sh('footprint', ['-p', String(workerHit.pid), '-f', 'bytes', '--noCategories']) : null,
    ]);

    if (tick % 2 === 1) {
      const io = parseIoreg(await sh('ioreg', ['-r', '-d', '1', '-w', '0', '-c', 'IOAccelerator']));
      if (io) slow.gpu = io;
      if (workerHit) {
        const base = `http://127.0.0.1:${opts.workerPort}`;
        const [health, metrics] = await Promise.all([
          fetchJson(`${base}/health`),
          fetchJson(`${base}/metrics`),
        ]);
        slow.health = health || metrics ? { health, metrics } : { health: null, metrics: null };
      } else {
        slow.health = null;
      }
    }
    if (tick % 5 === 1 && corePid) slow.threads = await probeThreads(corePid);
    if (tick % 10 === 1) {
      if (corePid) slow.fds = await probeFds(corePid);
      const ps2 = await fetchJson('http://127.0.0.1:11434/api/ps');
      slow.ollama = ps2
        ? { loaded: (ps2.models ?? []).map((m) => ({ name: m.name, size_mib: (m.size ?? 0) / MIB })) }
        : null;
    }
    if (tick % 3 === 1 && opts.rpcUrl) {
      const r = await fetchJson(
        opts.rpcUrl,
        { authorization: `Bearer ${token}` },
        { jsonrpc: '2.0', id: 1, method: 'openhuman.mlx_worker_status', params: {} },
      );
      let v = r?.result;
      if (v && typeof v === 'object' && 'result' in v && 'logs' in v) v = v.result;
      slow.neppy = v
        ? {
            pressure_state: v.pressure_state ?? null,
            gate_active: v.gate?.active ?? null,
            gate_waiting: v.gate?.waiting ?? null,
            restarts: v.restarts_in_window ?? null,
            worker_fp_mib: v.worker?.footprint_mib ?? null,
            core_fp_mib: v.last_sample?.core_footprint_mib ?? null,
            gpu_in_use_mib: v.last_sample?.gpu_in_use_mib ?? null,
          }
        : null;
    }

    const cf = parseFootprint(coreFp);
    const core = corePid
      ? {
          pid: corePid,
          rss_mib: rssOf(ps, corePid),
          fp_mib: cf?.fp_mib ?? null,
          fp_peak_mib: cf?.fp_peak_mib ?? null,
          threads: slow.threads,
          fds: slow.fds,
        }
      : null;

    let worker = null;
    if (workerHit) {
      const wf = parseFootprint(workerFp);
      const h = slow.health?.health;
      const inFlight = slow.health?.metrics?.summary?.in_flight ?? slow.health?.metrics?.in_flight ?? null;
      const loaded = h?.loaded_model ?? (Array.isArray(h?.loaded_models) ? h.loaded_models[0] : null) ?? null;
      let phase = 'starting';
      if (h) phase = inFlight > 0 ? 'busy' : loaded ? 'loaded' : 'unloaded';
      worker = {
        pid: workerHit.pid,
        rss_mib: workerHit.rss_mib,
        fp_mib: wf?.fp_mib ?? null,
        fp_peak_mib: wf?.fp_peak_mib ?? null,
        phase,
        loaded_model: loaded,
        in_flight: inFlight,
        neppy_fp_mib: slow.neppy?.worker_fp_mib ?? null,
      };
    }

    return {
      t: Date.now(),
      core,
      worker,
      sys,
      gpu_in_use_mib: slow.gpu?.gpu_in_use_mib ?? null,
      gpu_alloc_mib: slow.gpu?.gpu_alloc_mib ?? null,
      ollama: slow.ollama,
      neppy: slow.neppy,
    };
  }

  async function start(write) {
    while (!stopped) {
      const began = Date.now();
      try {
        write(await sample());
      } catch (err) {
        write({ t: Date.now(), error: String(err?.message ?? err) });
      }
      const wait = interval - (Date.now() - began);
      if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    }
  }

  return { start, stop: () => { stopped = true; }, sample };
}

function parseArgs(argv) {
  const o = { corePid: null, workerPort: null, intervalMs: 1000, rpcUrl: null, out: null, durationMs: null };
  for (let i = 2; i < argv.length; i += 1) {
    const key = argv[i];
    const val = argv[++i];
    if (val === undefined) throw new Error(`${key} expects a value`);
    if (key === '--core-pid') o.corePid = Number(val);
    else if (key === '--worker-port') o.workerPort = Number(val);
    else if (key === '--interval-ms') o.intervalMs = Number(val);
    else if (key === '--rpc-url') o.rpcUrl = val;
    else if (key === '--out') o.out = val;
    else if (key === '--duration-ms') o.durationMs = Number(val);
    else throw new Error(`unknown argument: ${key}`);
  }
  if (!o.workerPort) throw new Error('--worker-port is required');
  return o;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const o = parseArgs(process.argv);
  const sink = o.out ? fs.createWriteStream(o.out, { flags: 'a' }) : process.stdout;
  const sampler = createSampler(o);
  process.on('SIGTERM', () => sampler.stop());
  process.on('SIGINT', () => sampler.stop());
  if (o.durationMs) setTimeout(() => sampler.stop(), o.durationMs);
  sampler.start((s) => sink.write(`${JSON.stringify(s)}\n`)).then(() => {
    if (o.out) sink.end();
  });
}
