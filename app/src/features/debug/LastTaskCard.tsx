import { useState } from 'react';

import { Badge, type BadgeVariant, Button } from '../../components/ui';
import { useT } from '../../lib/i18n/I18nContext';
import {
  commitDebugTask,
  type DebugTask,
  type DebugTaskStatus,
  rollbackDebug,
} from '../../services/api/debugModeApi';
import { canCommitTask, canRollbackTask, errorText, shortSha } from './debugFormat';
import { CommitDialog, RollbackDialog } from './DebugTaskDialogs';

const STATUS_TONE: Record<DebugTaskStatus, BadgeVariant> = {
  planning: 'neutral',
  editing: 'primary',
  validating: 'primary',
  pass: 'success',
  partial: 'warning',
  failed: 'danger',
  rolled_back: 'neutral',
};

type Dialog = 'commit' | 'rollback' | null;

/**
 * Compact summary of the most recent debug task with its three decisions:
 * Commit (stages only the task's files), Rollback (reversible), and Keep (just
 * dismisses the card for this task). Errors from the core are shown inline.
 */
export function LastTaskCard({ task, onChanged }: { task: DebugTask; onChanged: () => void }) {
  const { t } = useT();
  const [dismissedId, setDismissedId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  if (dismissedId === task.id) return null;

  const close = () => {
    if (busy) return;
    setDialog(null);
    setError(null);
  };

  const doCommit = async (message: string) => {
    setBusy(true);
    setError(null);
    try {
      const r = await commitDebugTask(task.id, message);
      setNotice(t('debug.commit.done').replace('{sha}', shortSha(r.commit)));
      setDialog(null);
      onChanged();
    } catch (e) {
      setError(errorText(e) || t('debug.unknownError'));
    } finally {
      setBusy(false);
    }
  };

  const doRollback = async () => {
    if (!task.checkpoint_id) return;
    setBusy(true);
    setError(null);
    try {
      await rollbackDebug(task.checkpoint_id);
      setNotice(t('debug.rollback.done'));
      setDialog(null);
      onChanged();
    } catch (e) {
      setError(errorText(e) || t('debug.unknownError'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section
      aria-label={t('debug.lastTask.title')}
      data-testid="debug-last-task"
      className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line bg-surface px-4 py-2 text-sm">
      <span className="text-xs font-semibold uppercase tracking-wide text-content-muted">
        {t('debug.lastTask.title')}
      </span>
      <Badge variant={STATUS_TONE[task.status]} data-testid="debug-task-status">
        {t(`debug.status.${task.status}`)}
      </Badge>
      <span className="text-xs text-content-secondary" data-testid="debug-task-files">
        {t('debug.lastTask.files').replace('{count}', String(task.files_changed.length))}
      </span>
      {task.commit ? (
        <span className="font-mono text-xs text-content-secondary">
          {t('debug.lastTask.committed').replace('{sha}', shortSha(task.commit))}
        </span>
      ) : null}
      <div className="ml-auto flex items-center gap-2">
        <Button
          size="xs"
          analyticsId="debug-task-commit"
          data-testid="debug-commit"
          disabled={!canCommitTask(task)}
          onClick={() => {
            setError(null);
            setNotice(null);
            setDialog('commit');
          }}>
          {t('debug.action.commit')}
        </Button>
        <Button
          size="xs"
          variant="secondary"
          analyticsId="debug-task-keep"
          data-testid="debug-keep"
          onClick={() => setDismissedId(task.id)}>
          {t('debug.action.keep')}
        </Button>
        <Button
          size="xs"
          variant="secondary"
          tone="danger"
          analyticsId="debug-task-rollback"
          data-testid="debug-rollback"
          disabled={!canRollbackTask(task)}
          onClick={() => {
            setError(null);
            setNotice(null);
            setDialog('rollback');
          }}>
          {t('debug.action.rollback')}
        </Button>
      </div>
      {notice ? (
        <p
          role="status"
          className="w-full text-xs text-sage-700 dark:text-sage-300"
          data-testid="debug-notice">
          {notice}
        </p>
      ) : null}
      {error && dialog === null ? (
        <p role="alert" className="w-full text-xs text-coral" data-testid="debug-task-error">
          {error}
        </p>
      ) : null}
      {dialog === 'commit' ? (
        <CommitDialog
          task={task}
          busy={busy}
          error={error}
          onConfirm={m => void doCommit(m)}
          onCancel={close}
        />
      ) : null}
      {dialog === 'rollback' ? (
        <RollbackDialog
          busy={busy}
          error={error}
          onConfirm={() => void doRollback()}
          onCancel={close}
        />
      ) : null}
    </section>
  );
}
