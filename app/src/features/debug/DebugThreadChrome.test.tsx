import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { DebugThreadChrome } from './DebugThreadChrome';

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

describe('DebugThreadChrome', () => {
  beforeEach(() => {
    mockCall.mockReset();
    statusFails = false;
    mockCall.mockImplementation(async ({ method }) => {
      if (method === 'neppy.debug_mode_status') {
        if (statusFails) throw new Error('no repo');
        return STATUS;
      }
      if (method === 'neppy.debug_mode_task_list') return [TASK];
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
});
