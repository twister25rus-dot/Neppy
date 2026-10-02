/**
 * AgentRunsPanel — cross-conversation execution history for the supervisor's
 * helper agents (Brain → Orchestration → Agent runs).
 *
 * A read-only projection of the run ledger (`subagent_runs_history`): runs are
 * grouped under the conversation that spawned them, each group shows the
 * simplified Researching → … → Completed progress, and every run drills down to
 * its outcome. Filterable by status and by whether to include chat-mode threads
 * (whose blocking delegate runs are otherwise out of the supervisor's story).
 */
import { useMemo, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { matchesStatusFilter, type RunStatusFilter } from '../../lib/orchestration/agentRunPhases';
import { useSubagentRuns } from '../../lib/orchestration/useSubagentRuns';
import type { SubagentRunRow, SubagentRunThreadSummary } from '../../types/subagentRuns';
import { chatThreadPath } from '../../utils/chatRoutes';
import ChipTabs from '../layout/ChipTabs';
import Button from '../ui/Button';
import AgentRunPhaseStepper from './AgentRunPhaseStepper';
import AgentRunRowItem from './AgentRunRow';

type Scope = 'orchestration' | 'all';

const STATUS_FILTERS: readonly RunStatusFilter[] = ['all', 'running', 'completed', 'failed'];

const NO_THREAD = '__none__';

function groupRuns(
  runs: SubagentRunRow[],
  threads: SubagentRunThreadSummary[]
): Array<{ key: string; summary: SubagentRunThreadSummary | null; runs: SubagentRunRow[] }> {
  const byThread = new Map<string, SubagentRunRow[]>();
  for (const run of runs) {
    const key = run.threadId ?? NO_THREAD;
    const bucket = byThread.get(key);
    if (bucket) bucket.push(run);
    else byThread.set(key, [run]);
  }
  const summaryById = new Map(threads.map(th => [th.threadId, th] as const));
  // Keep the core's newest-first order: groups appear in order of first run.
  return [...byThread.entries()].map(([key, groupRuns]) => ({
    key,
    summary: summaryById.get(key) ?? null,
    runs: groupRuns,
  }));
}

export default function AgentRunsPanel() {
  const { t } = useT();
  const navigate = useNavigate();
  const [scope, setScope] = useState<Scope>('orchestration');
  const [status, setStatus] = useState<RunStatusFilter>('all');

  const { data, loading, error, refresh } = useSubagentRuns({
    limit: 100,
    onlyThreaded: true,
    ...(scope === 'orchestration' ? { mode: 'orchestration' as const } : {}),
  });

  const visibleRuns = useMemo(
    () => (data?.runs ?? []).filter(run => matchesStatusFilter(run, status)),
    [data, status]
  );
  const groups = useMemo(
    () => groupRuns(visibleRuns, data?.threads ?? []),
    [visibleRuns, data?.threads]
  );

  return (
    <div className="space-y-3" data-testid="agent-runs-panel">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <ChipTabs<RunStatusFilter>
          as="tab"
          ariaLabel={t('orchestrationRuns.filter.status')}
          testIdPrefix="agent-runs-status"
          className="inline-flex flex-wrap items-center gap-1.5"
          compact
          items={STATUS_FILTERS.map(id => ({ id, label: t(`orchestrationRuns.filter.${id}`) }))}
          value={status}
          onChange={setStatus}
        />
        <div className="flex items-center gap-2">
          <ChipTabs<Scope>
            as="tab"
            ariaLabel={t('orchestrationRuns.filter.scope')}
            testIdPrefix="agent-runs-scope"
            className="inline-flex flex-wrap items-center gap-1.5"
            compact
            items={[
              { id: 'orchestration', label: t('orchestrationRuns.scope.orchestration') },
              { id: 'all', label: t('orchestrationRuns.scope.all') },
            ]}
            value={scope}
            onChange={setScope}
          />
          <Button
            variant="tertiary"
            size="xs"
            analyticsId="agent-runs-refresh"
            onClick={refresh}
            className="px-2">
            {t('orchestrationRuns.refresh')}
          </Button>
        </div>
      </div>

      {loading ? (
        <p className="text-sm text-content-faint" role="status" data-testid="agent-runs-loading">
          {t('orchestrationRuns.loading')}
        </p>
      ) : error && !data ? (
        <div
          role="alert"
          data-testid="agent-runs-error"
          className="flex items-center gap-3 rounded-lg border border-coral-200 bg-coral-50 px-3 py-2 text-sm text-coral-700 dark:border-coral-700/50 dark:bg-coral-950/30 dark:text-coral-300">
          <span>{t('orchestrationRuns.error')}</span>
          <Button variant="secondary" size="xs" analyticsId="agent-runs-retry" onClick={refresh}>
            {t('orchestrationRuns.retry')}
          </Button>
        </div>
      ) : groups.length === 0 ? (
        <div
          data-testid="agent-runs-empty"
          className="rounded-lg border border-dashed border-line px-4 py-8 text-center text-sm text-content-muted">
          {status === 'all' && scope === 'orchestration'
            ? t('orchestrationRuns.empty')
            : t('orchestrationRuns.emptyFiltered')}
        </div>
      ) : (
        <>
          {error ? (
            <p role="alert" className="text-xs text-coral" data-testid="agent-runs-stale">
              {t('orchestrationRuns.staleError')}
            </p>
          ) : null}
          <ul className="space-y-3">
            {groups.map(group => (
              <li
                key={group.key}
                data-testid="agent-runs-thread"
                className="space-y-2 rounded-xl border border-line bg-surface-muted/40 p-3">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-semibold text-content">
                      {group.summary?.threadTitle ??
                        group.runs[0]?.threadTitle ??
                        t('orchestrationRuns.untitledThread')}
                    </p>
                    {group.summary ? (
                      <p className="text-[11px] text-content-faint">
                        {t('orchestrationRuns.threadCounts')
                          .replace('{runs}', String(group.summary.runCount))
                          .replace('{active}', String(group.summary.activeCount))
                          .replace('{failed}', String(group.summary.failedCount))}
                      </p>
                    ) : null}
                  </div>
                  {group.key !== NO_THREAD ? (
                    <Button
                      variant="tertiary"
                      size="xs"
                      analyticsId="agent-runs-open-thread"
                      onClick={() => navigate(chatThreadPath(group.key))}
                      className="px-2">
                      {t('orchestrationRuns.openThread')}
                    </Button>
                  ) : null}
                </div>
                {group.summary ? (
                  <AgentRunPhaseStepper phase={group.summary.phase} compact />
                ) : null}
                <ul className="space-y-1.5">
                  {group.runs.map(run => (
                    <AgentRunRowItem key={run.runId} run={run} />
                  ))}
                </ul>
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
