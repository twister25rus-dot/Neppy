import debug from 'debug';
import { useCallback, useState } from 'react';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { setThreadMode } from '../../../store/threadSlice';
import { resolveThreadMode, type ThreadMode } from '../../../types/thread';

const log = debug('neppy:conversations:thread-mode');

/**
 * The modes the toggle offers. `debug` is deliberately absent: Debug threads
 * are created only from the Debug page (it scopes the agent to the app's
 * source repository), so a normal chat can neither enter nor leave it here.
 */
const MODES: readonly ThreadMode[] = ['chat', 'orchestration'];

/**
 * Compact Chat / Orchestration switch for one conversation.
 *
 * Chat is one assistant with full tools; Orchestration lets a supervisor
 * coordinate specialist agents. The mode is persisted on the thread by the core
 * (`threads_set_mode`), so switching mid-conversation keeps the same thread and
 * history, and the next turn is rebuilt for the new mode. The toggle flips
 * optimistically and reverts if the core refuses; `thread_mode_changed` events
 * from another window land in the same slice, so the control always shows the
 * persisted truth. New (and pre-existing) threads read as Chat.
 */
export function ThreadModeToggle({ threadId }: { threadId: string | null }) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const mode = useAppSelector(state =>
    threadId ? resolveThreadMode(state.thread.threads.find(th => th.id === threadId)?.mode) : 'chat'
  );
  const [failed, setFailed] = useState(false);

  const choose = useCallback(
    (next: ThreadMode) => {
      if (!threadId || next === mode) return;
      log('toggle thread=%s %s->%s', threadId, mode, next);
      setFailed(false);
      dispatch(setThreadMode({ threadId, mode: next, previous: mode, source: 'composer_toggle' }))
        .unwrap()
        .catch((error: unknown) => {
          log('set mode failed thread=%s error=%o', threadId, error);
          setFailed(true);
        });
    },
    [dispatch, mode, threadId]
  );

  // A Debug thread is not switchable from here; show nothing rather than a
  // control that would silently drop the thread out of Debug mode.
  if (!threadId || mode === 'debug') return null;

  return (
    <div className="flex items-center gap-1.5">
      <div
        role="radiogroup"
        aria-label={t('threadMode.label')}
        data-testid="thread-mode-toggle"
        data-mode={mode}
        className="inline-flex h-6 items-center rounded-full border border-line bg-surface p-0.5 text-[11px] font-medium">
        {MODES.map(value => {
          const selected = value === mode;
          const description = t(`threadMode.${value}.description`);
          return (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={selected}
              title={description}
              data-testid={`thread-mode-${value}`}
              data-analytics-id={`chat-thread-mode-${value}`}
              onClick={() => choose(value)}
              className={cn(
                'h-5 rounded-full px-2 leading-none transition-colors focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/30',
                selected
                  ? 'bg-content text-surface'
                  : 'text-content-muted hover:bg-surface-hover hover:text-content-secondary'
              )}>
              {t(`threadMode.${value}.name`)}
            </button>
          );
        })}
      </div>
      {failed ? (
        <span role="alert" className="text-[11px] text-coral">
          {t('threadMode.error')}
        </span>
      ) : null}
    </div>
  );
}

export default ThreadModeToggle;
