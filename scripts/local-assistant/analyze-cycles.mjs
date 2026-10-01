#!/usr/bin/env node
/**
 * Cycle analyzer for the local-assistant soak: leak vs retention verdicts.
 *
 * Input is the sampler's JSONL series plus the driver's marks (when each
 * work/idle window of each cycle began and ended). Kept apart from the run so a
 * recorded soak can be re-analyzed with other thresholds.
 *
 * WHAT A VERDICT MEANS
 *
 * Cycle 1 of a phase is warm-up (first load, first index build) and excluded.
 * The per-cycle value of a series is the median of its stable idle samples; the
 * baseline is cycle 2. For each series over cycles 2..N:
 *   - growth     = mean(last third) - mean(first third), judged against noise
 *                  (3x MAD of cycle-2 samples, with a per-series floor)
 *   - lateGrowth = mean(last third) - mean(middle third): growth that is still
 *                  going on. Growth that stops is a plateau, which is what a
 *                  cache or an allocator arena looks like.
 *   - slope      = least-squares MiB per cycle, compared with a threshold.
 * Verdicts: no-growth | plateau | growth-below-threshold | retained-by-runtime
 * | retained-by-os | leak-suspected | confounded | insufficient-data.
 *
 * "Retained, not leaked" needs evidence beyond the slope. A worker series that
 * is still rising is `retained-by-runtime` only if an explicit /unload (which
 * runs mx.clear_cache) brought the footprint back within 5% of the unloaded
 * baseline. Growth freed only by stopping the process stays `leak-suspected`:
 * the process-stop policy bounds it, but nothing here proves it harmless. Core
 * growth that tracks the workspace (index/state DB) is `confounded`.
 *
 * Threads and fds have no legitimate reason to climb under steady work, so they
 * use a stricter monotonic rule.
 *
 * A "no leak" statement is only offered when a phase has >= 10 cycles and every
 * process series is no-growth or plateau, and it is scoped to this run.
 *
 * Usage: node analyze-cycles.mjs --samples s.jsonl --marks m.jsonl [--out report.json] [--md report.md]
 */

import fs from 'node:fs';

export const MIN_CYCLES_FOR_CLAIM = 10;
export const KV_BYTES_PER_TOKEN_PREDICTED = 32 * 1024; // plan section 0: 8 attention layers x 2 x 4 x 256 x 2 B

export const mean = (a) => (a.length ? a.reduce((x, y) => x + y, 0) / a.length : null);
export function median(a) {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}
export const mad = (a) => {
  const m = median(a);
  return m === null ? null : median(a.map((v) => Math.abs(v - m)));
};
export function linearFit(xs, ys) {
  const n = xs.length;
  if (n < 2) return { slope: null };
  const mx = mean(xs);
  const my = mean(ys);
  let sxy = 0;
  let sxx = 0;
  for (let i = 0; i < n; i += 1) {
    sxy += (xs[i] - mx) * (ys[i] - my);
    sxx += (xs[i] - mx) ** 2;
  }
  return { slope: sxx === 0 ? null : sxy / sxx };
}
export function pearson(xs, ys) {
  const n = xs.length;
  if (n < 3) return null;
  const mx = mean(xs);
  const my = mean(ys);
  let sxy = 0;
  let sxx = 0;
  let syy = 0;
  for (let i = 0; i < n; i += 1) {
    sxy += (xs[i] - mx) * (ys[i] - my);
    sxx += (xs[i] - mx) ** 2;
    syy += (ys[i] - my) ** 2;
  }
  return sxx === 0 || syy === 0 ? null : sxy / Math.sqrt(sxx * syy);
}

/** Per-series thresholds. perCycle is MiB per cycle; floor is the minimum noise band in MiB. */
export function thresholdsFor(series) {
  if (series.startsWith('core.threads')) return { kind: 'count', maxGrowth: 8 };
  if (series.startsWith('core.fds')) return { kind: 'count', maxGrowth: 32 };
  if (series.startsWith('core.')) return { kind: 'mib', perCycle: 2, floor: 3 };
  if (series.startsWith('worker.fp.unloaded') || series.startsWith('worker.rss.unloaded')) return { kind: 'mib', perCycle: 32, floor: 32 };
  if (series.startsWith('worker.')) return { kind: 'mib', perCycle: 64, floor: 32 };
  return { kind: 'mib', perCycle: 128, floor: 256 }; // sys.*: shared with the user's other apps
}

/** Label each sample by worker state and how long it has held that state. */
export function labelSamples(samples) {
  let prev = null;
  let since = 0;
  return samples.map((s) => {
    const label = s.worker ? (s.worker.phase === 'loaded' || s.worker.phase === 'unloaded' ? s.worker.phase : 'active') : 'stopped';
    if (label !== prev) { prev = label; since = s.t; }
    return { ...s, label, stableMs: s.t - since };
  });
}

export function buildCycles(marks) {
  const out = {};
  for (const m of marks) {
    if (m.cycle == null || !m.phase) continue;
    const c = ((out[m.phase] ??= {})[m.cycle] ??= { cycle: m.cycle });
    if (m.kind === 'work_start') { c.workStart = m.t; c.candidate = m.candidate ?? c.candidate; }
    if (m.kind === 'work_end') { c.workEnd = m.t; c.status = m.status; c.steps = m.steps; }
    if (m.kind === 'idle_end') { c.idleEnd = m.t; c.workspaceMib = m.workspace_mib ?? null; }
  }
  return out;
}

const pick = {
  'core.fp.idle': (s) => s.core?.fp_mib,
  'core.rss.idle': (s) => s.core?.rss_mib,
  'core.threads': (s) => s.core?.threads,
  'core.fds': (s) => s.core?.fds,
};
const byLabel = {
  loaded: { 'worker.fp.loaded': (s) => s.worker?.fp_mib, 'worker.rss.loaded': (s) => s.worker?.rss_mib },
  unloaded: { 'worker.fp.unloaded': (s) => s.worker?.fp_mib, 'worker.rss.unloaded': (s) => s.worker?.rss_mib },
  stopped: { 'sys.used.stopped': (s) => s.sys?.used, 'sys.file_backed.stopped': (s) => s.sys?.file_backed },
};

/** Series medians and raw samples for one cycle's idle window. */
export function cycleSeries(labeled, win, settleMs = 5000) {
  const idle = labeled.filter((s) => s.t >= win.workEnd && s.t <= win.idleEnd && s.stableMs >= settleMs);
  const out = {};
  const add = (name, fn, rows) => {
    const raw = rows.map(fn).filter((v) => typeof v === 'number' && Number.isFinite(v));
    if (raw.length >= 3) out[name] = { value: median(raw), raw };
  };
  for (const [name, fn] of Object.entries(pick)) add(name, fn, idle);
  for (const [label, fns] of Object.entries(byLabel)) {
    for (const [name, fn] of Object.entries(fns)) add(name, fn, idle.filter((s) => s.label === label));
  }
  return out;
}

/** Peaks over a window of samples (work + idle, every state). */
export function windowPeaks(samples) {
  const maxOf = (fn) => { const v = samples.map(fn).filter((x) => typeof x === 'number'); return v.length ? Math.max(...v) : null; };
  const minOf = (fn) => { const v = samples.map(fn).filter((x) => typeof x === 'number'); return v.length ? Math.min(...v) : null; };
  return {
    peak_worker_fp_mib: maxOf((s) => s.worker?.fp_mib),
    peak_worker_rss_mib: maxOf((s) => s.worker?.rss_mib),
    peak_core_fp_mib: maxOf((s) => s.core?.fp_mib),
    peak_core_rss_mib: maxOf((s) => s.core?.rss_mib),
    peak_sys_used_mib: maxOf((s) => s.sys?.used),
    min_avail_pct: minOf((s) => s.sys?.avail_pct),
    max_pressure_level: maxOf((s) => s.sys?.pressure),
    peak_gpu_in_use_mib: maxOf((s) => s.gpu_in_use_mib),
    peak_gpu_alloc_mib: maxOf((s) => s.gpu_alloc_mib),
    peak_swap_used_mib: maxOf((s) => s.sys?.swap_used_mib),
  };
}

const thirds = (v) => { const k = Math.max(1, Math.floor(v.length / 3)); return [v.slice(0, k), v.slice(k, v.length - k), v.slice(v.length - k)]; };

/** The verdict for one series. `values` are per-cycle values for cycles 2..N. */
export function classifySeries(name, values, baselineRaw, ctx = {}) {
  const th = thresholdsFor(name);
  const n = values.length;
  const base = { series: name, cycles: n, baseline: values[0] ?? null, final: values[n - 1] ?? null, values };
  if (n < 4) return { ...base, verdict: 'insufficient-data', note: `${n} cycles after warm-up; need at least 4` };
  const [first, mid, last] = thirds(values);
  const growth = mean(last) - mean(first);
  const lateGrowth = mean(last) - mean(mid.length ? mid : first);
  const { slope } = linearFit(values.map((_, i) => i), values);
  const noise = th.kind === 'count' ? 0 : Math.max(3 * (mad(baselineRaw ?? []) ?? 0), th.floor);
  const ev = { slope, growth, lateGrowth, noise };

  if (th.kind === 'count') {
    let rises = 0;
    for (let i = 1; i < n; i += 1) if (values[i] > values[i - 1]) rises += 1;
    const net = values[n - 1] - values[0];
    const monotonic = rises >= Math.ceil(0.8 * (n - 1)) && values.every((v, i) => i === 0 || v >= values[i - 1] - 0.5);
    if (net > th.maxGrowth && monotonic) return { ...base, ...ev, verdict: 'leak-suspected', note: `rose in ${rises}/${n - 1} steps, net +${net}` };
    if (net > th.maxGrowth) return { ...base, ...ev, verdict: 'growth-below-threshold', note: `net +${net}, not monotonic` };
    return { ...base, ...ev, verdict: 'no-growth' };
  }
  if (growth <= noise) return { ...base, ...ev, verdict: 'no-growth' };
  if (lateGrowth <= noise) return { ...base, ...ev, verdict: 'plateau', note: 'grew, then levelled off' };
  if (slope <= th.perCycle) return { ...base, ...ev, verdict: 'growth-below-threshold', note: `${slope.toFixed(2)} MiB/cycle, still rising, under the ${th.perCycle} MiB/cycle call` };

  // Still rising and over threshold: look for what explains it.
  const ws = ctx.workspaceSeries;
  if (name.startsWith('core.') && ws && ws.length === n) {
    const r = pearson(ws, values);
    if (r !== null && r >= 0.9 && ws[n - 1] - ws[0] >= 4) {
      return { ...base, ...ev, verdict: 'confounded', note: `tracks workspace size (r=${r.toFixed(2)}, +${(ws[n - 1] - ws[0]).toFixed(1)} MiB on disk)` };
    }
  }
  if (name.startsWith('worker.') && ctx.probes) {
    const p = ctx.probes;
    const near = (a, b) => a != null && b != null && a <= b * 1.05;
    if (name.includes('.loaded') && near(p.end_after_unload, p.baseline_unloaded)) {
      return { ...base, ...ev, verdict: 'retained-by-runtime', note: `after /unload the worker was ${p.end_after_unload.toFixed(0)} MiB vs ${p.baseline_unloaded.toFixed(0)} MiB unloaded baseline (within 5%): freed without a restart` };
    }
    const respawned = near(p.end_after_respawn_loaded, base.baseline);
    return { ...base, ...ev, verdict: 'leak-suspected', note: respawned ? 'not freed by /unload; freed only by stopping the process' : 'not freed by /unload' };
  }
  return { ...base, ...ev, verdict: 'leak-suspected', note: `${slope.toFixed(2)} MiB/cycle, still rising` };
}

/** Analyze every phase that has cycles. */
export function analyze({ samples, marks, opts = {} }) {
  const labeled = labelSamples(samples.filter((s) => s.t && !s.error));
  const cycles = buildCycles(marks);
  const probesByPhase = {};
  for (const m of marks) {
    if (m.kind === 'probe' && m.phase) (probesByPhase[m.phase] ??= {})[m.name] = m.fp_mib;
  }
  // The unloaded-state footprint of the first measured cycle of C is the
  // reference for "did /unload bring the worker back down" in other phases.
  const cFirst = Object.values(cycles.C ?? {}).filter((c) => c.workEnd && c.idleEnd).sort((a, b) => a.cycle - b.cycle)[1];
  if (cFirst) {
    const v = cycleSeries(labeled, cFirst, opts.settleMs)['worker.fp.unloaded']?.value;
    for (const ph of Object.keys(cycles)) if (v != null) (probesByPhase[ph] ??= {}).baseline_unloaded ??= v;
  }
  const phases = {};
  for (const [phase, byCycle] of Object.entries(cycles)) {
    const complete = Object.values(byCycle).filter((c) => c.workStart && c.workEnd && c.idleEnd).sort((a, b) => a.cycle - b.cycle);
    if (!complete.length) continue;
    const per = complete.map((c) => ({
      cycle: c.cycle, status: c.status, steps: c.steps, workspaceMib: c.workspaceMib,
      work_secs: (c.workEnd - c.workStart) / 1000,
      peaks: windowPeaks(labeled.filter((s) => s.t >= c.workStart && s.t <= c.idleEnd)),
      series: cycleSeries(labeled, c, opts.settleMs),
    }));
    const measured = per.slice(1); // cycle 1 is warm-up
    const names = new Set(measured.flatMap((c) => Object.keys(c.series)));
    const ws = measured.map((c) => c.workspaceMib);
    const workspaceSeries = ws.every((v) => typeof v === 'number') ? ws : null;
    const verdicts = [];
    for (const name of [...names].sort()) {
      if (!measured.every((c) => c.series[name])) continue;
      verdicts.push(classifySeries(name, measured.map((c) => c.series[name].value), measured[0].series[name].raw, { workspaceSeries, probes: probesByPhase[phase] }));
    }
    const procVerdicts = verdicts.filter((v) => !v.series.startsWith('sys.'));
    const leaky = procVerdicts.some((v) => v.verdict === 'leak-suspected');
    for (const v of verdicts) {
      if (v.series.startsWith('sys.') && v.verdict === 'leak-suspected') {
        v.verdict = leaky ? 'confounded' : 'retained-by-os';
        v.note = leaky ? 'system growth coincides with a process-level leak call' : 'system used/file cache grew while process footprints did not: OS retention';
      }
    }
    const claimable = complete.length >= MIN_CYCLES_FOR_CLAIM && procVerdicts.length > 0 && procVerdicts.every((v) => ['no-growth', 'plateau'].includes(v.verdict));
    phases[phase] = {
      cycles: complete.length,
      candidate: complete[0].candidate ?? null,
      peaks: windowPeaks(labeled.filter((s) => s.t >= complete[0].workStart && s.t <= complete[complete.length - 1].idleEnd)),
      perCycle: per,
      verdicts,
      noLeakStatementAllowed: claimable,
      noLeakStatement: claimable ? `No growth beyond noise was observed in any core or worker series over ${complete.length} cycles of this workload on this machine in this run. This is not proof the code is leak-free.` : null,
    };
  }
  return { phases, contextScaling: contextScaling(labeled, marks), sampleCount: samples.length };
}

/** Footprint added by a prompt of a given size, against the 32 KiB/token prediction. */
export function contextScaling(labeled, marks) {
  const rows = [];
  for (const start of marks.filter((m) => m.kind === 'ctx_start')) {
    const end = marks.find((m) => m.kind === 'ctx_end' && m.target === start.target && m.t >= start.t);
    if (!end) continue;
    const before = median(labeled.filter((s) => s.t >= start.t - 10000 && s.t < start.t && s.worker?.phase === 'loaded').map((s) => s.worker.fp_mib));
    const win = labeled.filter((s) => s.t >= start.t && s.t <= end.t + 3000 && s.worker);
    const peak = win.length ? Math.max(...win.map((s) => s.worker.fp_mib ?? 0)) : null;
    const gpuBefore = median(labeled.filter((s) => s.t >= start.t - 10000 && s.t < start.t).map((s) => s.gpu_in_use_mib).filter((v) => v != null));
    const gpuPeak = Math.max(0, ...win.map((s) => s.gpu_in_use_mib ?? 0));
    const tokens = end.prompt_tokens ?? start.target;
    rows.push({
      target_tokens: start.target, prompt_tokens: end.prompt_tokens ?? null, ok: end.ok !== false,
      fp_before_mib: before, fp_peak_mib: peak, delta_mib: before != null && peak != null ? peak - before : null,
      predicted_kv_mib: (tokens * KV_BYTES_PER_TOKEN_PREDICTED) / 1048576,
      gpu_delta_mib: gpuBefore != null && gpuPeak ? gpuPeak - gpuBefore : null,
    });
  }
  return rows;
}

/** Pass criteria for the worker kill test (plan section 3, kill test). */
export function evaluateKillTest({ killMs, events, taskStatus, testRuns, stackedNoteLines, editsApplied, orphanWorkerPids }) {
  const terminal = ['done', 'budget_exhausted'].includes(taskStatus);
  const runs = testRuns ?? { invocations: 0, ledgerRows: 0 };
  const crash = events.find((e) => e.event === 'worker_crash' && e.ts_ms >= killMs - 1000);
  const restart = events.find((e) => e.event === 'worker_restart' && e.ts_ms >= killMs - 1000);
  const checks = [
    { name: 'worker_crash recorded within 5 s of the kill', pass: !!crash && crash.ts_ms - killMs <= 5000, detail: crash ? `${crash.ts_ms - killMs} ms after kill` : 'no worker_crash event' },
    { name: 'worker_restart followed', pass: !!restart && (!crash || restart.ts_ms >= crash.ts_ms), detail: restart ? `${restart.ts_ms - killMs} ms after kill` : 'no worker_restart event' },
    { name: 'task reached a clean terminal state (done, or budget_exhausted at its step cap)', pass: terminal, detail: `status=${taskStatus}` },
    {
      name: 'test command ran once per step it was requested',
      pass: runs.invocations === runs.ledgerRows,
      exercised: runs.ledgerRows > 0,
      detail: runs.ledgerRows > 0 ? `${runs.invocations} invocations for ${runs.ledgerRows} ledger rows` : 'no step requested the test command: not exercised',
    },
    {
      // A run in which no edit landed cannot show that edits are applied once:
      // zero stacked lines is then vacuously true, so say it was not exercised.
      name: 'each edit applied once (no stacked NOTE(soak) lines)',
      pass: stackedNoteLines === 0,
      exercised: (editsApplied ?? 0) > 0,
      detail: (editsApplied ?? 0) > 0 ? `${stackedNoteLines} stacked across ${editsApplied} applied edit(s)` : 'no edit was applied: not exercised',
    },
    { name: 'no orphan mlx worker', pass: (orphanWorkerPids ?? []).length === 0, detail: JSON.stringify(orphanWorkerPids ?? []) },
  ];
  return { pass: checks.every((c) => c.pass), checks };
}

const f = (v, d = 0) => (v == null ? 'n/a' : Number(v).toFixed(d));

export function renderMarkdown(report) {
  const out = ['## Analyzer output', ''];
  for (const [phase, p] of Object.entries(report.phases)) {
    const k = p.peaks;
    out.push(`### Phase ${phase} (${p.candidate ?? 'candidate n/a'}, ${p.cycles} cycles)`, '');
    out.push('| worker fp peak | worker rss peak | core fp peak | system used peak | min avail % | max pressure | GPU in use peak | GPU alloc peak | swap peak |', '|---|---|---|---|---|---|---|---|---|');
    out.push(`| ${f(k.peak_worker_fp_mib)} MiB | ${f(k.peak_worker_rss_mib)} MiB | ${f(k.peak_core_fp_mib)} MiB | ${f(k.peak_sys_used_mib)} MiB | ${f(k.min_avail_pct)} | ${k.max_pressure_level ?? 'n/a'} | ${f(k.peak_gpu_in_use_mib)} MiB | ${f(k.peak_gpu_alloc_mib)} MiB | ${f(k.peak_swap_used_mib)} MiB |`, '');
    out.push('| series | verdict | baseline (cycle 2) | final | slope MiB/cycle | growth | noise | note |', '|---|---|---|---|---|---|---|---|');
    for (const v of p.verdicts) out.push(`| ${v.series} | **${v.verdict}** | ${f(v.baseline, 1)} | ${f(v.final, 1)} | ${f(v.slope, 2)} | ${f(v.growth, 1)} | ${f(v.noise, 1)} | ${v.note ?? ''} |`);
    out.push('', 'Per-cycle delta vs cycle 2 (MiB):', '', `| cycle | ${p.verdicts.map((v) => v.series).join(' | ')} |`, `|---|${p.verdicts.map(() => '---').join('|')}|`);
    p.perCycle.slice(1).forEach((c, i) => out.push(`| ${c.cycle} | ${p.verdicts.map((v) => f(v.values[i] - v.values[0], 1)).join(' | ')} |`));
    out.push('', p.noLeakStatement ?? `No "no leak" statement is offered for phase ${phase}: it needs >= ${MIN_CYCLES_FOR_CLAIM} cycles and no-growth or plateau on every process series.`, '');
  }
  if (report.contextScaling.length) {
    out.push('### Context scaling', '', '| target tokens | prompt tokens | footprint before | peak | delta | predicted KV | GPU delta |', '|---|---|---|---|---|---|---|');
    for (const r of report.contextScaling) out.push(`| ${r.target_tokens} | ${r.prompt_tokens ?? 'n/a'} | ${f(r.fp_before_mib)} | ${f(r.fp_peak_mib)} | ${f(r.delta_mib)} | ${f(r.predicted_kv_mib)} | ${f(r.gpu_delta_mib)} |`);
    out.push('');
  }
  return out.join('\n');
}

function readJsonl(file) {
  return fs.readFileSync(file, 'utf8').split('\n').filter((l) => l.trim()).flatMap((l) => { try { return [JSON.parse(l)]; } catch { return []; } });
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const a = process.argv.slice(2);
  const get = (k) => { const i = a.indexOf(k); return i >= 0 ? a[i + 1] : null; };
  if (!get('--samples') || !get('--marks')) { console.error('usage: analyze-cycles.mjs --samples s.jsonl --marks m.jsonl [--out r.json] [--md r.md]'); process.exit(2); }
  const report = analyze({ samples: readJsonl(get('--samples')), marks: readJsonl(get('--marks')) });
  if (get('--out')) fs.writeFileSync(get('--out'), JSON.stringify(report, null, 2));
  if (get('--md')) fs.writeFileSync(get('--md'), renderMarkdown(report));
  console.log(renderMarkdown(report));
}
