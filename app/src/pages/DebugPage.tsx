import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';
import { useNavigate, useParams } from 'react-router-dom';

import { Button } from '../components/ui';
import { ConversationsPage } from '../features/conversations/Conversations';
import { isDebugThread } from '../features/conversations/utils/threadFilter';
import { DebugBanner } from '../features/debug/DebugBanner';
import { errorText } from '../features/debug/debugFormat';
import { LastTaskCard } from '../features/debug/LastTaskCard';
import { DebugPanels } from '../features/debug/panels';
import { useDebugSnapshot } from '../features/debug/useDebugSnapshot';
import { useT } from '../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../store/hooks';
import { createDebugThread, loadThreads, setSelectedThread } from '../store/threadSlice';

const log = debug('neppy:debug:page');

const debugThreadPath = (id: string) => `/debug/${encodeURIComponent(id)}`;

/**
 * `/debug/:threadId?`: the Debug Mode surface.
 *
 * It is the normal conversation view (`ConversationsPage`, scoped to Debug
 * threads) wrapped in a persistent banner, a "last task" decision card and a
 * "New debug task" button; no chat UI is forked. Nothing is created unless
 * `debug_mode_status` succeeds, so a build without the source repository never
 * leaves stray debug threads behind.
 */
const DebugPage = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const { threadId } = useParams<{ threadId?: string }>();
  const turnActive = useAppSelector(s =>
    threadId ? Boolean(s.thread.activeThreadIds?.[threadId]) : false
  );
  const { status, statusError, lastTask, loading, refresh } = useDebugSnapshot(turnActive);
  const [verifiedId, setVerifiedId] = useState<string | null>(null);
  const [threadError, setThreadError] = useState(false);
  const [creating, setCreating] = useState(false);
  // Re-fetch the diff/history/checkpoint panels whenever the latest task changes
  // state (a turn finishing, a commit, a rollback, a new task): the key is a
  // cheap hash of that state, so no effect or extra render is needed.
  const panelsRefresh = hashKey(
    lastTask ? `${lastTask.id}:${lastTask.status}:${lastTask.commit ?? ''}` : ''
  );

  const available = status !== null;

  // Resolve which debug thread to show. Only runs once the repo is known good.
  useEffect(() => {
    if (!available) return;
    if (threadId && verifiedId === threadId) return;
    let cancelled = false;
    void (async () => {
      try {
        const data = await dispatch(loadThreads()).unwrap();
        if (cancelled) return;
        const requested = threadId ? data.threads.find(th => th.id === threadId) : undefined;
        if (requested && isDebugThread(requested)) {
          dispatch(setSelectedThread(requested.id));
          setVerifiedId(requested.id);
          return;
        }
        const newest = data.threads
          .filter(isDebugThread)
          .sort(
            (a, b) => new Date(b.lastMessageAt).getTime() - new Date(a.lastMessageAt).getTime()
          )[0];
        const target = newest ?? (await dispatch(createDebugThread()).unwrap());
        if (cancelled) return;
        log('resolved debug thread created=%s', !newest);
        dispatch(setSelectedThread(target.id));
        setVerifiedId(target.id);
        navigate(debugThreadPath(target.id), { replace: true });
      } catch (error) {
        if (cancelled) return;
        log('thread resolution failed: %s', errorText(error) ? 'error' : 'unknown');
        setThreadError(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [available, threadId, verifiedId, dispatch, navigate]);

  const startNewTask = useCallback(async () => {
    if (creating) return;
    setCreating(true);
    setThreadError(false);
    try {
      const thread = await dispatch(createDebugThread()).unwrap();
      dispatch(setSelectedThread(thread.id));
      setVerifiedId(thread.id);
      navigate(debugThreadPath(thread.id));
    } catch (error) {
      log('new debug task failed: %s', errorText(error) ? 'error' : 'unknown');
      setThreadError(true);
    } finally {
      setCreating(false);
    }
  }, [creating, dispatch, navigate]);

  if (loading) {
    return (
      <div className="p-6 text-sm text-content-muted" data-testid="debug-loading">
        {t('debug.loading')}
      </div>
    );
  }

  if (!status) {
    return (
      <div className="mx-auto max-w-xl space-y-3 p-6" data-testid="debug-unavailable">
        <h1 className="text-lg font-semibold text-content">{t('debug.unavailable.title')}</h1>
        <p className="text-sm text-content-secondary">{t('debug.unavailable.body')}</p>
        {statusError ? (
          <p className="break-words font-mono text-xs text-content-muted">
            {t('debug.unavailable.detail').replace('{error}', statusError)}
          </p>
        ) : null}
        <Button
          size="sm"
          variant="secondary"
          analyticsId="debug-unavailable-retry"
          onClick={() => void refresh()}>
          {t('debug.unavailable.retry')}
        </Button>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col" data-testid="debug-page">
      <DebugBanner status={status} />
      <div className="flex items-center justify-end gap-2 px-4 py-2">
        {threadError ? (
          <span
            role="alert"
            className="mr-auto text-xs text-coral"
            data-testid="debug-thread-error">
            {t('debug.createFailed')}
          </span>
        ) : null}
        <Button
          size="xs"
          variant="secondary"
          analyticsId="debug-new-task"
          data-testid="debug-new-task"
          disabled={creating}
          onClick={() => void startNewTask()}>
          {t('debug.newTask')}
        </Button>
      </div>
      {lastTask ? (
        <LastTaskCard key={lastTask.id} task={lastTask} onChanged={() => void refresh()} />
      ) : null}
      <details className="mx-4 mb-2 rounded-lg border border-line" data-testid="debug-panels-slot">
        <summary className="cursor-pointer select-none px-3 py-1.5 text-xs font-medium text-content-secondary">
          {t('debug.panels.toggle')}
        </summary>
        <div className="max-h-[45vh] overflow-auto border-t border-line">
          <DebugPanels refreshKey={panelsRefresh} />
        </div>
      </details>
      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        {threadId && verifiedId === threadId ? <ConversationsPage scope="debug" /> : null}
      </div>
    </div>
  );
};

export default DebugPage;

function hashKey(value: string): number {
  let h = 0;
  for (let i = 0; i < value.length; i += 1) h = (h * 31 + value.charCodeAt(i)) | 0;
  return h;
}
