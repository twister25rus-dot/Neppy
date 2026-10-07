import type { DebugCheckpoint, DebugDiff, DebugTask } from '../../../services/api/debugModeApi';

export const SAMPLE_DIFF_TEXT = [
  'diff --git a/src/a.ts b/src/a.ts',
  '--- a/src/a.ts',
  '+++ b/src/a.ts',
  '@@ -1,2 +1,2 @@',
  ' keep',
  '-old line',
  '+new line',
  'diff --git a/n.txt b/n.txt',
  'new file mode 100644',
  '--- /dev/null',
  '+++ b/n.txt',
  '@@ -0,0 +1 @@',
  '+fresh',
  '',
].join('\n');

export const makeDiff = (over: Partial<DebugDiff> = {}): DebugDiff => ({
  base: 'HEAD',
  text: SAMPLE_DIFF_TEXT,
  truncated: false,
  summary: { modified: 1, created: 2, deleted: 0 },
  files: [
    { path: 'src/a.ts', added: 1, deleted: 1 },
    { path: 'n.txt', added: 1, deleted: 0 },
  ],
  untracked: ['scratch.log'],
  ...over,
});

export const makeTask = (over: Partial<DebugTask> = {}): DebugTask => ({
  id: 't1',
  request: 'Fix the flaky login test',
  created_at: '2026-10-04T10:00:00Z',
  updated_at: '2026-10-04T10:05:00Z',
  status: 'pass',
  files_changed: ['a.ts', 'b.ts'],
  validation: [
    {
      check_id: 'unit',
      command: ['pnpm', 'test'],
      exit_code: 0,
      passed: true,
      timed_out: false,
      duration_ms: 1200,
      at: '2026-10-04T10:04:00Z',
      output_tail: '',
    },
  ],
  summary: 'Fixed the race in login.',
  checkpoint_id: 'cp1',
  branch: 'main',
  commit: null,
  ...over,
});

export const makeCheckpoint = (over: Partial<DebugCheckpoint> = {}): DebugCheckpoint => ({
  id: 'cp1',
  description: 'Before fixing login',
  task_id: 't1',
  project_root: '/repo',
  created_at: '2026-10-04T10:00:00Z',
  head: 'abcdef1234567890',
  branch: 'main',
  snapshot_sha: 'x',
  index_tree: 'y',
  dirty_files: [],
  untracked_files: [],
  ...over,
});
