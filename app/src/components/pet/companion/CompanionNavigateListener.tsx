import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import debug from 'debug';
import { useEffect } from 'react';
import { useNavigate } from 'react-router-dom';

import { isTauri } from '../../../utils/tauriCommands/common';

const log = debug('pet:companion:navigate');

/** Tauri event the tray "Pet..." item emits. */
export const COMPANION_NAVIGATE_EVENT = 'pet-companion://navigate';

/** Only in-app Pet paths are followed, so the event can never steer the router elsewhere. */
export const isPetPath = (path: unknown): path is string =>
  typeof path === 'string' && /^\/pet(\?[\w=&%.-]*)?$/.test(path);

/**
 * Renderless. Listens for the tray's `pet-companion://navigate` event and moves
 * the HashRouter to the requested Pet path (for example `/pet?tab=now`). Mount it
 * once, inside the Router, in always-mounted code: PetPage is not mounted when
 * the user is on another page, so it cannot host this listener itself.
 */
export default function CompanionNavigateListener() {
  const navigate = useNavigate();

  useEffect(() => {
    if (!isTauri()) return;
    let disposed = false;
    let unlisten: UnlistenFn | undefined;
    void (async () => {
      try {
        const off = await listen<{ path?: unknown }>(COMPANION_NAVIGATE_EVENT, e => {
          const path = e.payload?.path;
          if (!isPetPath(path)) {
            log('ignored navigate event with unexpected path');
            return;
          }
          log('navigate -> %s', path);
          navigate(path);
        });
        if (disposed) off();
        else unlisten = off;
      } catch (err) {
        log('listen failed: %o', err);
      }
    })();
    return () => {
      disposed = true;
      try {
        unlisten?.();
      } catch (err) {
        log('unlisten threw: %o', err);
      }
    };
  }, [navigate]);

  return null;
}
