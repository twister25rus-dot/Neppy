import { useCallback, useMemo, useState } from 'react';

import {
  Badge,
  type BadgeVariant,
  CenteredLoadingState,
  EmptyState,
  ErrorBanner,
} from '../../../components/ui';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  type DebugTask,
  type DebugTaskStatus,
  getDebugTask,
  listDebugTasks,
} from '../../../services/api/debugModeApi';
import { errorText, shortSha } from '../debugFormat';
import { DiffViewer } from './DiffViewer';
import { dayKey, formatDateTime, formatDay, truncateText } from './panelFormat';
import { useAsyncData } from './useAsyncData';

const TASK_LIMIT = 50;
const REQUEST_CHARS = 90;

const STATUS_META: Record<DebugTaskStatus, { icon: string; tone: BadgeVariant }> = {
  pass: { icon: '✓', tone: 'success' },
  failed: { icon: '✗', tone: 'danger' },
  partial: { icon: '◐', tone: 'warning' },
  rolled_back: { icon: '↩', tone: 'neutral' },
  planning: { icon: '…', tone: 'primary' },
  editing: { icon: '…', tone: 'primary' },
  validating: { icon: '…', tone: 'primary' },
};

function StatusChip({ status }: { status: DebugTaskStatus }) {
  const { t } = useT();
  const meta = STATUS_META[status] ?? STATUS_META.planning;
  const statusLabel = (s: DebugTaskStatus): string => {
    switch (s) {
      case 'pass':
        return t('debug.panels.history.status.pass');
      case 'failed':
        return t('debug.panels.history.status.failed');
      case 'partial':
        return t('debug.panels.history.status.partial');
      case 'rolled_back':
        return t('debug.panels.history.status.rolled_back');
      case 'validating':
        return t('debug.panels.history.status.validating');
      case 'editing':
        return t('debug.panels.history.status.editing');
      default:
        return t('debug.panels.history.status.planning');
    }
  };
  return (
    <Badge variant={meta.tone} data-testid="debug-task-status">
      <span aria-hidden="true" className="mr-1">
        {meta.icon}
      </span>
      {statusLabel(status)}
    </Badge>
  );
}

function TaskDetails({ taskId }: { taskId: string }) {
  const { t, locale } = useT();
  const [showDiff, setShowDiff] = useState(false);
  const load = useCallback(() => getDebugTask(taskId), [taskId]);
  const { data: task, loading, error } = useAsyncData(load);

  if (loading) return <CenteredLoadingState label={t('debug.panels.loading')} />;
  if (error) {
    return (
      <ErrorBanner>
        {t('debug.panels.history.detailError').replace(
          '{error}',
          errorText(error) || t('debug.panels.unknownError')
        )}
      </ErrorBanner>
    );
  }
  if (!task) return null;

  return (
    <div className="space-y-3 text-xs" data-testid="debug-task-details">
      <p className="whitespace-pre-wrap text-content-secondary">
        {task.summary ?? t('debug.panels.history.noSummary')}
      </p>
      <div>
        <h4 className="mb-1 font-medium text-content">{t('debug.panels.history.validation')}</h4>
        {task.validation.length === 0 ? (
          <p className="italic text-content-faint">{t('debug.panels.history.noValidation')}</p>
        ) : (
          <ul className="space-y-0.5">
            {task.validation.map((v, i) => (
              <li key={`${v.at}-${i}`} className="flex items-center gap-2">
                <span
                  aria-hidden="true"
                  className={
                    v.passed
                      ? 'text-sage-700 dark:text-sage-300'
                      : 'text-coral-600 dark:text-coral-300'
                  }>
                  {v.passed ? '✓' : '✗'}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-content-secondary">
                  {v.check_id ?? v.command.join(' ')}
                </span>
                <span className="text-content-muted">
                  {v.timed_out
                    ? t('debug.panels.history.timedOut')
                    : v.passed
                      ? t('debug.panels.history.passed')
                      : t('debug.panels.history.failed')}
                </span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
        <dt className="text-content-muted">{t('debug.panels.history.checkpoint')}</dt>
        <dd className="truncate font-mono text-content-secondary">
          {task.checkpoint_id ?? t('debug.panels.history.none')}
        </dd>
        <dt className="text-content-muted">{t('debug.panels.history.branch')}</dt>
        <dd className="truncate font-mono text-content-secondary">
          {task.branch ?? t('debug.panels.history.none')}
        </dd>
        <dt className="text-content-muted">{t('debug.panels.history.commit')}</dt>
        <dd className="truncate font-mono text-content-secondary">
          {task.commit ? shortSha(task.commit) : t('debug.panels.history.notCommitted')}
        </dd>
        <dt className="text-content-muted">{t('debug.panels.history.updated')}</dt>
        <dd className="text-content-secondary">{formatDateTime(task.updated_at, locale)}</dd>
      </dl>
      {task.checkpoint_id ? (
        <div className="space-y-2">
          <button
            type="button"
            aria-expanded={showDiff}
            data-analytics-id="debug-history-toggle-diff"
            data-testid="debug-history-view-diff"
            onClick={() => setShowDiff(s => !s)}
            className="rounded-md border border-line px-2 py-1 text-xs text-content-secondary hover:bg-surface-hover focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25">
            {showDiff ? t('debug.panels.history.hideDiff') : t('debug.panels.history.viewDiff')}
          </button>
          {showDiff ? <DiffViewer checkpointId={task.checkpoint_id} /> : null}
        </div>
      ) : null}
    </div>
  );
}

function TaskRow({ task }: { task: DebugTask }) {
  const { t, locale } = useT();
  const [open, setOpen] = useState(false);
  const panelId = `debug-task-${task.id}`;
  return (
    <li className="px-3 py-2" data-testid="debug-task-row">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        data-analytics-id="debug-history-toggle-task"
        onClick={() => setOpen(o => !o)}
        className="flex w-full items-center gap-3 text-left focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25">
        <StatusChip status={task.status} />
        <span className="min-w-0 flex-1 truncate text-sm text-content">
          {truncateText(task.request, REQUEST_CHARS)}
        </span>
        <span className="shrink-0 text-xs text-content-muted">
          {t('debug.panels.history.filesChanged').replace(
            '{count}',
            String(task.files_changed.length)
          )}
        </span>
        <span className="shrink-0 text-xs text-content-faint">
          {new Intl.DateTimeFormat(locale, { timeStyle: 'short' }).format(
            new Date(task.created_at)
          )}
        </span>
      </button>
      {open ? (
        <div id={panelId} className="mt-2 border-t border-line pt-2">
          <TaskDetails taskId={task.id} />
        </div>
      ) : null}
    </li>
  );
}

/** Last 50 debug tasks grouped by day; rows expand to show the full record. */
export function TaskHistory({ refreshKey = 0 }: { refreshKey?: number }) {
  const { t, locale } = useT();
  const load = useCallback(() => {
    void refreshKey;
    return listDebugTasks(TASK_LIMIT);
  }, [refreshKey]);
  const { data, loading, error, reload } = useAsyncData(load);

  const groups = useMemo(() => {
    const map = new Map<string, { label: string; tasks: DebugTask[] }>();
    for (const task of data ?? []) {
      const key = dayKey(task.created_at);
      const group = map.get(key) ?? { label: formatDay(task.created_at, locale), tasks: [] };
      group.tasks.push(task);
      map.set(key, group);
    }
    return [...map.entries()];
  }, [data, locale]);

  if (loading) return <CenteredLoadingState label={t('debug.panels.loading')} />;
  if (error) {
    return (
      <ErrorBanner
        action={
          <button
            type="button"
            className="text-xs underline"
            data-analytics-id="debug-history-retry"
            onClick={reload}>
            {t('common.retry')}
          </button>
        }>
        {t('debug.panels.history.error').replace(
          '{error}',
          errorText(error) || t('debug.panels.unknownError')
        )}
      </ErrorBanner>
    );
  }
  if (groups.length === 0) return <EmptyState label={t('debug.panels.history.empty')} />;

  return (
    <div className="space-y-4">
      {groups.map(([key, group]) => (
        <section key={key} data-testid="debug-task-day">
          <h3 className="mb-1 text-xs font-medium text-content-muted">{group.label}</h3>
          <ul className="divide-y divide-line rounded-lg border border-line">
            {group.tasks.map(task => (
              <TaskRow key={task.id} task={task} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

export default TaskHistory;
