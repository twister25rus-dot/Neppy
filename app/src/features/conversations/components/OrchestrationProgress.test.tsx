import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { fetchSubagentRunsHistory } from '../../../services/api/subagentRunsApi';
import { createTestStore, renderWithProviders } from '../../../test/test-utils';
import type { SubagentRunRow } from '../../../types/subagentRuns';
import type { ThreadMode } from '../../../types/thread';
import { OrchestrationProgress } from './OrchestrationProgress';
import { ThreadModeBar } from './ThreadModeBar';

vi.mock('../../../services/api/subagentRunsApi', () => ({ fetchSubagentRunsHistory: vi.fn() }));
const fetchMock = vi.mocked(fetchSubagentRunsHistory);

const run = (o: Partial<SubagentRunRow>): SubagentRunRow => ({
  runId: 'r1',
  threadId: 't-1',
  threadTitle: 'T',
  threadMode: 'orchestration',
  agentId: 'planner',
  kind: 'subagent',
  status: 'running',
  phase: 'planning',
  summary: null,
  error: null,
  startedAt: null,
  updatedAt: null,
  completedAt: null,
  elapsedMs: 5000,
  model: null,
  toolCount: null,
  costUsd: null,
  ...o,
});

function storeWithMode(mode?: ThreadMode) {
  return createTestStore({
    thread: {
      threads: [
        {
          id: 't-1',
          title: 'T',
          chatId: null,
          isActive: false,
          messageCount: 0,
          lastMessageAt: '',
          createdAt: '',
          labels: [],
          ...(mode ? { mode } : {}),
        },
      ],
      selectedThreadId: 't-1',
      activeThreadIds: {},
      welcomeThreadId: null,
      messagesByThreadId: {},
      messages: [],
      isLoadingThreads: false,
      isLoadingMessages: false,
      messagesError: null,
      createThreadError: null,
      createThreadRequestId: null,
    },
  });
}

describe('OrchestrationProgress', () => {
  beforeEach(() => {
    fetchMock.mockReset();
  });

  it('renders nothing and never fetches in Chat mode (the default)', () => {
    const { container } = renderWithProviders(<OrchestrationProgress threadId="t-1" />, {
      store: storeWithMode(),
    });
    expect(container).toBeEmptyDOMElement();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('shows the thread phase stepper and run list in Orchestration mode', async () => {
    fetchMock.mockResolvedValue({
      count: 2,
      runs: [
        run({}),
        run({ runId: 'r2', agentId: 'researcher', phase: 'completed', status: 'completed' }),
        // A run for another thread must never leak into this strip.
        run({ runId: 'r3', threadId: 't-9', agentId: 'intruder' }),
      ],
      threads: [
        {
          threadId: 't-1',
          threadTitle: 'T',
          threadMode: 'orchestration',
          runCount: 2,
          activeCount: 1,
          failedCount: 0,
          phase: 'planning',
          lastUpdatedAt: null,
        },
      ],
    });
    renderWithProviders(<OrchestrationProgress threadId="t-1" />, {
      store: storeWithMode('orchestration'),
    });

    const panel = await screen.findByTestId('orchestration-progress');
    expect(fetchMock).toHaveBeenCalledWith({ threadId: 't-1', limit: 50 });
    expect(panel.querySelector('[data-step="planning"]')).toHaveAttribute('data-state', 'current');
    expect(panel.querySelector('[data-step="researching"]')).toHaveAttribute('data-state', 'done');
    expect(panel).toHaveTextContent('2 agents, 1 active');

    fireEvent.click(screen.getByText('Show runs'));
    const rows = screen.getAllByTestId('agent-run-row');
    expect(rows.map(r => r.getAttribute('data-run-id'))).toEqual(['r1', 'r2']);
  });

  it('hints instead of showing a panel when the thread has not delegated yet', async () => {
    fetchMock.mockResolvedValue({ count: 0, runs: [], threads: [] });
    renderWithProviders(<OrchestrationProgress threadId="t-1" />, {
      store: storeWithMode('orchestration'),
    });
    expect(await screen.findByTestId('orchestration-progress-empty')).toBeInTheDocument();
    expect(screen.queryByTestId('orchestration-progress')).toBeNull();
  });

  it('shows a retryable error', async () => {
    fetchMock.mockRejectedValueOnce(new Error('x'));
    renderWithProviders(<OrchestrationProgress threadId="t-1" />, {
      store: storeWithMode('orchestration'),
    });
    expect(await screen.findByTestId('orchestration-progress-error')).toBeInTheDocument();
    fetchMock.mockResolvedValue({ count: 0, runs: [], threads: [] });
    fireEvent.click(screen.getByText('Retry'));
    await waitFor(() =>
      expect(screen.getByTestId('orchestration-progress-empty')).toBeInTheDocument()
    );
  });

  it('offers a way into the cross-conversation view', async () => {
    fetchMock.mockResolvedValue({
      count: 1,
      runs: [run({})],
      threads: [
        {
          threadId: 't-1',
          threadTitle: 'T',
          threadMode: 'orchestration',
          runCount: 1,
          activeCount: 1,
          failedCount: 0,
          phase: 'planning',
          lastUpdatedAt: null,
        },
      ],
    });
    const onOpenAllRuns = vi.fn();
    renderWithProviders(<OrchestrationProgress threadId="t-1" onOpenAllRuns={onOpenAllRuns} />, {
      store: storeWithMode('orchestration'),
    });
    fireEvent.click(await screen.findByText('All agent runs'));
    expect(onOpenAllRuns).toHaveBeenCalledTimes(1);
  });
});

describe('ThreadModeBar', () => {
  it('always carries the mode toggle for the open thread', () => {
    renderWithProviders(<ThreadModeBar threadId="t-1" />, { store: storeWithMode() });
    expect(screen.getByTestId('thread-mode-toggle')).toHaveAttribute('data-mode', 'chat');
  });

  it('renders nothing without a thread', () => {
    const { container } = renderWithProviders(<ThreadModeBar threadId={null} />, {
      store: storeWithMode(),
    });
    expect(container).toBeEmptyDOMElement();
  });
});
