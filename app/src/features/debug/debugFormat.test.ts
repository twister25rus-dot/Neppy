import { describe, expect, it } from 'vitest';

import type { DebugStatus, DebugTask } from '../../services/api/debugModeApi';
import {
  canCommitTask,
  canRollbackTask,
  defaultCommitMessage,
  dirtyCount,
  repoBasename,
  shortSha,
} from './debugFormat';

const task = (over: Partial<DebugTask> = {}): DebugTask => ({
  id: 't1',
  request: 'fix the sidebar',
  created_at: '',
  updated_at: '',
  status: 'pass',
  files_changed: ['a.ts'],
  validation: [],
  summary: null,
  checkpoint_id: 'cp-1',
  branch: null,
  commit: null,
  ...over,
});

describe('debugFormat', () => {
  it('takes the last segment of a posix or windows root', () => {
    expect(repoBasename('/Users/a/Neppy')).toBe('Neppy');
    expect(repoBasename('C:\\src\\Neppy\\')).toBe('Neppy');
  });

  it('shortens shas and tolerates null', () => {
    expect(shortSha('0123456789abcdef')).toBe('0123456');
    expect(shortSha(null)).toBe('');
  });

  it('sums every dirty category', () => {
    const status = {
      dirty: { modified: ['a'], added: ['b'], deleted: ['c'], untracked: ['d', 'e'] },
    } as DebugStatus;
    expect(dirtyCount(status)).toBe(5);
  });

  it('prefills the commit message from the first 60 characters of the request', () => {
    expect(defaultCommitMessage('fix   the\nsidebar')).toBe('feat(debug): fix the sidebar');
    const long = defaultCommitMessage('x'.repeat(100));
    expect(long).toBe(`feat(debug): ${'x'.repeat(60)}`);
  });

  it('gates commit and rollback on task state', () => {
    expect(canCommitTask(task())).toBe(true);
    expect(canCommitTask(task({ status: 'editing' }))).toBe(false);
    expect(canCommitTask(task({ status: 'rolled_back' }))).toBe(false);
    expect(canCommitTask(task({ commit: 'abc' }))).toBe(false);
    expect(canCommitTask(task({ files_changed: [] }))).toBe(false);
    expect(canRollbackTask(task())).toBe(true);
    expect(canRollbackTask(task({ checkpoint_id: null }))).toBe(false);
    expect(canRollbackTask(task({ status: 'rolled_back' }))).toBe(false);
    expect(canRollbackTask(task({ status: 'validating' }))).toBe(false);
  });
});
