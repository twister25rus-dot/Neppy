import assert from 'node:assert/strict';
import test from 'node:test';

import {
  analyze, classifySeries, evaluateKillTest, labelSamples, linearFit, mad, median, pearson,
  renderMarkdown, thresholdsFor, windowPeaks,
} from './analyze-cycles.mjs';

// Deterministic noise so the tests do not flake.
function prng(seed) {
  let s = seed;
  return () => { s = (s * 1664525 + 1013904223) % 4294967296; return s / 4294967296 - 0.5; };
}

/**
 * Synthetic phase: per cycle a 60 s work window, then 150 s idle that is
 * loaded for 45 s, unloaded to 120 s, stopped after. `fn` returns the per-cycle
 * level of each series, so a test says exactly what grows.
 */
function synth({ cycles = 12, phase = 'C', levels, noise = 0.5 }) {
  const rnd = prng(7);
  const samples = [];
  const marks = [];
  let t = 1_000_000;
  for (let c = 1; c <= cycles; c += 1) {
    const L = levels(c);
    marks.push({ t, kind: 'work_start', phase, cycle: c, candidate: 'synthetic' });
    for (let i = 0; i < 60; i += 1, t += 1000) samples.push(mk(t, 'busy', L, rnd, noise));
    marks.push({ t, kind: 'work_end', phase, cycle: c, status: 'done', steps: 3 });
    for (let i = 0; i < 150; i += 1, t += 1000) {
      samples.push(mk(t, i < 45 ? 'loaded' : i < 120 ? 'unloaded' : 'stopped', L, rnd, noise));
    }
    marks.push({ t, kind: 'idle_end', phase, cycle: c, workspace_mib: L.workspace ?? 100 });
  }
  return { samples, marks };
}

function mk(t, state, L, rnd, noise) {
  const n = () => rnd() * noise;
  const base = {
    t,
    core: { pid: 1, rss_mib: L.core + 50 + n(), fp_mib: L.core + n(), threads: L.threads ?? 40, fds: L.fds ?? 100 },
    sys: { avail_pct: 60, pressure: 1, used: (L.sys ?? 12000) + n() * 40, file_backed: 9000 + n() * 20, swap_used_mib: 0 },
    gpu_in_use_mib: 100,
  };
  if (state === 'stopped') return { ...base, worker: null };
  const fp = state === 'unloaded' ? L.workerUnloaded : L.workerLoaded;
  return { ...base, worker: { pid: 2, phase: state, fp_mib: fp + n(), rss_mib: fp * 0.6 + n() } };
}

const flat = { core: 200, workerLoaded: 9500, workerUnloaded: 600 };
const verdictOf = (report, phase, series) => report.phases[phase].verdicts.find((v) => v.series === series);

test('stats helpers', () => {
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 2, 3]), 2.5);
  assert.equal(mad([1, 2, 3, 4, 100]), 1);
  assert.ok(Math.abs(linearFit([0, 1, 2, 3], [1, 3, 5, 7]).slope - 2) < 1e-9);
  assert.ok(pearson([1, 2, 3, 4], [2, 4, 6, 8.1]) > 0.999);
  assert.equal(pearson([1, 2], [1, 2]), null);
});

test('flat series are no-growth', () => {
  const { samples, marks } = synth({ levels: () => flat });
  const r = analyze({ samples, marks });
  for (const v of r.phases.C.verdicts) assert.equal(v.verdict, 'no-growth', `${v.series}: ${v.verdict}`);
  assert.equal(r.phases.C.noLeakStatementAllowed, true);
  assert.match(r.phases.C.noLeakStatement, /not proof/);
});

test('steady linear worker growth is a leak, with the note that /unload did not free it', () => {
  const { samples, marks } = synth({ levels: (c) => ({ ...flat, workerLoaded: 9500 + c * 120, workerUnloaded: 600 + c * 120 }) });
  marks.push({ t: 1, kind: 'probe', phase: 'C', name: 'baseline_unloaded', fp_mib: 600 });
  marks.push({ t: 2, kind: 'probe', phase: 'C', name: 'end_after_unload', fp_mib: 600 + 12 * 120 });
  const r = analyze({ samples, marks });
  const v = verdictOf(r, 'C', 'worker.fp.loaded');
  assert.equal(v.verdict, 'leak-suspected');
  assert.match(v.note, /not freed by \/unload/);
  assert.equal(r.phases.C.noLeakStatementAllowed, false);
  assert.equal(r.phases.C.noLeakStatement, null);
});

test('growth that an /unload brings back is retained-by-runtime, not a leak', () => {
  const { samples, marks } = synth({ levels: (c) => ({ ...flat, workerLoaded: 9500 + c * 120 }) });
  marks.push({ t: 1, kind: 'probe', phase: 'C', name: 'baseline_unloaded', fp_mib: 600 });
  marks.push({ t: 2, kind: 'probe', phase: 'C', name: 'end_after_unload', fp_mib: 610 });
  const v = verdictOf(analyze({ samples, marks }), 'C', 'worker.fp.loaded');
  assert.equal(v.verdict, 'retained-by-runtime');
  assert.match(v.note, /within 5%/);
});

test('growth that levels off is a plateau', () => {
  const { samples, marks } = synth({ levels: (c) => ({ ...flat, core: 200 + Math.min(c, 5) * 20 }) });
  const v = verdictOf(analyze({ samples, marks }), 'C', 'core.fp.idle');
  assert.equal(v.verdict, 'plateau');
});

test('warm-up cycle is excluded: a big first-cycle jump alone is no-growth', () => {
  const { samples, marks } = synth({ levels: (c) => ({ ...flat, core: c === 1 ? 600 : 200 }) });
  const v = verdictOf(analyze({ samples, marks }), 'C', 'core.fp.idle');
  assert.equal(v.verdict, 'no-growth');
  assert.equal(v.baseline < 210, true);
});

test('core growth that tracks workspace size is confounded', () => {
  const { samples, marks } = synth({ levels: (c) => ({ ...flat, core: 200 + c * 6, workspace: 100 + c * 12 }) });
  const v = verdictOf(analyze({ samples, marks }), 'C', 'core.fp.idle');
  assert.equal(v.verdict, 'confounded');
  assert.match(v.note, /workspace/);
});

test('core growth with a flat workspace is a leak', () => {
  const { samples, marks } = synth({ levels: (c) => ({ ...flat, core: 200 + c * 6 }) });
  assert.equal(verdictOf(analyze({ samples, marks }), 'C', 'core.fp.idle').verdict, 'leak-suspected');
});

test('slow growth under the threshold is reported, and blocks the no-leak statement', () => {
  const { samples, marks } = synth({ levels: (c) => ({ ...flat, workerLoaded: 9500 + c * 10 }) });
  const r = analyze({ samples, marks });
  assert.equal(verdictOf(r, 'C', 'worker.fp.loaded').verdict, 'growth-below-threshold');
  assert.equal(r.phases.C.noLeakStatementAllowed, false);
});

test('thread counts that only climb are flagged; a jittering count is not', () => {
  const up = synth({ levels: (c) => ({ ...flat, threads: 40 + c * 2 }) });
  assert.equal(verdictOf(analyze(up), 'C', 'core.threads').verdict, 'leak-suspected');
  const wob = synth({ levels: (c) => ({ ...flat, threads: 40 + (c % 3) }) });
  assert.equal(verdictOf(analyze(wob), 'C', 'core.threads').verdict, 'no-growth');
});

test('system growth with flat processes is OS retention; with a process leak it is confounded', () => {
  const os = synth({ levels: (c) => ({ ...flat, sys: 12000 + c * 400 }) });
  assert.equal(verdictOf(analyze(os), 'C', 'sys.used.stopped').verdict, 'retained-by-os');
  const both = synth({ levels: (c) => ({ ...flat, core: 200 + c * 6, sys: 12000 + c * 400 }) });
  assert.equal(verdictOf(analyze(both), 'C', 'sys.used.stopped').verdict, 'confounded');
});

test('too few cycles gives insufficient-data and no claim', () => {
  const { samples, marks } = synth({ cycles: 3, levels: () => flat });
  const r = analyze({ samples, marks });
  assert.equal(r.phases.C.verdicts[0].verdict, 'insufficient-data');
  assert.equal(r.phases.C.noLeakStatementAllowed, false);
});

test('fewer than 10 cycles never allows the statement even when flat', () => {
  const { samples, marks } = synth({ cycles: 8, levels: () => flat });
  assert.equal(analyze({ samples, marks }).phases.C.noLeakStatementAllowed, false);
});

test('labelSamples tracks how long a state has held', () => {
  const l = labelSamples([
    { t: 0, worker: null }, { t: 1000, worker: null },
    { t: 2000, worker: { phase: 'loaded' } }, { t: 8000, worker: { phase: 'loaded' } },
  ]);
  assert.deepEqual(l.map((s) => s.label), ['stopped', 'stopped', 'loaded', 'loaded']);
  assert.equal(l[3].stableMs, 6000);
});

test('windowPeaks and thresholds', () => {
  const p = windowPeaks([
    { worker: { fp_mib: 100 }, sys: { avail_pct: 50, pressure: 1 } },
    { worker: { fp_mib: 300 }, sys: { avail_pct: 31, pressure: 2 } },
  ]);
  assert.equal(p.peak_worker_fp_mib, 300);
  assert.equal(p.min_avail_pct, 31);
  assert.equal(p.max_pressure_level, 2);
  assert.equal(thresholdsFor('core.threads').kind, 'count');
  assert.equal(thresholdsFor('worker.fp.loaded').perCycle, 64);
});

test('context scaling compares the footprint step with the 32 KiB/token prediction', () => {
  const samples = [];
  for (let t = 0; t < 60000; t += 1000) {
    const during = t >= 20000 && t <= 40000;
    samples.push({ t, worker: { phase: during ? 'busy' : 'loaded', fp_mib: during ? 9900 : 9500 }, gpu_in_use_mib: during ? 9700 : 9300 });
  }
  const marks = [
    { t: 20000, kind: 'ctx_start', target: 8000 },
    { t: 40000, kind: 'ctx_end', target: 8000, prompt_tokens: 8192, ok: true },
  ];
  const [row] = analyze({ samples, marks }).contextScaling;
  assert.equal(Math.round(row.delta_mib), 400);
  assert.equal(Math.round(row.predicted_kv_mib), 256);
  assert.equal(Math.round(row.gpu_delta_mib), 400);
});

test('kill test: passes when everything holds, names the failing criterion otherwise', () => {
  const good = evaluateKillTest({
    killMs: 10_000,
    events: [{ event: 'worker_crash', ts_ms: 10_800 }, { event: 'worker_restart', ts_ms: 14_000 }],
    taskStatus: 'done', testRuns: { invocations: 2, ledgerRows: 2 }, stackedNoteLines: 0, orphanWorkerPids: [],
  });
  assert.equal(good.pass, true);
  const slow = evaluateKillTest({
    killMs: 10_000,
    events: [{ event: 'worker_crash', ts_ms: 17_000 }, { event: 'worker_restart', ts_ms: 18_000 }],
    taskStatus: 'failed', testRuns: { invocations: 2, ledgerRows: 1 }, stackedNoteLines: 1, orphanWorkerPids: [123],
  });
  assert.equal(slow.pass, false);
  assert.equal(slow.checks.filter((c) => !c.pass).length, 5);
  const none = evaluateKillTest({ killMs: 1, events: [], taskStatus: 'budget_exhausted', testRuns: { invocations: 0, ledgerRows: 0 }, stackedNoteLines: 0, orphanWorkerPids: [] });
  assert.equal(none.pass, false); // no crash event recorded
  assert.equal(none.checks.find((c) => c.name.startsWith('test command')).exercised, false);
});

test('classifySeries rejects a noisy-but-flat series and renderMarkdown mentions verdicts', () => {
  const noisy = classifySeries('worker.fp.loaded', [9500, 9520, 9480, 9510, 9490, 9505], [9500, 9510, 9490, 9520, 9480], {});
  assert.equal(noisy.verdict, 'no-growth');
  const { samples, marks } = synth({ levels: () => flat });
  const md = renderMarkdown(analyze({ samples, marks }));
  assert.match(md, /Phase C/);
  assert.match(md, /\*\*no-growth\*\*/);
});

test('without an explicit probe, the unloaded baseline comes from the first measured C cycle', () => {
  const c = synth({ phase: 'C', levels: () => flat });
  const d = synth({ phase: 'D', levels: (k) => ({ ...flat, workerLoaded: 9500 + k * 120 }) });
  const shift = c.samples[c.samples.length - 1].t + 1000;
  const samples = [...c.samples, ...d.samples.map((x) => ({ ...x, t: x.t + shift }))];
  const marks = [...c.marks, ...d.marks.map((m) => ({ ...m, t: m.t + shift }))];
  marks.push({ t: shift + 1, kind: 'probe', phase: 'D', name: 'end_after_unload', fp_mib: 610 });
  const v = analyze({ samples, marks }).phases.D.verdicts.find((x) => x.series === 'worker.fp.loaded');
  assert.equal(v.verdict, 'retained-by-runtime');
});
