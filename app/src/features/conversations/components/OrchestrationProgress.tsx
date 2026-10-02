import { useState } from 'react';

import AgentRunPhaseStepper from '../../../components/orchestration/AgentRunPhaseStepper';
import AgentRunRowItem from '../../../components/orchestration/AgentRunRow';
import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import { useSubagentRuns } from '../../../lib/orchestration/useSubagentRuns';
import { useAppSelector } from '../../../store/hooks';
import { resolveThreadMode } from '../../../types/thread';

/**
 * Orchestration-mode execution history for one conversation: where the
 * supervisor's team is on the Researching → … → Completed path, and a list of
 * the runs behind it that the user can open. Backed by the same run-ledger
 * projection as the cross-conversation "Agent runs" view (filtered to this
 * thread); the in-thread helper rows (live tool calls) stay in the transcript.
 *
 * Renders nothing in Chat mode, and nothing in Orchestration mode until the
 * thread has actually delegated (an empty ledger is a hint, not a panel).
 */
export function OrchestrationProgress({
  threadId,
  onOpenAllRuns,
}: {
  threadId: string | null;
  onOpenAllRuns?: () => void;
}) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const isOrchestration = useAppSelector(state =>
    threadId
      ? resolveThreadMode(state.thread.threads.find(th => th.id === threadId)?.mode) ===
        'orchestration'
      : false
  );
  // Refetch when the turn moves (started / settled) so a finished run shows
  // its outcome without waiting for the next poll.
  const lifecycle = useAppSelector(state =>
    threadId ? state.chatRuntime.inferenceTurnLifecycleByThread?.[threadId] : undefined
  );
  const { data, loading, error, refresh } = useSubagentRuns(
    { threadId: threadId ?? undefined, limit: 50 },
    { enabled: isOrchestration && threadId !== null, refreshKey: lifecycle ?? 'idle' }
  );

  if (!isOrchestration || !threadId) return null;

  const summary = data?.threads.find(th => th.threadId === threadId) ?? null;
  // The hook keeps the previous response while a new thread's loads; scope it.
  const runs = (data?.runs ?? []).filter(run => run.threadId === threadId);

  if (loading) {
    return (
      <p
        role="status"
        data-testid="orchestration-progress-loading"
        className="px-1 pb-2 text-[11px] text-content-faint">
        {t('orchestrationRuns.loading')}
      </p>
    );
  }
  if (error && !data) {
    return (
      <div
        role="alert"
        data-testid="orchestration-progress-error"
        className="mb-2 flex items-center gap-2 px-1 text-[11px] text-coral-600 dark:text-coral-300">
        <span>{t('orchestrationRuns.error')}</span>
        <Button
          variant="tertiary"
          size="xs"
          analyticsId="orchestration-progress-retry"
          onClick={refresh}>
          {t('orchestrationRuns.retry')}
        </Button>
      </div>
    );
  }
  if (runs.length === 0 || !summary) {
    return (
      <p
        data-testid="orchestration-progress-empty"
        className="px-1 pb-2 text-[11px] text-content-faint">
        {t('orchestrationRuns.threadEmpty')}
      </p>
    );
  }

  return (
    <section
      aria-label={t('orchestrationRuns.progressTitle')}
      data-testid="orchestration-progress"
      className="mb-2 rounded-xl border border-line-subtle bg-surface-muted/40 px-3 py-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <AgentRunPhaseStepper phase={summary.phase} compact />
        <div className="flex items-center gap-1">
          <span className="text-[11px] text-content-faint">
            {t('orchestrationRuns.progressCounts')
              .replace('{runs}', String(summary.runCount))
              .replace('{active}', String(summary.activeCount))}
          </span>
          <Button
            variant="tertiary"
            size="xs"
            analyticsId="orchestration-progress-toggle"
            aria-expanded={open}
            onClick={() => setOpen(v => !v)}
            className="px-2">
            {open ? t('orchestrationRuns.hideRuns') : t('orchestrationRuns.showRuns')}
          </Button>
          {onOpenAllRuns ? (
            <Button
              variant="tertiary"
              size="xs"
              analyticsId="orchestration-progress-all-runs"
              onClick={onOpenAllRuns}
              className="px-2">
              {t('orchestrationRuns.allRuns')}
            </Button>
          ) : null}
        </div>
      </div>
      {open ? (
        <ul className="mt-2 space-y-1.5" data-testid="orchestration-progress-runs">
          {runs.map(run => (
            <AgentRunRowItem key={run.runId} run={run} />
          ))}
        </ul>
      ) : null}
    </section>
  );
}

export default OrchestrationProgress;
