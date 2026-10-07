import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { fetchSubagentRunsHistory } from '../../services/api/subagentRunsApi';
import type { SubagentRunsHistory, SubagentRunsHistoryParams } from '../../types/subagentRuns';
import { isLiveRun } from './agentRunPhases';

const log = debug('neppy:orchestration:runs');

export interface UseSubagentRunsOptions {
  /** Poll interval while any run is live. `0` disables polling. */
  pollMs?: number;
  /** Change to force a refetch (e.g. a turn just settled). */
  refreshKey?: unknown;
  /** Skip fetching entirely (e.g. the surface is not visible). */
  enabled?: boolean;
}

export interface UseSubagentRunsResult {
  data: SubagentRunsHistory | null;
  /** True only for the first load; refreshes keep showing the previous data. */
  loading: boolean;
  /** Localizable-agnostic flag; the raw message stays in debug logs only. */
  error: boolean;
  refresh: () => void;
}

/**
 * Read the run-ledger projection with loading / error states and light
 * polling while something is still running. Stale responses (a slower request
 * overtaken by a newer one, or one landing after unmount) are dropped.
 */
export function useSubagentRuns(
  params: SubagentRunsHistoryParams,
  { pollMs = 4000, refreshKey, enabled = true }: UseSubagentRunsOptions = {}
): UseSubagentRunsResult {
  const [data, setData] = useState<SubagentRunsHistory | null>(null);
  const [error, setError] = useState(false);
  const [tick, setTick] = useState(0);
  const seqRef = useRef(0);
  const paramsKey = JSON.stringify(params);

  const refresh = useCallback(() => setTick(n => n + 1), []);

  useEffect(() => {
    if (!enabled) return;
    const seq = ++seqRef.current;
    const parsed = JSON.parse(paramsKey) as SubagentRunsHistoryParams;
    fetchSubagentRunsHistory(parsed)
      .then(next => {
        if (seq !== seqRef.current) return;
        setData(next);
        setError(false);
      })
      .catch((err: unknown) => {
        log('fetch failed: %o', err);
        if (seq !== seqRef.current) return;
        setError(true);
      });
    return () => {
      // Invalidate this request if the deps change or the hook unmounts.
      if (seqRef.current === seq) seqRef.current += 1;
    };
  }, [paramsKey, refreshKey, tick, enabled]);

  const live = data?.runs.some(isLiveRun) ?? false;
  useEffect(() => {
    if (!enabled || !pollMs || !live) return;
    const id = window.setInterval(() => setTick(n => n + 1), pollMs);
    return () => window.clearInterval(id);
  }, [enabled, pollMs, live]);

  return { data, loading: enabled && data === null && !error, error, refresh };
}
