import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  type DebugModeSettings,
  getDebugSettings,
  updateDebugSettings,
} from '../../../services/api/debugModeApi';

const log = debug('settings:debugMode');

export const MAX_REPAIR_MIN = 1;
export const MAX_REPAIR_MAX = 20;

interface DebugModeSettingsState {
  settings: DebugModeSettings | null;
  loading: boolean;
  loadError: string | null;
  /** Inline error from the last failed save (cleared on the next attempt). */
  saveError: string | null;
  saving: boolean;
  savedAt: number | null;
  reload: () => void;
  /** Optimistic minimal-patch save; reverts only the patched keys on failure. */
  update: (patch: Partial<DebugModeSettings>) => Promise<boolean>;
}

const messageOf = (e: unknown, fallback: string): string =>
  e instanceof Error && e.message ? e.message : fallback;

/**
 * Loads and edits Debug Mode settings. Every edit sends only the fields that
 * changed; the UI updates optimistically and the patched keys snap back to
 * their previous values if the core rejects the patch. Only the most recent
 * save may write the server response back (last write wins).
 */
export function useDebugModeSettings(
  loadFallback: string,
  saveFallback: string
): DebugModeSettingsState {
  const [settings, setSettings] = useState<DebugModeSettings | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [reloadTick, setReloadTick] = useState(0);
  const seqRef = useRef(0);
  const settingsRef = useRef<DebugModeSettings | null>(null);
  settingsRef.current = settings;
  const fallbacksRef = useRef({ loadFallback, saveFallback });
  fallbacksRef.current = { loadFallback, saveFallback };

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setLoadError(null);
    log('[debug-mode-settings] load start');
    getDebugSettings()
      .then(s => {
        if (cancelled) return;
        log('[debug-mode-settings] load ok');
        setSettings(s);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        log('[debug-mode-settings] load failed');
        setLoadError(messageOf(e, fallbacksRef.current.loadFallback));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [reloadTick]);

  const reload = useCallback(() => setReloadTick(n => n + 1), []);

  const update = useCallback(async (patch: Partial<DebugModeSettings>): Promise<boolean> => {
    const prev = settingsRef.current;
    if (!prev) return false;
    const keys = Object.keys(patch) as (keyof DebugModeSettings)[];
    const seq = ++seqRef.current;
    log('[debug-mode-settings] update seq=%d keys=%s', seq, keys.join(','));
    setSaveError(null);
    setSavedAt(null);
    setSaving(true);
    setSettings(s => (s ? { ...s, ...patch } : s));
    try {
      const next = await updateDebugSettings(patch);
      if (seqRef.current === seq) {
        setSettings(next);
        setSavedAt(Date.now());
      }
      log('[debug-mode-settings] update ok seq=%d', seq);
      return true;
    } catch (e) {
      log('[debug-mode-settings] update failed seq=%d', seq);
      // Revert only what this patch touched so a concurrent edit survives.
      setSettings(s => {
        if (!s) return s;
        const reverted = { ...s } as Record<string, unknown>;
        for (const k of keys) reverted[k] = prev[k];
        return reverted as unknown as DebugModeSettings;
      });
      if (seqRef.current === seq) setSaveError(messageOf(e, fallbacksRef.current.saveFallback));
      return false;
    } finally {
      if (seqRef.current === seq) setSaving(false);
    }
  }, []);

  return { settings, loading, loadError, saveError, saving, savedAt, reload, update };
}
