import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { PHASE_STEPS, phaseLabelKey, stepStates } from '../../lib/orchestration/agentRunPhases';
import type { AgentRunPhase } from '../../types/subagentRuns';

/**
 * The simplified execution story: Researching → Planning → Implementing →
 * Testing → Reviewing → Completed. `phase` marks the current step; earlier
 * steps read as done. Off-path outcomes (failed, cancelled, waiting on the
 * user) leave the steps untouched and show as a badge beside them, so the
 * stepper never claims progress it cannot vouch for.
 */
export default function AgentRunPhaseStepper({
  phase,
  compact = false,
}: {
  phase: AgentRunPhase;
  compact?: boolean;
}) {
  const { t } = useT();
  const states = stepStates(phase);
  const offPath = phase === 'failed' || phase === 'cancelled' || phase === 'awaiting_user';
  return (
    <div
      className="flex flex-wrap items-center gap-x-1 gap-y-1"
      data-testid="agent-run-stepper"
      data-phase={phase}>
      <ol className="flex flex-wrap items-center gap-x-1 gap-y-1">
        {PHASE_STEPS.map((step, i) => {
          const state = states[i];
          return (
            <li
              key={step}
              data-step={step}
              data-state={state}
              aria-current={state === 'current' ? 'step' : undefined}
              className="flex items-center gap-1">
              {i > 0 ? (
                <span aria-hidden className="text-[10px] text-content-faint">
                  →
                </span>
              ) : null}
              <span
                className={cn(
                  'rounded-full px-2 py-0.5 font-medium',
                  compact ? 'text-[10px]' : 'text-[11px]',
                  state === 'current' && 'bg-primary-500/15 text-primary-700 dark:text-primary-300',
                  state === 'done' && 'text-sage-700 dark:text-sage-300',
                  state === 'upcoming' && 'text-content-faint'
                )}>
                {state === 'done' ? '✓ ' : ''}
                {t(phaseLabelKey(step))}
              </span>
            </li>
          );
        })}
      </ol>
      {offPath ? (
        <span
          data-testid="agent-run-offpath"
          className={cn(
            'rounded-full px-2 py-0.5 text-[11px] font-medium',
            phase === 'awaiting_user'
              ? 'bg-amber-500/15 text-amber-700 dark:text-amber-300'
              : 'bg-coral-500/15 text-coral-700 dark:text-coral-300'
          )}>
          {t(phaseLabelKey(phase))}
        </span>
      ) : null}
    </div>
  );
}
