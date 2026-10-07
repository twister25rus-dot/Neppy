import { useCallback, useState } from 'react';

import {
  Button,
  CenteredLoadingState,
  ConfirmDialog,
  EmptyState,
  ErrorBanner,
} from '../../../components/ui';
import { useT } from '../../../lib/i18n/I18nContext';
import {
  type DebugCheckpoint,
  listDebugCheckpoints,
  rollbackDebug,
  type RollbackResult,
} from '../../../services/api/debugModeApi';
import { errorText, shortSha } from '../debugFormat';
import { formatDateTime } from './panelFormat';
import { useAsyncData } from './useAsyncData';

const CHECKPOINT_LIMIT = 30;

/** Recent checkpoints with a confirmed, reversible "Rollback to here". */
export function CheckpointList({ refreshKey = 0 }: { refreshKey?: number }) {
  const { t, locale } = useT();
  const load = useCallback(() => {
    void refreshKey;
    return listDebugCheckpoints(CHECKPOINT_LIMIT);
  }, [refreshKey]);
  const { data, loading, error, reload } = useAsyncData(load);

  const [target, setTarget] = useState<DebugCheckpoint | null>(null);
  const [busy, setBusy] = useState(false);
  const [rollbackError, setRollbackError] = useState<string | null>(null);
  const [result, setResult] = useState<RollbackResult | null>(null);

  const closeDialog = () => {
    if (busy) return;
    setTarget(null);
    setRollbackError(null);
  };

  const doRollback = async () => {
    if (!target || busy) return;
    setBusy(true);
    setRollbackError(null);
    try {
      const res = await rollbackDebug(target.id);
      setResult(res);
      setTarget(null);
      reload();
    } catch (e) {
      setRollbackError(errorText(e) || t('debug.panels.unknownError'));
    } finally {
      setBusy(false);
    }
  };

  if (loading && !data) return <CenteredLoadingState label={t('debug.panels.loading')} />;
  if (error && !data) {
    return (
      <ErrorBanner
        action={
          <button
            type="button"
            className="text-xs underline"
            data-analytics-id="debug-checkpoints-retry"
            onClick={reload}>
            {t('common.retry')}
          </button>
        }>
        {t('debug.panels.checkpoints.error').replace(
          '{error}',
          errorText(error) || t('debug.panels.unknownError')
        )}
      </ErrorBanner>
    );
  }

  return (
    <div className="space-y-3">
      {result ? (
        <div
          role="status"
          className="rounded-lg border border-sage-500/30 bg-sage-500/10 px-3 py-2 text-xs text-sage-700 dark:text-sage-300"
          data-testid="debug-rollback-result">
          <p className="font-medium">{t('debug.panels.checkpoints.rollbackDone')}</p>
          <p>
            {t('debug.panels.checkpoints.rollbackCounts')
              .replace('{restored}', String(result.restored.length))
              .replace('{removed}', String(result.removed.length))}
          </p>
          <p>
            {t('debug.panels.checkpoints.preRollback').replace(
              '{id}',
              result.pre_rollback_checkpoint_id
            )}
          </p>
        </div>
      ) : null}

      {data && data.length === 0 ? (
        <EmptyState label={t('debug.panels.checkpoints.empty')} />
      ) : (
        <ul className="divide-y divide-line rounded-lg border border-line">
          {(data ?? []).map(cp => (
            <li
              key={cp.id}
              className="flex items-center gap-3 px-3 py-2"
              data-testid="debug-checkpoint-row">
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm text-content">{cp.description}</p>
                <p className="truncate text-xs text-content-muted">
                  {formatDateTime(cp.created_at, locale)}
                  {cp.branch ? <span className="font-mono"> · {cp.branch}</span> : null}
                  <span className="font-mono"> · {shortSha(cp.head)}</span>
                </p>
              </div>
              <Button
                variant="secondary"
                size="sm"
                analyticsId="debug-checkpoint-rollback"
                data-testid="debug-checkpoint-rollback"
                onClick={() => {
                  setRollbackError(null);
                  setTarget(cp);
                }}>
                {t('debug.panels.checkpoints.rollback')}
              </Button>
            </li>
          ))}
        </ul>
      )}

      {target ? (
        <ConfirmDialog
          title={t('debug.panels.checkpoints.confirmTitle')}
          titleId="debug-checkpoint-rollback-title"
          confirmLabel={t('debug.panels.checkpoints.confirm')}
          busy={busy}
          destructive
          onConfirm={() => void doRollback()}
          onCancel={closeDialog}
          body={
            <div className="space-y-2">
              <p>
                {t('debug.panels.checkpoints.confirmBody').replace(
                  '{description}',
                  target.description
                )}
              </p>
              {rollbackError ? (
                <p role="alert" className="text-coral-600 dark:text-coral-300">
                  {t('debug.panels.checkpoints.rollbackError').replace('{error}', rollbackError)}
                </p>
              ) : null}
            </div>
          }
        />
      ) : null}
    </div>
  );
}

export default CheckpointList;
