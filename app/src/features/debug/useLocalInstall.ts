import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  applyLocalInstall,
  getLocalInstallResult,
  getLocalInstallStatus,
  type LocalInstallResult,
  type LocalInstallStatus,
  quitApp,
  startLocalInstallBuild,
} from '../../services/api/debugModeApi';
import { errorText } from './debugFormat';

const log = debug('neppy:debug:local-install');

/** How often the build status is re-read while a build or install is running. */
export const LOCAL_INSTALL_POLL_MS = 3_000;

export type LocalInstallErrorKind = 'start' | 'install' | 'quit';

export interface LocalInstallError {
  kind: LocalInstallErrorKind;
  message: string;
}

/** A status payload from the core, or `null` when it is not one. */
function asStatus(value: unknown): LocalInstallStatus | null {
  if (!value || typeof value !== 'object') return null;
  return typeof (value as { phase?: unknown }).phase === 'string'
    ? (value as LocalInstallStatus)
    : null;
}

function asResult(value: unknown): LocalInstallResult | null {
  if (!value || typeof value !== 'object') return null;
  return typeof (value as { status?: unknown }).status === 'string'
    ? (value as LocalInstallResult)
    : null;
}

/**
 * Drives "Build & install locally": the build status (polled every
 * {@link LOCAL_INSTALL_POLL_MS} only while a build or install is in flight),
 * the start / apply actions, and the installer's last verdict for the one-time
 * "restored" notice. Only phases and message text are logged, never output.
 */
export function useLocalInstall() {
  const [status, setStatus] = useState<LocalInstallStatus | null>(null);
  const [result, setResult] = useState<LocalInstallResult | null>(null);
  const [error, setError] = useState<LocalInstallError | null>(null);
  const [busy, setBusy] = useState(false);
  const aliveRef = useRef(true);

  const refresh = useCallback(async () => {
    try {
      const next = asStatus(await getLocalInstallStatus());
      if (aliveRef.current) setStatus(next);
    } catch (e) {
      log('status failed: %s', errorText(e) ? 'error' : 'unknown');
    }
  }, []);

  useEffect(() => {
    aliveRef.current = true;
    void refresh();
    getLocalInstallResult()
      .then(r => {
        if (aliveRef.current) setResult(asResult(r));
      })
      .catch(() => log('result unavailable'));
    return () => {
      aliveRef.current = false;
    };
  }, [refresh]);

  const phase = status?.phase;
  useEffect(() => {
    if (phase !== 'building' && phase !== 'installing') return undefined;
    const id = window.setInterval(() => void refresh(), LOCAL_INSTALL_POLL_MS);
    return () => window.clearInterval(id);
  }, [phase, refresh]);

  const build = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const next = asStatus(await startLocalInstallBuild());
      log('build started phase=%s', next?.phase);
      if (aliveRef.current) setStatus(next);
    } catch (e) {
      if (aliveRef.current) {
        setError({ kind: 'start', message: errorText(e) });
      }
    } finally {
      if (aliveRef.current) setBusy(false);
    }
  }, []);

  /** Hands the bundle to the helper, then quits so it can swap the app. */
  const install = useCallback(async () => {
    setBusy(true);
    setError(null);
    try {
      const next = asStatus(await applyLocalInstall());
      log('install handed over phase=%s', next?.phase);
      if (aliveRef.current) setStatus(next);
    } catch (e) {
      if (aliveRef.current) {
        setError({ kind: 'install', message: errorText(e) });
        setBusy(false);
      }
      return;
    }
    try {
      await quitApp();
    } catch (e) {
      log('quit failed');
      if (aliveRef.current) setError({ kind: 'quit', message: errorText(e) });
    } finally {
      if (aliveRef.current) setBusy(false);
    }
  }, []);

  const acknowledgeResult = useCallback(async () => {
    setResult(null);
    try {
      await getLocalInstallResult(true);
    } catch {
      log('acknowledge failed');
    }
  }, []);

  return { status, result, error, busy, build, install, acknowledgeResult };
}
