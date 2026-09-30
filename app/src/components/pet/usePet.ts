import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import {
  fetchPetFeed,
  fetchPetInbox,
  getPet,
  type PetFeed,
  type PetInbox,
  type PetProfile,
} from '../../services/api/petApi';
import { socketService } from '../../services/socketService';

const log = debug('pet:usePet');

/** Background refresh cadence while the Pet page is mounted. */
export const PET_POLL_MS = 30_000;

/** Core notifications raised by Pet mode carry ids with this prefix. */
const PET_NOTIFICATION_PREFIX = 'pet-';

export interface UsePet {
  pet: PetProfile | null;
  feed: PetFeed | null;
  inbox: PetInbox | null;
  /** True only until the first load settles; later refreshes are silent. */
  loading: boolean;
  /** True when the latest load failed. Cleared by the next success. */
  loadError: boolean;
  /** Bumps on every successful load so child lists can refetch their own data. */
  tick: number;
  refresh: () => Promise<void>;
  /** Replace the cached profile after a local save, without a round trip. */
  applyPet: (next: PetProfile) => void;
}

/**
 * Loads the pet profile, feed and inbox, and keeps them fresh: a 30 second poll
 * plus an immediate refresh whenever a `pet-` core notification arrives.
 * Everything is torn down on unmount. Note text is never logged.
 */
export function usePet(): UsePet {
  const [pet, setPet] = useState<PetProfile | null>(null);
  const [feed, setFeed] = useState<PetFeed | null>(null);
  const [inbox, setInbox] = useState<PetInbox | null>(null);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState(false);
  const [tick, setTick] = useState(0);
  const seq = useRef(0);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    const mine = ++seq.current;
    log('refresh start seq=%d', mine);
    try {
      const [nextPet, nextFeed, nextInbox] = await Promise.all([
        getPet(),
        fetchPetFeed(),
        fetchPetInbox(),
      ]);
      // A newer refresh superseded this one, or the page unmounted.
      if (!mounted.current || mine !== seq.current) return;
      setPet(nextPet);
      setFeed(nextFeed);
      setInbox(nextInbox);
      setLoadError(false);
      setTick(n => n + 1);
      log(
        'refresh ok seq=%d digests=%d notes=%d proposals=%d approvals=%d',
        mine,
        nextFeed.digests.length,
        nextFeed.notes.length,
        nextInbox.proposals.length,
        nextInbox.approvals.length
      );
    } catch (err) {
      log('refresh failed seq=%d err=%o', mine, err);
      if (!mounted.current || mine !== seq.current) return;
      setLoadError(true);
    } finally {
      if (mounted.current && mine === seq.current) setLoading(false);
    }
  }, []);

  const applyPet = useCallback((next: PetProfile) => setPet(next), []);

  useEffect(() => {
    mounted.current = true;
    void refresh();
    const timer = setInterval(() => {
      log('poll tick');
      void refresh();
    }, PET_POLL_MS);
    return () => {
      mounted.current = false;
      clearInterval(timer);
    };
  }, [refresh]);

  useEffect(() => {
    const onNotification = (...args: unknown[]) => {
      const id = (args[0] as { id?: unknown } | undefined)?.id;
      if (typeof id !== 'string' || !id.startsWith(PET_NOTIFICATION_PREFIX)) return;
      log('core_notification kind=%s: refreshing', id.split(':')[0]);
      void refresh();
    };
    socketService.on('core_notification', onNotification);
    return () => socketService.off('core_notification', onNotification);
  }, [refresh]);

  return { pet, feed, inbox, loading, loadError, tick, refresh, applyPet };
}
