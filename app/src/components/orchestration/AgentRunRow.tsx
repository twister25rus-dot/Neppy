import { useState } from 'react';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { formatElapsed, phaseLabelKey } from '../../lib/orchestration/agentRunPhases';
import type { SubagentRunRow } from '../../types/subagentRuns';

function toneFor(phase: SubagentRunRow['phase']): string {
  if (phase === 'completed') return 'bg-sage-500/15 text-sage-700 dark:text-sage-300';
  if (phase === 'failed' || phase === 'cancelled')
    return 'bg-coral-500/15 text-coral-700 dark:text-coral-300';
  if (phase === 'awaiting_user') return 'bg-amber-500/15 text-amber-700 dark:text-amber-300';
  return 'bg-primary-500/15 text-primary-700 dark:text-primary-300';
}

/**
 * One run from the ledger: agent, phase, elapsed time, with an inline drill
 * down to its outcome summary / error and run facts. The summary and error come
 * bounded from the core, so they are safe to render as plain text.
 */
export default function AgentRunRow({ run }: { run: SubagentRunRow }) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const elapsed = formatElapsed(run.elapsedMs);
  const hasDetail = Boolean(run.summary || run.error || run.model || run.toolCount != null);
  return (
    <li
      data-testid="agent-run-row"
      data-run-id={run.runId}
      data-phase={run.phase}
      className="rounded-lg border border-line-subtle bg-surface px-2.5 py-1.5 text-[12px]">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-medium text-content">{run.agentId}</span>
        <span
          className={cn('rounded-full px-2 py-0.5 text-[11px] font-medium', toneFor(run.phase))}>
          {t(phaseLabelKey(run.phase))}
        </span>
        {elapsed ? <span className="text-content-faint">{elapsed}</span> : null}
        {hasDetail ? (
          <button
            type="button"
            aria-expanded={open}
            data-analytics-id="agent-run-toggle-details"
            onClick={() => setOpen(v => !v)}
            className="ml-auto text-[11px] font-medium text-primary-600 hover:underline dark:text-primary-300">
            {open ? t('orchestrationRuns.hideDetails') : t('orchestrationRuns.showDetails')}
          </button>
        ) : null}
      </div>
      {open ? (
        <div className="mt-1.5 space-y-1 text-content-secondary" data-testid="agent-run-details">
          {run.summary ? (
            <p className="whitespace-pre-wrap wrap-break-word">{run.summary}</p>
          ) : null}
          {run.error ? (
            <p className="whitespace-pre-wrap wrap-break-word text-coral-600 dark:text-coral-300">
              {run.error}
            </p>
          ) : null}
          <p className="text-[11px] text-content-faint">
            {[
              run.model,
              run.toolCount != null
                ? t('orchestrationRuns.toolCount').replace('{count}', String(run.toolCount))
                : null,
              run.costUsd != null && run.costUsd > 0 ? `$${run.costUsd.toFixed(4)}` : null,
            ]
              .filter(Boolean)
              .join(' · ')}
          </p>
        </div>
      ) : null}
    </li>
  );
}
