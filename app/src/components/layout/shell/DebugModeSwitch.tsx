import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { LuCodeXml } from 'react-icons/lu';
import { Link, useMatch } from 'react-router-dom';

import { isDebugThread } from '../../../features/conversations/utils/threadFilter';
import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import { getDebugStatus } from '../../../services/api/debugModeApi';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { setThreadMode } from '../../../store/threadSlice';
import { resolveThreadMode, type ThreadMode } from '../../../types/thread';
import { PopoverAnchor, PopoverContent, PopoverRoot, Tooltip } from '../../ui';

const log = debug('neppy:debug:switch');

/**
 * The mode each thread was in before Debug was switched on, so switching off
 * returns to Chat or Orchestration as it was. Module-level so it survives the
 * switch unmounting while another route is open; a reload forgets it and the
 * default (Chat) applies, which is also the mode every new thread starts in.
 */
const previousModes = new Map<string, ThreadMode>();

type Notice = { threadId: string; kind: 'repo' | 'error' };

/**
 * Debug Mode, one click from the open chat thread.
 *
 * Sits beside the MLX control in the top bar. ON puts the open thread in Debug
 * mode (the core then routes its turns to the repository-scoped `debug_agent`),
 * OFF restores the mode it had before. The state is the thread's persisted
 * mode, so it is right after a reload or a thread change and agrees with any
 * other window that toggles it (`thread_mode_changed` lands in the same slice).
 *
 * Turning it on first asks the core for the source repository; without one
 * there is nothing for the agent to work on, so the switch stays off and says
 * why. Hidden outside `/chat` and while no thread is open.
 */
export default function DebugModeSwitch() {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const onChatRoute = useMatch({ path: '/chat', end: false }) !== null;
  const threadId = useAppSelector(state => state.thread.selectedThreadId);
  const thread = useAppSelector(state =>
    state.thread.selectedThreadId
      ? state.thread.threads.find(th => th.id === state.thread.selectedThreadId)
      : undefined
  );
  const [checking, setChecking] = useState(false);
  const [rawNotice, setNotice] = useState<Notice | null>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // A notice belongs to the thread it was raised on; switching threads hides it.
  const notice = rawNotice && rawNotice.threadId === threadId ? rawNotice : null;

  const on = thread ? isDebugThread(thread) : false;
  const mode: ThreadMode = thread ? resolveThreadMode(thread.mode) : 'chat';

  const change = useCallback(
    async (next: ThreadMode, previous: ThreadMode, id: string): Promise<boolean> => {
      try {
        await dispatch(
          setThreadMode({ threadId: id, mode: next, previous, source: 'topbar_debug_switch' })
        ).unwrap();
        log('mode set thread=%s %s->%s', id, previous, next);
        return true;
      } catch (error) {
        // The slice reverts the optimistic value on rejection.
        log('set mode failed thread=%s next=%s error=%o', id, next, error);
        if (mounted.current) setNotice({ threadId: id, kind: 'error' });
        return false;
      }
    },
    [dispatch]
  );

  const toggle = useCallback(async () => {
    if (!threadId || checking) return;
    setNotice(null);
    if (on) {
      const restore = previousModes.get(threadId) ?? 'chat';
      log('switch off thread=%s restore=%s', threadId, restore);
      const ok = await change(restore === 'debug' ? 'chat' : restore, 'debug', threadId);
      if (ok) previousModes.delete(threadId);
      return;
    }
    setChecking(true);
    try {
      await getDebugStatus();
    } catch (error) {
      log('status failed, not switching thread=%s error=%o', threadId, error);
      if (mounted.current) {
        setNotice({ threadId, kind: 'repo' });
        setChecking(false);
      }
      return;
    }
    if (mounted.current) setChecking(false);
    log('switch on thread=%s previous=%s', threadId, mode);
    const ok = await change('debug', mode, threadId);
    if (ok) previousModes.set(threadId, mode);
  }, [threadId, checking, on, mode, change]);

  if (!onChatRoute || !threadId || !thread) return null;

  return (
    <PopoverRoot open={notice !== null} onOpenChange={open => !open && setNotice(null)}>
      <Tooltip label={t('debug.switch.tooltip')} side="bottom" align="end" multiline>
        <PopoverAnchor asChild>
          <button
            type="button"
            role="switch"
            aria-checked={on}
            aria-label={t('debug.switch.label')}
            disabled={checking}
            data-testid="debug-mode-switch"
            data-analytics-id="topbar-debug-toggle"
            onClick={() => void toggle()}
            className={cn(
              'inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-xs font-medium transition-colors',
              'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/30',
              'disabled:cursor-wait disabled:opacity-60',
              on
                ? 'text-amber-700 hover:bg-amber-50 dark:text-amber-200 dark:hover:bg-amber-500/10'
                : 'text-content-muted hover:bg-surface-hover hover:text-content-secondary'
            )}>
            <LuCodeXml aria-hidden className="h-3.5 w-3.5 shrink-0" />
            {t('debug.switch.label')}
            <span
              aria-hidden
              className={cn(
                'relative inline-block h-3 w-5 shrink-0 rounded-full transition-colors',
                on ? 'bg-amber-500' : 'bg-surface-strong'
              )}>
              <span
                className={cn(
                  'absolute top-0.5 h-2 w-2 rounded-full bg-surface shadow-xs transition-transform',
                  on ? 'translate-x-2.5' : 'translate-x-0.5'
                )}
              />
            </span>
          </button>
        </PopoverAnchor>
      </Tooltip>

      <PopoverContent align="end" className="w-72 p-3" data-testid="debug-mode-switch-notice">
        {notice?.kind === 'repo' ? (
          <div className="space-y-1.5">
            <p className="text-sm font-semibold text-content">{t('debug.unavailable.title')}</p>
            <p className="text-xs text-content-secondary">{t('debug.switch.repo.body')}</p>
            <Link
              to="/settings/debug-mode"
              data-analytics-id="topbar-debug-toggle-settings"
              onClick={() => setNotice(null)}
              className="inline-block text-xs font-medium text-primary-600 underline underline-offset-2">
              {t('settings.debugMode.bannerLink')}
            </Link>
          </div>
        ) : (
          <p role="alert" className="text-xs text-coral-500">
            {t('threadMode.error')}
          </p>
        )}
      </PopoverContent>
    </PopoverRoot>
  );
}
