import { act, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { DebugThreadChrome } from './DebugThreadChrome';
import { DEBUG_POLL_MS } from './useDebugSnapshot';

// Records every `refreshKey` the chrome hands the release card, so the wiring
// (which repo facts re-run the release preflight) can be asserted directly.
const releaseKeys = vi.hoisted(() => [] as Array<string | number | undefined>);
vi.mock('./ReleaseCard', () => ({
  ReleaseCard: ({ refreshKey }: { refreshKey?: string | number }) => {
    releaseKeys.push(refreshKey);
    return <div data-testid="debug-release" />;
  },
}));

const mockCall = vi.fn();
vi.mock('../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCall(...args),
}));
vi.mock('../../services/analytics', () => ({ trackEvent: vi.fn() }));

const STATUS = {
  project_root: '/Users/alex/Neppy',
  branch: 'feat/debug-mode',
  head: '0123456789abcdef',
  dirty: { modified: ['a', 'b'], added: [], deleted: ['c'], untracked: ['d'] },
  task_active: false,
  active_task_id: null,
};

const TASK = {
  id: 'task-1',
  request: 'Fix the sidebar overflow',
  created_at: '',
  updated_at: '',
  status: 'pass',
  files_changed: ['a.ts'],
  validation: [],
  summary: null,
  checkpoint_id: 'cp-1',
  branch: 'main',
  commit: null,
};

let statusFails: boolean;
let currentStatus: typeof STATUS;
let currentTask: Omit<typeof TASK, 'commit'> & { commit: string | null };

describe('DebugThreadChrome', () => {
  beforeEach(() => {
    mockCall.mockReset();
    statusFails = false;
    currentStatus = STATUS;
    currentTask = TASK;
    releaseKeys.length = 0;
    mockCall.mockImplementation(async ({ method }) => {
      if (method === 'neppy.debug_mode_status') {
        if (statusFails) throw new Error('no repo');
        return currentStatus;
      }
      if (method === 'neppy.debug_mode_task_list') return [currentTask];
      if (method === 'neppy.debug_mode_diff') {
        return {
          base: 'HEAD',
          text: '',
          truncated: false,
          summary: { modified: 0, created: 0, deleted: 0 },
          files: [],
          untracked: [],
        };
      }
      return [];
    });
  });

  it('shows the compact banner, the last task and the collapsible panels', async () => {
    renderWithProviders(<DebugThreadChrome threadId="t-1" />);

    expect(await screen.findByTestId('debug-banner')).toBeInTheDocument();
    expect(screen.getByTestId('debug-dirty')).toHaveTextContent('4');
    // Compact: the repository and HEAD facts are left to the settings page.
    expect(screen.queryByText('Repository:')).toBeNull();
    expect(screen.getByRole('link', { name: 'Debug Mode settings' })).toHaveAttribute(
      'href',
      '/settings/debug-mode'
    );
    await waitFor(() => expect(screen.getByText('Last task')).toBeInTheDocument());
    expect(screen.getByTestId('debug-panels-slot')).toBeInTheDocument();
    // The release card sits right after the local-install card.
    const install = screen.getByTestId('debug-local-install');
    expect(screen.getByTestId('debug-release')).toBe(install.nextElementSibling);
  });

  it('says what is missing, with a settings link, when there is no source repository', async () => {
    statusFails = true;
    renderWithProviders(<DebugThreadChrome threadId="t-1" />);

    const notice = await screen.findByTestId('debug-unavailable');
    expect(notice).toHaveTextContent('Neppy source repository');
    expect(screen.queryByTestId('debug-banner')).toBeNull();
    expect(screen.queryByTestId('debug-release')).toBeNull();
    expect(notice.querySelector('a')).toHaveAttribute('href', '/settings/debug-mode');
  });

  describe('release card refresh key', () => {
    const lastKey = () => releaseKeys[releaseKeys.length - 1];

    beforeEach(() => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
    });
    afterEach(() => {
      vi.useRealTimers();
    });

    /** Applies a repo change, lets the snapshot poll pick it up, returns the new key. */
    async function pollWith(status: typeof STATUS, task: typeof currentTask) {
      currentStatus = status;
      currentTask = task;
      await act(async () => {
        await vi.advanceTimersByTimeAsync(DEBUG_POLL_MS + 50);
      });
      return lastKey();
    }

    async function mountSettled() {
      renderWithProviders(<DebugThreadChrome threadId="t-1" />);
      await screen.findByTestId('debug-release');
      return lastKey();
    }

    it('passes a numeric key derived from the repo state', async () => {
      const base = await mountSettled();
      expect(typeof base).toBe('number');
      expect(base).not.toBe(0);
    });

    it('changes when HEAD changes', async () => {
      const base = await mountSettled();
      expect(await pollWith({ ...STATUS, head: 'fedcba9876543210' }, TASK)).not.toBe(base);
    });

    it('changes when the branch changes', async () => {
      const base = await mountSettled();
      expect(await pollWith({ ...STATUS, branch: 'feat/other' }, TASK)).not.toBe(base);
    });

    it('changes when the tree goes from dirty to clean', async () => {
      const base = await mountSettled();
      const clean = { ...STATUS, dirty: { modified: [], added: [], deleted: [], untracked: [] } };
      expect(await pollWith(clean, TASK)).not.toBe(base);
    });

    it("changes when the last task's commit changes", async () => {
      const base = await mountSettled();
      expect(await pollWith(STATUS, { ...TASK, commit: 'abc1234' })).not.toBe(base);
    });

    it('does not change when only the dirty-file count changes', async () => {
      const base = await mountSettled();
      const moreDirty = {
        ...STATUS,
        dirty: { ...STATUS.dirty, modified: ['a', 'b', 'x', 'y'], untracked: [] },
      };
      const before = mockCall.mock.calls.filter(
        ([a]) => a.method === 'neppy.debug_mode_status'
      ).length;
      expect(await pollWith(moreDirty, TASK)).toBe(base);
      // The poll really ran with the new status; only the key stayed put.
      const after = mockCall.mock.calls.filter(
        ([a]) => a.method === 'neppy.debug_mode_status'
      ).length;
      expect(after).toBeGreaterThan(before);
      expect(screen.getByTestId('debug-dirty')).toHaveTextContent('5');
    });
  });
});
