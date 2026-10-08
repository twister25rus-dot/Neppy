import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';
import { type IconType } from 'react-icons';
import { LuCodeXml, LuGitFork, LuMessageCircle } from 'react-icons/lu';
import { Link } from 'react-router-dom';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import { getDebugStatus } from '../../../services/api/debugModeApi';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { setThreadMode } from '../../../store/threadSlice';
import { resolveThreadMode, type ThreadMode } from '../../../types/thread';

const log = debug('neppy:conversations:thread-mode');

type Notice = { threadId: string; kind: 'repo' | 'error' };

const MODES: readonly { value: ThreadMode; icon: IconType; iconClassName?: string }[] = [
  { value: 'chat', icon: LuMessageCircle },
  // Upside-down fork: one supervisor node fanning out to its specialists.
  { value: 'orchestration', icon: LuGitFork, iconClassName: 'rotate-180' },
  { value: 'debug', icon: LuCodeXml },
];

/**
 * Chat / Orchestration / Debug tabs for one conversation.
 *
 * Chat is one assistant with full tools; Orchestration lets a supervisor
 * coordinate specialist agents; Debug routes turns to the repository-scoped
 * `debug_agent`. The mode is persisted on the thread by the core
 * (`threads_set_mode`), so switching mid-conversation keeps the same thread and
 * history. Chat/Orchestration flip optimistically and revert if the core
 * refuses. Debug first asks the core for the source repository — without one
 * there is nothing for the agent to work on, so the tab stays unselected and
 * says why — and leaving Debug via another tab is a plain switch to that tab.
 * `thread_mode_changed` events from another window land in the same slice, so
 * the control always shows the persisted truth.
 *
 * `size="lg"` is the new-chat hero variant; `sm` sits above a running
 * conversation's composer.
 */
export function ThreadModeToggle({
  threadId,
  size = 'sm',
}: {
  threadId: string | null;
  size?: 'sm' | 'lg';
}) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const mode = useAppSelector(state =>
    threadId ? resolveThreadMode(state.thread.threads.find(th => th.id === threadId)?.mode) : 'chat'
  );
  const [rawNotice, setNotice] = useState<Notice | null>(null);
  const [checking, setChecking] = useState(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  // A notice belongs to the thread it was raised on; switching threads hides it.
  const notice = rawNotice && rawNotice.threadId === threadId ? rawNotice.kind : null;

  const change = useCallback(
    async (next: ThreadMode, previous: ThreadMode, id: string): Promise<boolean> => {
      try {
        await dispatch(
          setThreadMode({ threadId: id, mode: next, previous, source: 'composer_toggle' })
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

  const choose = useCallback(
    async (next: ThreadMode) => {
      if (!threadId || next === mode || checking) return;
      log('toggle thread=%s %s->%s', threadId, mode, next);
      setNotice(null);
      if (next !== 'debug') {
        await change(next, mode, threadId);
        return;
      }
      setChecking(true);
      try {
        await getDebugStatus();
      } catch (error) {
        log('debug status failed, not switching thread=%s error=%o', threadId, error);
        if (mounted.current) {
          setNotice({ threadId, kind: 'repo' });
          setChecking(false);
        }
        return;
      }
      if (mounted.current) setChecking(false);
      await change('debug', mode, threadId);
    },
    [change, checking, mode, threadId]
  );

  if (!threadId) return null;

  const lg = size === 'lg';

  return (
    <div className={cn('flex flex-col gap-1.5', lg ? 'items-center' : 'items-start')}>
      <div
        role="radiogroup"
        aria-label={t('threadMode.label')}
        data-testid="thread-mode-toggle"
        data-mode={mode}
        data-size={size}
        className={cn(
          'inline-flex items-center rounded-full border border-line bg-surface-muted font-medium',
          lg ? 'gap-1 p-1 text-[15px]' : 'gap-0.5 p-0.5 text-xs'
        )}>
        {MODES.map(({ value, icon: Icon, iconClassName }) => {
          const selected = value === mode;
          const isDebug = value === 'debug';
          const label = isDebug ? t('debug.switch.label') : t(`threadMode.${value}.name`);
          const description = isDebug
            ? t('debug.switch.tooltip')
            : t(`threadMode.${value}.description`);
          return (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={selected}
              title={description}
              disabled={isDebug && checking}
              data-testid={`thread-mode-${value}`}
              data-analytics-id={`chat-thread-mode-${value}`}
              onClick={() => void choose(value)}
              className={cn(
                'inline-flex items-center rounded-full leading-none transition-colors',
                'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/30',
                'disabled:cursor-wait disabled:opacity-60',
                lg ? 'h-10 gap-2 px-5' : 'h-6 gap-1.5 px-2.5',
                selected
                  ? 'bg-content text-surface shadow-soft'
                  : 'text-content-muted hover:bg-surface-hover hover:text-content-secondary'
              )}>
              <Icon aria-hidden className={cn(lg ? 'h-4.5 w-4.5' : 'h-3.5 w-3.5', iconClassName)} />
              {label}
            </button>
          );
        })}
      </div>
      {notice === 'repo' ? (
        <p
          role="alert"
          data-testid="thread-mode-debug-notice"
          className="max-w-md text-center text-[11px] text-content-secondary">
          {t('debug.switch.repo.body')}{' '}
          <Link
            to="/settings/debug-mode"
            data-analytics-id="chat-thread-mode-debug-settings"
            className="font-medium text-primary-600 underline underline-offset-2">
            {t('settings.debugMode.bannerLink')}
          </Link>
        </p>
      ) : notice === 'error' ? (
        <span role="alert" className="text-[11px] text-coral">
          {t('threadMode.error')}
        </span>
      ) : null}
    </div>
  );
}

export default ThreadModeToggle;
