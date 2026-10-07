import debug from 'debug';
import { Link } from 'react-router-dom';

import { Alert } from '../../components/ui';
import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import { DebugBanner } from './DebugBanner';
import { LastTaskCard } from './LastTaskCard';
import { DebugPanels } from './panels';
import { useDebugSnapshot } from './useDebugSnapshot';

const log = debug('neppy:debug:chrome');

/**
 * Everything Debug Mode adds around a normal conversation: the repo banner, the
 * last-task decision card and the collapsible diff / history / checkpoints
 * panels. Mounted by the chat view only while the open thread is in Debug mode,
 * so the snapshot polling (`useDebugSnapshot`) stops with the mode.
 *
 * No chat UI lives here; the conversation below is the ordinary one. When the
 * source repository cannot be resolved the banner is replaced by a short notice
 * that links to the Debug Mode settings, rather than showing stale facts.
 */
export function DebugThreadChrome({ threadId }: { threadId: string }) {
  const { t } = useT();
  const turnActive = useAppSelector(s => Boolean(s.thread.activeThreadIds?.[threadId]));
  const { status, lastTask, loading, refresh } = useDebugSnapshot(turnActive);
  // Re-fetch the diff/history/checkpoint panels whenever the latest task changes
  // state (a turn finishing, a commit, a rollback, a new task): the key is a
  // cheap hash of that state, so no effect or extra render is needed.
  const panelsRefresh = hashKey(
    lastTask ? `${lastTask.id}:${lastTask.status}:${lastTask.commit ?? ''}` : ''
  );

  if (loading) return null;

  if (!status) {
    log('repo unavailable thread=%s', threadId);
    return (
      <div className="shrink-0" data-testid="debug-thread-chrome" data-state="unavailable">
        <Alert
          variant="warning"
          data-testid="debug-unavailable"
          className="flex-wrap items-center gap-x-4 gap-y-1 rounded-none border-x-0 border-t-0 py-1.5 text-xs">
          <span className="font-semibold">{t('debug.unavailable.title')}</span>
          <Link
            to="/settings/debug-mode"
            data-analytics-id="debug-unavailable-settings-link"
            className="ml-auto font-medium underline underline-offset-2">
            {t('settings.debugMode.bannerLink')}
          </Link>
        </Alert>
      </div>
    );
  }

  return (
    <div
      className="max-h-[45vh] shrink-0 overflow-y-auto"
      data-testid="debug-thread-chrome"
      data-state="ready">
      <DebugBanner status={status} compact />
      {lastTask ? (
        <div className="pt-2">
          <LastTaskCard key={lastTask.id} task={lastTask} onChanged={() => void refresh()} />
        </div>
      ) : null}
      <details className="mx-4 my-2 rounded-lg border border-line" data-testid="debug-panels-slot">
        <summary className="cursor-pointer select-none px-3 py-1.5 text-xs font-medium text-content-secondary">
          {t('debug.panels.toggle')}
        </summary>
        <div className="max-h-[35vh] overflow-auto border-t border-line">
          <DebugPanels refreshKey={panelsRefresh} />
        </div>
      </details>
    </div>
  );
}

export default DebugThreadChrome;

function hashKey(value: string): number {
  let h = 0;
  for (let i = 0; i < value.length; i += 1) h = (h * 31 + value.charCodeAt(i)) | 0;
  return h;
}
