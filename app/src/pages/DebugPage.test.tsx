import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { Route, Routes, useLocation } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { createTestStore, renderWithProviders } from '../test/test-utils';
import DebugPage from './DebugPage';

const mockCall = vi.fn();
vi.mock('../services/coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCall(...args),
}));
vi.mock('../services/analytics', () => ({ trackEvent: vi.fn() }));

// The real conversation view is exercised in its own suite; here we only need
// to know it mounted, with the Debug scope.
vi.mock('../features/conversations/Conversations', () => ({
  ConversationsPage: ({ scope }: { scope?: string }) => (
    <div data-testid="conversations" data-scope={scope} />
  ),
}));

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
  request: 'Fix the sidebar overflow on narrow windows',
  created_at: '',
  updated_at: '',
  status: 'pass',
  files_changed: ['a.ts', 'b.ts'],
  validation: [],
  summary: null,
  checkpoint_id: 'cp-1',
  branch: 'main',
  commit: null,
};

const thread = (over: Record<string, unknown> = {}) => ({
  id: 'dbg-1',
  title: 'Debug',
  chatId: null,
  isActive: false,
  messageCount: 0,
  lastMessageAt: '2026-10-01T00:00:00Z',
  createdAt: '2026-10-01T00:00:00Z',
  labels: ['mode:debug'],
  mode: 'debug',
  ...over,
});

interface World {
  status: unknown | Error;
  tasks: unknown[];
  threads: Record<string, unknown>[];
}
let world: World;

const methods = () => mockCall.mock.calls.map(c => (c[0] as { method: string }).method);
const callsOf = (m: string) =>
  mockCall.mock.calls.filter(c => (c[0] as { method: string }).method === m);

function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="loc">{loc.pathname}</div>;
}

function renderPage(path: string) {
  const store = createTestStore();
  renderWithProviders(
    <>
      <Routes>
        <Route path="/debug/:threadId?" element={<DebugPage />} />
      </Routes>
      <LocationProbe />
    </>,
    { store, initialEntries: [path] }
  );
  return store;
}

beforeEach(() => {
  mockCall.mockReset();
  world = { status: STATUS, tasks: [TASK], threads: [thread()] };
  mockCall.mockImplementation(async ({ method, params }: { method: string; params?: unknown }) => {
    switch (method) {
      case 'neppy.debug_mode_status':
        if (world.status instanceof Error) throw world.status;
        return world.status;
      case 'neppy.debug_mode_task_list':
        return world.tasks;
      case 'neppy.threads_list':
        return { data: { threads: world.threads, count: world.threads.length } };
      case 'neppy.threads_create_new': {
        const t = thread({ id: 'dbg-new', mode: 'chat', labels: [] });
        world.threads = [...world.threads, t];
        return { data: t };
      }
      case 'neppy.threads_set_mode': {
        const id = (params as { thread_id: string }).thread_id;
        world.threads = world.threads.map(t =>
          t.id === id ? { ...t, mode: 'debug', labels: ['mode:debug'] } : t
        );
        return {
          data: {
            thread: world.threads.find(t => t.id === id),
            previousMode: 'chat',
            changed: true,
          },
        };
      }
      case 'neppy.debug_mode_commit':
        return { task_id: 'task-1', commit: 'deadbeefcafe', branch: 'main', files: ['a.ts'] };
      case 'neppy.debug_mode_rollback':
        return { checkpoint_id: 'cp-1', pre_rollback_checkpoint_id: 'cp-2' };
      default:
        throw new Error(`unexpected rpc ${method}`);
    }
  });
});

describe('DebugPage', () => {
  it('shows a clear message and creates no thread when status fails', async () => {
    world.status = new Error('project root is not a git work tree');
    renderPage('/debug');

    const panel = await screen.findByTestId('debug-unavailable');
    expect(panel).toHaveTextContent('NEPPY_DEBUG_PROJECT_ROOT');
    expect(panel).toHaveTextContent('project root is not a git work tree');
    expect(screen.queryByTestId('conversations')).not.toBeInTheDocument();
    expect(methods()).not.toContain('neppy.threads_create_new');
    expect(methods()).not.toContain('neppy.threads_list');
  });

  it('renders the banner with repo, branch, short HEAD and dirty count', async () => {
    renderPage('/debug/dbg-1');

    const banner = await screen.findByTestId('debug-banner');
    expect(banner).toHaveTextContent('DEBUG MODE • Development access enabled');
    expect(banner).toHaveTextContent('Neppy');
    expect(banner).toHaveTextContent('feat/debug-mode');
    expect(banner).toHaveTextContent('0123456');
    expect(banner).not.toHaveTextContent('0123456789abcdef');
    expect(within(banner).getByTestId('debug-dirty')).toHaveTextContent('4');
  });

  it('reuses the conversation view in the Debug scope for an existing debug thread', async () => {
    renderPage('/debug/dbg-1');
    const convo = await screen.findByTestId('conversations');
    expect(convo).toHaveAttribute('data-scope', 'debug');
    expect(methods()).not.toContain('neppy.threads_create_new');
  });

  it('opens the most recent debug thread when no id is given', async () => {
    world.threads = [
      thread({ id: 'old', lastMessageAt: '2026-09-01T00:00:00Z' }),
      thread({ id: 'new', lastMessageAt: '2026-10-02T00:00:00Z' }),
      thread({ id: 'chat', mode: 'chat', labels: [], lastMessageAt: '2026-10-03T00:00:00Z' }),
    ];
    renderPage('/debug');

    await waitFor(() => expect(screen.getByTestId('loc')).toHaveTextContent('/debug/new'));
    expect(methods()).not.toContain('neppy.threads_create_new');
  });

  it('creates a debug-mode thread when none exists', async () => {
    world.threads = [thread({ id: 'chat', mode: 'chat', labels: [] })];
    renderPage('/debug');

    await waitFor(() => expect(screen.getByTestId('loc')).toHaveTextContent('/debug/dbg-new'));
    expect(callsOf('neppy.threads_set_mode')[0][0]).toMatchObject({
      params: { thread_id: 'dbg-new', mode: 'debug' },
    });
  });

  it('redirects away from a non-debug thread id instead of showing it', async () => {
    world.threads = [thread(), thread({ id: 'chat', mode: 'chat', labels: [] })];
    renderPage('/debug/chat');
    await waitFor(() => expect(screen.getByTestId('loc')).toHaveTextContent('/debug/dbg-1'));
  });

  it('New debug task creates another debug thread and navigates to it', async () => {
    renderPage('/debug/dbg-1');
    fireEvent.click(await screen.findByTestId('debug-new-task'));

    await waitFor(() => expect(screen.getByTestId('loc')).toHaveTextContent('/debug/dbg-new'));
    expect(callsOf('neppy.threads_set_mode')).toHaveLength(1);
  });

  describe('last task card', () => {
    it('shows status, file count and the three actions', async () => {
      renderPage('/debug/dbg-1');
      const card = await screen.findByTestId('debug-last-task');
      expect(within(card).getByTestId('debug-task-status')).toHaveTextContent('Passed');
      expect(within(card).getByTestId('debug-task-files')).toHaveTextContent('Files changed: 2');
      expect(callsOf('neppy.debug_mode_task_list')[0][0]).toMatchObject({ params: { limit: 1 } });
      expect(screen.getByTestId('debug-commit')).toBeEnabled();
      expect(screen.getByTestId('debug-keep')).toBeEnabled();
      expect(screen.getByTestId('debug-rollback')).toBeEnabled();
    });

    it('Commit opens a dialog prefilled from the request and sends confirm:true', async () => {
      renderPage('/debug/dbg-1');
      fireEvent.click(await screen.findByTestId('debug-commit'));

      const message = (await screen.findByTestId('debug-commit-message')) as HTMLTextAreaElement;
      expect(message.value).toBe('feat(debug): Fix the sidebar overflow on narrow windows');
      expect(callsOf('neppy.debug_mode_commit')).toHaveLength(0);

      fireEvent.change(message, { target: { value: 'fix: sidebar overflow' } });
      fireEvent.click(screen.getByTestId('debug-commit-confirm'));

      await waitFor(() => expect(callsOf('neppy.debug_mode_commit')).toHaveLength(1));
      expect(callsOf('neppy.debug_mode_commit')[0][0]).toMatchObject({
        params: { task_id: 'task-1', message: 'fix: sidebar overflow', confirm: true },
      });
      expect(await screen.findByTestId('debug-notice')).toHaveTextContent('deadbee');
    });

    it('shows a commit refusal inline and keeps the dialog open', async () => {
      const base = mockCall.getMockImplementation()!;
      mockCall.mockImplementation(async (arg: { method: string }) => {
        if (arg.method === 'neppy.debug_mode_commit') {
          throw new Error('index already has staged changes outside this task: b.txt');
        }
        return base(arg);
      });
      renderPage('/debug/dbg-1');
      fireEvent.click(await screen.findByTestId('debug-commit'));
      fireEvent.click(await screen.findByTestId('debug-commit-confirm'));

      expect(await screen.findByTestId('debug-commit-error')).toHaveTextContent('b.txt');
      expect(screen.getByTestId('debug-commit-message')).toBeInTheDocument();
    });

    it('Rollback explains it is reversible and sends the checkpoint with confirm:true', async () => {
      renderPage('/debug/dbg-1');
      fireEvent.click(await screen.findByTestId('debug-rollback'));

      expect(await screen.findByText(/pre-rollback checkpoint is saved first/)).toBeInTheDocument();
      expect(callsOf('neppy.debug_mode_rollback')).toHaveLength(0);

      fireEvent.click(screen.getByTestId('confirm-dialog-confirm'));
      await waitFor(() => expect(callsOf('neppy.debug_mode_rollback')).toHaveLength(1));
      expect(callsOf('neppy.debug_mode_rollback')[0][0]).toMatchObject({
        params: { checkpoint_id: 'cp-1', confirm: true },
      });
    });

    it('Keep just dismisses the card without calling the core', async () => {
      renderPage('/debug/dbg-1');
      fireEvent.click(await screen.findByTestId('debug-keep'));
      expect(screen.queryByTestId('debug-last-task')).not.toBeInTheDocument();
      expect(callsOf('neppy.debug_mode_commit')).toHaveLength(0);
      expect(callsOf('neppy.debug_mode_rollback')).toHaveLength(0);
    });

    it('disables Commit and Rollback while the task is still running', async () => {
      world.tasks = [{ ...TASK, status: 'editing' }];
      renderPage('/debug/dbg-1');
      await screen.findByTestId('debug-last-task');
      expect(screen.getByTestId('debug-commit')).toBeDisabled();
      expect(screen.getByTestId('debug-rollback')).toBeDisabled();
    });

    it('renders no card when there are no tasks yet', async () => {
      world.tasks = [];
      renderPage('/debug/dbg-1');
      await screen.findByTestId('debug-banner');
      expect(screen.queryByTestId('debug-last-task')).not.toBeInTheDocument();
    });
  });
});
