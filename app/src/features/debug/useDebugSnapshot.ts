import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  type DebugStatus,
  type DebugTask,
  getDebugStatus,
  listDebugTasks,
} from '../../services/api/debugModeApi';
import { errorText } from './debugFormat';

const log = debug('neppy:debug:snapshot');

/** Fallback refresh cadence while the page is visible. */
export const DEBUG_POLL_MS = 10_000;

export interface DebugSnapshot {
  status: DebugStatus | null;
  /** Set when `debug_mode_status` fails (typically: no source repository). */
  statusError: string | null;
  lastTask: DebugTask | null;
  /** True until the first status call settles. */
  loading: boolean;
  refresh: () => Promise<void>;
}

/**
 * Repo status plus the most recent task, refreshed on mount, whenever
 * `turnActive` falls from true to false (a turn just finished), and every
 * {@link DEBUG_POLL_MS} while the document is visible.
 *
 * A failed *refresh* keeps the last good data: only the first load can put the
 * hook into the error state, and a later success clears it.
 */
export function useDebugSnapshot(turnActive: boolean): DebugSnapshot {
  const [status, setStatus] = useState<DebugStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [lastTask, setLastTask] = useState<DebugTask | null>(null);
  const [loading, setLoading] = useState(true);
  const aliveRef = useRef(true);
  const seqRef = useRef(0);
  const hasStatusRef = useRef(false);

  const refresh = useCallback(async () => {
    const seq = ++seqRef.current;
    try {
      const [s, tasks] = await Promise.all([
        getDebugStatus(),
        listDebugTasks(1).catch((error: unknown) => {
          log('task_list failed: %s', errorText(error) ? 'error' : 'unknown');
          return null;
        }),
      ]);
      if (!aliveRef.current || seq !== seqRef.current) return;
      hasStatusRef.current = true;
      setStatus(s);
      setStatusError(null);
      if (tasks) setLastTask(tasks[0] ?? null);
    } catch (error) {
      if (!aliveRef.current || seq !== seqRef.current) return;
      log('status failed');
      // Keep the last good snapshot on a transient refresh failure.
      if (!hasStatusRef.current) setStatusError(errorText(error) || 'unavailable');
    } finally {
      if (aliveRef.current && seq === seqRef.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    aliveRef.current = true;
    void refresh();
    return () => {
      aliveRef.current = false;
    };
  }, [refresh]);

  const wasActiveRef = useRef(turnActive);
  useEffect(() => {
    if (wasActiveRef.current && !turnActive) {
      log('turn finished: refreshing');
      void refresh();
    }
    wasActiveRef.current = turnActive;
  }, [turnActive, refresh]);

  useEffect(() => {
    const id = window.setInterval(() => {
      if (document.visibilityState === 'hidden') return;
      void refresh();
    }, DEBUG_POLL_MS);
    return () => window.clearInterval(id);
  }, [refresh]);

  return { status, statusError, lastTask, loading, refresh };
}
