import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  getReleasePreflight,
  getReleaseStatus,
  type ReleasePreflight,
  type ReleaseRecord,
  startRelease,
} from '../../services/api/debugModeApi';
import { errorText } from './debugFormat';

const log = debug('neppy:debug:release');

/** How often the release status is re-read while a release is running. */
export const RELEASE_POLL_MS = 3_000;

/** Consecutive failed status reads (while running) before "lost contact" shows. */
export const RELEASE_LOST_CONTACT_AFTER = 3;

/** localStorage key remembering which finished run the user already dismissed. */
export const RELEASE_DISMISSED_KEY = 'neppy:debug:release:dismissed';

export type ReleaseErrorKind = 'load' | 'start';

export interface ReleaseError {
  kind: ReleaseErrorKind;
  message: string;
}

export type ReleaseVersionProblem = 'format' | 'notGreater';

const VERSION_RE = /^\d+\.\d+\.\d+$/;

function parts(version: string): number[] {
  return version.split('.').map(Number);
}

/**
 * Client-side check of a version the user typed: `X.Y.Z`, and strictly greater
 * than `current` compared part by part as numbers (so 0.68.10 > 0.68.9). The
 * core validates again; this only avoids a round trip for an obvious typo.
 */
export function validateReleaseVersion(
  version: string,
  current: string
): ReleaseVersionProblem | null {
  const v = version.trim();
  if (!VERSION_RE.test(v)) return 'format';
  if (!VERSION_RE.test(current)) return null;
  const next = parts(v);
  const base = parts(current);
  for (let i = 0; i < 3; i += 1) {
    if (next[i] > base[i]) return null;
    if (next[i] < base[i]) return 'notGreater';
  }
  return 'notGreater';
}

function asPreflight(value: unknown): ReleasePreflight | null {
  if (!value || typeof value !== 'object') return null;
  const v = value as { blockers?: unknown; current_version?: unknown };
  return Array.isArray(v.blockers) && typeof v.current_version === 'string'
    ? (value as ReleasePreflight)
    : null;
}

function asRecord(value: unknown): ReleaseRecord | null {
  if (!value || typeof value !== 'object') return null;
  return typeof (value as { phase?: unknown }).phase === 'string' ? (value as ReleaseRecord) : null;
}

/** Identity of one finished run, stable across remounts and restarts. */
function runKey(record: ReleaseRecord | null): string | null {
  if (!record || (record.phase !== 'succeeded' && record.phase !== 'failed')) return null;
  return `${record.version ?? ''}|${record.started_at ?? ''}|${record.finished_at ?? ''}`;
}

function readDismissed(): string | null {
  try {
    return window.localStorage.getItem(RELEASE_DISMISSED_KEY);
  } catch {
    return null;
  }
}

function writeDismissed(key: string): void {
  try {
    window.localStorage.setItem(RELEASE_DISMISSED_KEY, key);
  } catch {
    log('could not persist dismissed run');
  }
}

/**
 * Drives "Publish release": the preflight (re-read on mount, whenever
 * `refreshKey` changes, after a start, when a run finishes, and on demand), the
 * release record (polled every {@link RELEASE_POLL_MS} only while
 * `phase === 'running'`), and the start action. Failures become typed state,
 * never exceptions into React. Only phases and counts are logged, never the
 * version output or log contents.
 *
 * A finished run the user dismissed stays dismissed across remounts (remembered
 * by its identity in localStorage, best effort); a new run always shows.
 */
export function useRelease(refreshKey?: string | number) {
  const [preflight, setPreflight] = useState<ReleasePreflight | null>(null);
  const [record, setRecord] = useState<ReleaseRecord | null>(null);
  const [error, setError] = useState<ReleaseError | null>(null);
  const [busy, setBusy] = useState(false);
  const [checking, setChecking] = useState(false);
  const [dismissedKey, setDismissedKey] = useState<string | null>(readDismissed);
  const [pollFailures, setPollFailures] = useState(0);
  const aliveRef = useRef(true);
  const prevPhaseRef = useRef<string | undefined>(undefined);
  const lastRefreshKeyRef = useRef(refreshKey);

  const loadPreflight = useCallback(async () => {
    try {
      const next = asPreflight(await getReleasePreflight());
      log('preflight loaded blockers=%d', next?.blockers.length ?? -1);
      if (!aliveRef.current) return;
      setPreflight(next);
      setError(prev => (prev?.kind === 'load' ? null : prev));
    } catch (e) {
      log('preflight failed');
      if (aliveRef.current) setError({ kind: 'load', message: errorText(e) });
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      const next = asRecord(await getReleaseStatus());
      if (!aliveRef.current) return;
      setRecord(next);
      setPollFailures(0);
    } catch {
      log('status failed');
      if (aliveRef.current) setPollFailures(n => n + 1);
    }
  }, []);

  useEffect(() => {
    aliveRef.current = true;
    void loadPreflight();
    void refreshStatus();
    return () => {
      aliveRef.current = false;
    };
  }, [loadPreflight, refreshStatus]);

  // The debug snapshot moved (a commit, a new HEAD, a task action): readiness
  // may have changed, so re-run the preflight. The mount effect covers the first.
  useEffect(() => {
    if (lastRefreshKeyRef.current === refreshKey) return;
    lastRefreshKeyRef.current = refreshKey;
    log('refresh key changed: re-checking');
    void loadPreflight();
  }, [refreshKey, loadPreflight]);

  const phase = record?.phase;

  useEffect(() => {
    if (phase !== 'running') return undefined;
    const id = window.setInterval(() => void refreshStatus(), RELEASE_POLL_MS);
    return () => window.clearInterval(id);
  }, [phase, refreshStatus]);

  // A run that just finished changes what is releasable (new tag, new HEAD).
  useEffect(() => {
    const prev = prevPhaseRef.current;
    prevPhaseRef.current = phase;
    if (prev === 'running' && (phase === 'succeeded' || phase === 'failed')) {
      log('release finished phase=%s', phase);
      void loadPreflight();
    }
  }, [phase, loadPreflight]);

  const start = useCallback(
    async (version: string) => {
      setBusy(true);
      setError(null);
      try {
        const next = asRecord(await startRelease(version));
        log('release started phase=%s', next?.phase);
        if (aliveRef.current) {
          setPollFailures(0);
          setRecord(next);
        }
        await loadPreflight();
      } catch (e) {
        log('release start failed');
        if (aliveRef.current) setError({ kind: 'start', message: errorText(e) });
      } finally {
        if (aliveRef.current) setBusy(false);
      }
    },
    [loadPreflight]
  );

  /** Re-runs the preflight on demand ("Check again"). */
  const recheck = useCallback(async () => {
    setChecking(true);
    try {
      await loadPreflight();
    } finally {
      if (aliveRef.current) setChecking(false);
    }
  }, [loadPreflight]);

  /** Leaves a finished run's result view and re-checks readiness ("Try again"). */
  const reset = useCallback(async () => {
    const key = runKey(record);
    if (key) {
      setDismissedKey(key);
      writeDismissed(key);
    }
    setError(null);
    await loadPreflight();
  }, [record, loadPreflight]);

  const key = runKey(record);
  const dismissed = key !== null && key === dismissedKey;
  const lostContact = phase === 'running' && pollFailures >= RELEASE_LOST_CONTACT_AFTER;

  return {
    preflight,
    record,
    error,
    busy,
    checking,
    dismissed,
    lostContact,
    start,
    recheck,
    reset,
  };
}
