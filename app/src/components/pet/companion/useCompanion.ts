import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  actOnCompanionSuggestion,
  type CompanionActResult,
  type CompanionSettings,
  type CompanionSettingsPatch,
  type CompanionSocketEvent,
  type CompanionStatus,
  type CompanionSuggestion,
  fetchCompanionSuggestions,
  getCompanionSettings,
  getCompanionStatus,
  pauseCompanion,
  type PermissionKind,
  requestCompanionPermission,
  resumeCompanion,
  type SuggestionAction,
  updateCompanionSettings,
} from '../../../services/api/petCompanionApi';
import { socketService } from '../../../services/socketService';
import { companionDisplayState, type CompanionDisplayState } from '../petFormat';

const log = debug('pet:companion');

/** Status refresh cadence while the Pet page is mounted. */
export const COMPANION_POLL_MS = 10_000;

const SOCKET_EVENT = 'pet:companion';
const SUGGESTION_LIMIT = 20;

export interface UseCompanion {
  settings: CompanionSettings | null;
  status: CompanionStatus | null;
  suggestions: CompanionSuggestion[];
  /** True only until the first load settles. */
  loading: boolean;
  /** True when the first load failed (for example an older core without the companion). */
  loadError: boolean;
  displayState: CompanionDisplayState;
  refresh: () => Promise<void>;
  /** Saves a patch and stores the returned settings. Rejects on failure. */
  saveSettings: (patch: CompanionSettingsPatch) => Promise<CompanionSettings>;
  pause: (minutes?: number) => Promise<void>;
  resume: () => Promise<void>;
  requestPermission: (kind: PermissionKind) => Promise<void>;
  /** Runs a suggestion action and stores the updated suggestion. Rejects on failure. */
  act: (id: string, action: SuggestionAction, text?: string) => Promise<CompanionActResult>;
}

const upsert = (list: CompanionSuggestion[], next: CompanionSuggestion): CompanionSuggestion[] => {
  const idx = list.findIndex(s => s.id === next.id);
  if (idx === -1) return [next, ...list].slice(0, SUGGESTION_LIMIT);
  const copy = list.slice();
  copy[idx] = next;
  return copy;
};

/**
 * Loads the companion settings, status and recent suggestions, and keeps them
 * fresh: live `pet:companion` socket events plus a 10 second status poll that
 * exists only while the page is mounted. Suggestion text, titles and prompts
 * are never logged.
 */
export function useCompanion(): UseCompanion {
  const [settings, setSettings] = useState<CompanionSettings | null>(null);
  const [status, setStatus] = useState<CompanionStatus | null>(null);
  const [suggestions, setSuggestions] = useState<CompanionSuggestion[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const mounted = useRef(true);
  const seq = useRef(0);

  const refresh = useCallback(async () => {
    const mine = ++seq.current;
    log('refresh start seq=%d', mine);
    try {
      const [nextSettings, nextStatus, nextSuggestions] = await Promise.all([
        getCompanionSettings(),
        getCompanionStatus(),
        fetchCompanionSuggestions({ limit: SUGGESTION_LIMIT }),
      ]);
      if (!mounted.current || mine !== seq.current) return;
      setSettings(nextSettings);
      setStatus(nextStatus);
      setSuggestions(nextSuggestions);
      setLoadError(false);
      log(
        'refresh ok seq=%d state=%s suggestions=%d',
        mine,
        nextStatus.state,
        nextSuggestions.length
      );
    } catch (err) {
      log('refresh failed seq=%d err=%o', mine, err);
      if (!mounted.current || mine !== seq.current) return;
      setLoadError(true);
    } finally {
      if (mounted.current && mine === seq.current) setLoading(false);
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      const next = await getCompanionStatus();
      if (mounted.current) setStatus(next);
    } catch (err) {
      log('status poll failed: %o', err);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const timer = setInterval(() => {
      log('poll tick');
      void refreshStatus();
    }, COMPANION_POLL_MS);
    return () => {
      mounted.current = false;
      clearInterval(timer);
    };
  }, [refresh, refreshStatus]);

  useEffect(() => {
    const onEvent = (...args: unknown[]) => {
      const event = args[0] as CompanionSocketEvent | undefined;
      if (!event || typeof event !== 'object') return;
      if (event.type === 'state') {
        log('socket state=%s', event.state);
        setStatus(prev =>
          prev
            ? {
                ...prev,
                state: event.state,
                suspended_reason: event.suspended_reason ?? null,
                screen_capture_active: event.screen_capture_active ?? false,
              }
            : prev
        );
      } else if (event.type === 'suggestion' || event.type === 'suggestion_update') {
        if (!event.suggestion?.id) return;
        log('socket %s id=%s', event.type, event.suggestion.id);
        setSuggestions(prev => upsert(prev, event.suggestion));
      }
    };
    socketService.on(SOCKET_EVENT, onEvent);
    return () => socketService.off(SOCKET_EVENT, onEvent);
  }, []);

  const saveSettings = useCallback(async (patch: CompanionSettingsPatch) => {
    log('save fields=%s', Object.keys(patch).join(','));
    const next = await updateCompanionSettings(patch);
    if (mounted.current) setSettings(next);
    // Enabling or disabling changes the indicator right away.
    void getCompanionStatus()
      .then(s => mounted.current && setStatus(s))
      .catch(err => log('status after save failed: %o', err));
    return next;
  }, []);

  const pause = useCallback(async (minutes?: number) => {
    log('pause minutes=%s', minutes ?? 'until-resumed');
    const next = await pauseCompanion(minutes, 'ui');
    if (mounted.current) setStatus(next);
  }, []);

  const resume = useCallback(async () => {
    log('resume');
    const next = await resumeCompanion('ui');
    if (mounted.current) setStatus(next);
  }, []);

  const requestPermission = useCallback(
    async (kind: PermissionKind) => {
      log('request permission kind=%s', kind);
      try {
        const res = await requestCompanionPermission(kind);
        log('permission kind=%s state=%s opened=%s', kind, res.state, res.opened_settings);
      } finally {
        await refreshStatus();
      }
    },
    [refreshStatus]
  );

  const act = useCallback(async (id: string, action: SuggestionAction, text?: string) => {
    log('act id=%s action=%s', id, action);
    const result = await actOnCompanionSuggestion(id, action, text);
    if (mounted.current && result?.suggestion?.id) {
      setSuggestions(prev => upsert(prev, result.suggestion));
    }
    return result;
  }, []);

  return {
    settings,
    status,
    suggestions,
    loading,
    loadError,
    displayState: companionDisplayState(
      settings?.enabled === false ? 'off' : (status?.state ?? 'off'),
      status?.screen_capture_active
    ),
    refresh,
    saveSettings,
    pause,
    resume,
    requestPermission,
    act,
  };
}
