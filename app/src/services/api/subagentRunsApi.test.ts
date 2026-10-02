import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../coreRpcClient';
import { fetchSubagentRunsHistory } from './subagentRunsApi';

vi.mock('../coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const wire = {
  count: 1,
  runs: [
    {
      runId: 'sub-9f2c',
      threadId: 'thread-1',
      threadTitle: 'Ship the release',
      threadMode: 'orchestration',
      agentId: 'researcher',
      kind: 'subagent',
      status: 'running',
      phase: 'researching',
      summary: null,
      error: null,
      startedAt: '2026-10-02T10:00:00+00:00',
      updatedAt: '2026-10-02T10:00:41+00:00',
      completedAt: null,
      elapsedMs: 41200,
      model: 'agentic-v1',
      toolCount: 4,
      costUsd: 0.0123,
    },
  ],
  threads: [
    {
      threadId: 'thread-1',
      threadTitle: 'Ship the release',
      threadMode: 'orchestration',
      runCount: 1,
      activeCount: 1,
      failedCount: 0,
      phase: 'researching',
      lastUpdatedAt: '2026-10-02T10:00:41+00:00',
    },
  ],
};

describe('fetchSubagentRunsHistory', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  it('calls the namespaced method and sends only the filters that are set', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue({ data: wire });
    await fetchSubagentRunsHistory({
      limit: 100,
      mode: 'orchestration',
      threadId: undefined,
      status: '',
      onlyThreaded: true,
    });
    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.subagent_runs_history',
      params: { limit: 100, mode: 'orchestration', onlyThreaded: true },
    });
  });

  it('returns the camelCase projection unchanged (envelope or bare)', async () => {
    vi.mocked(callCoreRpc).mockResolvedValueOnce({ data: wire });
    const wrapped = await fetchSubagentRunsHistory();
    expect(wrapped.runs[0].phase).toBe('researching');
    expect(wrapped.runs[0].elapsedMs).toBe(41200);
    expect(wrapped.threads[0].activeCount).toBe(1);

    vi.mocked(callCoreRpc).mockResolvedValueOnce(wire);
    const bare = await fetchSubagentRunsHistory();
    expect(bare.count).toBe(1);
    expect(bare.runs).toHaveLength(1);
  });

  it('survives a partial result without crashing on .runs', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue({ data: {} });
    expect(await fetchSubagentRunsHistory()).toEqual({ count: 0, runs: [], threads: [] });
  });

  it('propagates core errors to the caller', async () => {
    vi.mocked(callCoreRpc).mockRejectedValue(new Error('unknown method'));
    await expect(fetchSubagentRunsHistory()).rejects.toThrow('unknown method');
  });
});
