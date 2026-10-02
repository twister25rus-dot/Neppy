import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { fetchSubagentRunsHistory } from '../../../services/api/subagentRunsApi';
import { renderWithProviders } from '../../../test/test-utils';
import type { SubagentRunRow, SubagentRunsHistory } from '../../../types/subagentRuns';
import AgentRunsPanel from '../AgentRunsPanel';

vi.mock('../../../services/api/subagentRunsApi', () => ({ fetchSubagentRunsHistory: vi.fn() }));

const fetchMock = vi.mocked(fetchSubagentRunsHistory);

function row(overrides: Partial<SubagentRunRow>): SubagentRunRow {
  return {
    runId: 'r1',
    threadId: 'thread-1',
    threadTitle: 'Ship the release',
    threadMode: 'orchestration',
    agentId: 'researcher',
    kind: 'subagent',
    status: 'completed',
    phase: 'completed',
    summary: null,
    error: null,
    startedAt: null,
    updatedAt: null,
    completedAt: null,
    elapsedMs: 41_200,
    model: 'agentic-v1',
    toolCount: 4,
    costUsd: 0.0123,
    ...overrides,
  };
}

const history: SubagentRunsHistory = {
  count: 3,
  runs: [
    row({ runId: 'a', agentId: 'researcher', phase: 'researching', status: 'running' }),
    row({ runId: 'b', agentId: 'code_executor', summary: 'Patched 3 files, build green' }),
    row({
      runId: 'c',
      threadId: 'thread-2',
      threadTitle: 'Other task',
      agentId: 'critic',
      phase: 'failed',
      status: 'failed',
      error: 'tests red',
    }),
  ],
  threads: [
    {
      threadId: 'thread-1',
      threadTitle: 'Ship the release',
      threadMode: 'orchestration',
      runCount: 2,
      activeCount: 1,
      failedCount: 0,
      phase: 'researching',
      lastUpdatedAt: null,
    },
    {
      threadId: 'thread-2',
      threadTitle: 'Other task',
      threadMode: 'orchestration',
      runCount: 1,
      activeCount: 0,
      failedCount: 1,
      phase: 'failed',
      lastUpdatedAt: null,
    },
  ],
};

describe('AgentRunsPanel', () => {
  beforeEach(() => {
    fetchMock.mockReset();
  });

  it('shows a loading state, then runs grouped under their conversation', async () => {
    fetchMock.mockResolvedValue(history);
    renderWithProviders(<AgentRunsPanel />);
    expect(screen.getByTestId('agent-runs-loading')).toBeInTheDocument();

    const threads = await screen.findAllByTestId('agent-runs-thread');
    expect(threads).toHaveLength(2);
    expect(threads[0]).toHaveTextContent('Ship the release');
    expect(threads[0]).toHaveTextContent('researcher');
    expect(threads[0]).toHaveTextContent('code_executor');
    expect(threads[1]).toHaveTextContent('Other task');
    // Per-thread simplified progress.
    expect(threads[0].querySelector('[data-testid="agent-run-stepper"]')).toHaveAttribute(
      'data-phase',
      'researching'
    );
    expect(threads[1].querySelector('[data-testid="agent-run-stepper"]')).toHaveAttribute(
      'data-phase',
      'failed'
    );
  });

  it('asks the core for orchestration threads by default and widens to all', async () => {
    fetchMock.mockResolvedValue(history);
    renderWithProviders(<AgentRunsPanel />);
    await screen.findAllByTestId('agent-runs-thread');
    expect(fetchMock).toHaveBeenLastCalledWith({
      limit: 100,
      onlyThreaded: true,
      mode: 'orchestration',
    });

    fireEvent.click(screen.getByTestId('agent-runs-scope-all'));
    await waitFor(() =>
      expect(fetchMock).toHaveBeenLastCalledWith({ limit: 100, onlyThreaded: true })
    );
  });

  it('filters by status client-side', async () => {
    fetchMock.mockResolvedValue(history);
    renderWithProviders(<AgentRunsPanel />);
    await screen.findAllByTestId('agent-runs-thread');

    fireEvent.click(screen.getByTestId('agent-runs-status-failed'));
    await waitFor(() => expect(screen.getAllByTestId('agent-runs-thread')).toHaveLength(1));
    expect(screen.getByTestId('agent-runs-thread')).toHaveTextContent('Other task');

    fireEvent.click(screen.getByTestId('agent-runs-status-running'));
    await waitFor(() => expect(screen.getAllByTestId('agent-run-row')).toHaveLength(1));
    expect(screen.getByTestId('agent-run-row')).toHaveAttribute('data-run-id', 'a');
  });

  it('drills into a run to see its summary / error', async () => {
    fetchMock.mockResolvedValue(history);
    renderWithProviders(<AgentRunsPanel />);
    const rows = await screen.findAllByTestId('agent-run-row');
    const done = rows.find(r => r.getAttribute('data-run-id') === 'b')!;
    expect(done.querySelector('[data-testid="agent-run-details"]')).toBeNull();
    fireEvent.click(done.querySelector('button')!);
    expect(done.querySelector('[data-testid="agent-run-details"]')).toHaveTextContent(
      'Patched 3 files, build green'
    );
  });

  it('shows an empty state', async () => {
    fetchMock.mockResolvedValue({ count: 0, runs: [], threads: [] });
    renderWithProviders(<AgentRunsPanel />);
    expect(await screen.findByTestId('agent-runs-empty')).toHaveTextContent('No agent runs yet');
  });

  it('shows a filtered-empty message when filters exclude everything', async () => {
    fetchMock.mockResolvedValue({ ...history, runs: [history.runs[1]] });
    renderWithProviders(<AgentRunsPanel />);
    await screen.findAllByTestId('agent-runs-thread');
    fireEvent.click(screen.getByTestId('agent-runs-status-running'));
    expect(await screen.findByTestId('agent-runs-empty')).toHaveTextContent(
      'No runs match these filters.'
    );
  });

  it('shows an error state with retry, and recovers', async () => {
    fetchMock.mockRejectedValueOnce(new Error('boom'));
    renderWithProviders(<AgentRunsPanel />);
    expect(await screen.findByTestId('agent-runs-error')).toBeInTheDocument();
    // Raw error text is never shown to the user.
    expect(screen.queryByText(/boom/)).toBeNull();

    fetchMock.mockResolvedValue(history);
    fireEvent.click(screen.getByText('Retry'));
    expect(await screen.findAllByTestId('agent-runs-thread')).toHaveLength(2);
  });
});
