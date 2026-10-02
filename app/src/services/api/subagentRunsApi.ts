import debug from 'debug';

import type { SubagentRunsHistory, SubagentRunsHistoryParams } from '../../types/subagentRuns';
import { callCoreRpc } from '../coreRpcClient';

const log = debug('subagentRunsApi');

interface Envelope<T> {
  data?: T;
}

/**
 * Fetch the cross-thread agent-run history (`openhuman.subagent_runs_history`).
 * Only filters the caller set are sent; the core applies its own defaults
 * (limit 50, max 200; `onlyThreaded` true). Tolerates a missing/partial result
 * so a core that predates the method surfaces as an error to the caller, never
 * as a crash on `.runs`.
 */
export async function fetchSubagentRunsHistory(
  params: SubagentRunsHistoryParams = {}
): Promise<SubagentRunsHistory> {
  const clean: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== '') clean[key] = value;
  }
  log('fetch params=%o', clean);
  const response = await callCoreRpc<Envelope<SubagentRunsHistory> | SubagentRunsHistory>({
    method: 'openhuman.subagent_runs_history',
    params: clean,
  });
  const data: Partial<SubagentRunsHistory> | undefined =
    response && typeof response === 'object' && 'data' in response
      ? (response as Envelope<SubagentRunsHistory>).data
      : (response as SubagentRunsHistory);
  const runs = Array.isArray(data?.runs) ? data.runs : [];
  const threads = Array.isArray(data?.threads) ? data.threads : [];
  log('fetched runs=%d threads=%d', runs.length, threads.length);
  return { count: data?.count ?? runs.length, runs, threads };
}
