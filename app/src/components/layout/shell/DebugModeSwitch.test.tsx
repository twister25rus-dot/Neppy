import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import DebugModeSwitch from './DebugModeSwitch';

const mockCall = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCall(...args),
}));
vi.mock('../../../services/analytics', () => ({ trackEvent: vi.fn() }));

const STATUS = {
  project_root: '/Users/alex/Neppy',
  branch: 'main',
  head: '0123456789abcdef',
  dirty: { modified: [], added: [], deleted: [], untracked: [] },
  task_active: false,
  active_task_id: null,
};

const thread = (over: Record<string, unknown> = {}) => ({
  id: 't-1',
  title: 'T',
  chatId: null,
  isActive: false,
  messageCount: 0,
  lastMessageAt: '',
  createdAt: '',
  labels: [],
  ...over,
});

const threadState = (threads: Record<string, unknown>[], selectedThreadId: string | null) => ({
  threads,
  selectedThreadId,
  activeThreadIds: {},
  welcomeThreadId: null,
  messagesByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
});

let statusFails: boolean;
let setModeFails: boolean;

const methods = () => mockCall.mock.calls.map(c => (c[0] as { method: string }).method);
const setModeCalls = () =>
  mockCall.mock.calls
    .map(c => c[0] as { method: string; params: Record<string, unknown> })
    .filter(c => c.method === 'neppy.threads_set_mode');

function renderSwitch(opts: {
  threads?: Record<string, unknown>[];
  selected?: string | null;
  path?: string;
}) {
  const threads = opts.threads ?? [thread()];
  const store = createTestStore({
    thread: threadState(threads, opts.selected === undefined ? 't-1' : opts.selected),
  });
  renderWithProviders(<DebugModeSwitch />, { store, initialEntries: [opts.path ?? '/chat/t-1'] });
  return store;
}

describe('DebugModeSwitch', () => {
  beforeEach(() => {
    mockCall.mockReset();
    statusFails = false;
    setModeFails = false;
    mockCall.mockImplementation(async ({ method, params }) => {
      if (method === 'neppy.debug_mode_status') {
        if (statusFails) throw new Error('no source repository');
        return STATUS;
      }
      if (method === 'neppy.threads_set_mode') {
        if (setModeFails) throw new Error('refused');
        return {
          data: {
            thread: thread({ id: params.thread_id, mode: params.mode }),
            previousMode: 'chat',
            changed: true,
          },
        };
      }
      throw new Error(`unexpected ${method}`);
    });
  });

  it('is hidden when no thread is open', () => {
    renderSwitch({ selected: null });
    expect(screen.queryByTestId('debug-mode-switch')).toBeNull();
  });

  it('is hidden outside the chat route', () => {
    renderSwitch({ path: '/brain' });
    expect(screen.queryByTestId('debug-mode-switch')).toBeNull();
  });

  it('carries a content-free analytics id and starts off for a normal thread', () => {
    renderSwitch({});
    const sw = screen.getByTestId('debug-mode-switch');
    expect(sw).toHaveAttribute('data-analytics-id', 'topbar-debug-toggle');
    expect(sw).toHaveAttribute('role', 'switch');
    expect(sw).toHaveAttribute('aria-checked', 'false');
  });

  it('reflects a persisted debug mode', () => {
    renderSwitch({ threads: [thread({ mode: 'debug', labels: ['mode:debug'] })] });
    expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'true');
  });

  it('follows the open thread when the selection changes', () => {
    const store = renderSwitch({
      threads: [thread({ id: 'a', mode: 'debug' }), thread({ id: 'b' })],
      selected: 'a',
    });
    expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'true');
    store.dispatch({ type: 'thread/setSelectedThread', payload: 'b' });
    return waitFor(() =>
      expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'false')
    );
  });

  it('checks the source repository, then puts the open thread in debug mode', async () => {
    renderSwitch({});
    fireEvent.click(screen.getByTestId('debug-mode-switch'));

    await waitFor(() =>
      expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'true')
    );
    expect(methods().indexOf('neppy.debug_mode_status')).toBeLessThan(
      methods().indexOf('neppy.threads_set_mode')
    );
    expect(setModeCalls()[0].params).toMatchObject({ thread_id: 't-1', mode: 'debug' });
  });

  it('restores the previous mode when switched off', async () => {
    renderSwitch({ threads: [thread({ id: 'orch', mode: 'orchestration' })], selected: 'orch' });

    fireEvent.click(screen.getByTestId('debug-mode-switch'));
    await waitFor(() =>
      expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'true')
    );

    fireEvent.click(screen.getByTestId('debug-mode-switch'));
    await waitFor(() =>
      expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'false')
    );
    const calls = setModeCalls();
    expect(calls).toHaveLength(2);
    expect(calls[1].params).toMatchObject({ thread_id: 'orch', mode: 'orchestration' });
  });

  it('falls back to chat when the previous mode is unknown', async () => {
    renderSwitch({ threads: [thread({ id: 'old', mode: 'debug' })], selected: 'old' });
    fireEvent.click(screen.getByTestId('debug-mode-switch'));
    await waitFor(() =>
      expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'false')
    );
    expect(setModeCalls()[0].params).toMatchObject({ thread_id: 'old', mode: 'chat' });
  });

  it('does not switch and explains the missing repository when the status call fails', async () => {
    statusFails = true;
    renderSwitch({});
    fireEvent.click(screen.getByTestId('debug-mode-switch'));

    const notice = await screen.findByTestId('debug-mode-switch-notice');
    expect(notice).toHaveTextContent('Neppy source repository');
    expect(notice.querySelector('a')).toHaveAttribute('href', '/settings/debug-mode');
    expect(setModeCalls()).toHaveLength(0);
    expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'false');
  });

  it('reverts and shows an inline error when the core refuses the mode change', async () => {
    setModeFails = true;
    renderSwitch({});
    fireEvent.click(screen.getByTestId('debug-mode-switch'));

    const notice = await screen.findByTestId('debug-mode-switch-notice');
    expect(notice).toHaveTextContent('Could not change the mode.');
    expect(screen.getByTestId('debug-mode-switch')).toHaveAttribute('aria-checked', 'false');
  });
});
