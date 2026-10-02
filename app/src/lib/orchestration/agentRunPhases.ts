import type {
  AgentRunPhase,
  SubagentRunRow,
  SubagentRunThreadSummary,
} from '../../types/subagentRuns';

/**
 * The simplified execution story shown for Orchestration mode, in order. A run
 * or thread sits at one of these; `awaiting_user`, `failed` and `cancelled`
 * are off-path outcomes rendered beside the stepper rather than as steps.
 */
export const PHASE_STEPS = [
  'researching',
  'planning',
  'implementing',
  'testing',
  'reviewing',
  'completed',
] as const;

export type PhaseStep = (typeof PHASE_STEPS)[number];

const KNOWN_PHASES: readonly AgentRunPhase[] = [
  ...PHASE_STEPS,
  'awaiting_user',
  'failed',
  'cancelled',
];

/** Coerce a wire phase to a known one (a newer core may add phases). */
export function normalizePhase(value: unknown): AgentRunPhase {
  return typeof value === 'string' && (KNOWN_PHASES as readonly string[]).includes(value)
    ? (value as AgentRunPhase)
    : 'implementing';
}

/** i18n key for a phase's label. */
export function phaseLabelKey(phase: AgentRunPhase): string {
  return `orchestrationRuns.phase.${phase}`;
}

/** Index of `phase` on the stepper, or -1 when it is an off-path outcome. */
export function phaseStepIndex(phase: AgentRunPhase): number {
  return (PHASE_STEPS as readonly string[]).indexOf(phase);
}

export type StepState = 'done' | 'current' | 'upcoming';

/**
 * Per-step state for the stepper given the thread's (or run's) phase.
 * Completed fills every step; an off-path outcome (failed, cancelled, awaiting
 * user) leaves the stepper untouched so it never claims progress it cannot
 * vouch for.
 */
export function stepStates(phase: AgentRunPhase): StepState[] {
  if (phase === 'completed') return PHASE_STEPS.map(() => 'done');
  const idx = phaseStepIndex(phase);
  return PHASE_STEPS.map((_, i) =>
    idx < 0 ? 'upcoming' : i < idx ? 'done' : i === idx ? 'current' : 'upcoming'
  );
}

export type RunStatusFilter = 'all' | 'running' | 'completed' | 'failed';

/** Raw ledger statuses each filter chip stands for. */
export function matchesStatusFilter(run: SubagentRunRow, filter: RunStatusFilter): boolean {
  switch (filter) {
    case 'all':
      return true;
    case 'running':
      return isLiveRun(run);
    case 'completed':
      return run.phase === 'completed';
    case 'failed':
      return run.phase === 'failed' || run.phase === 'cancelled';
  }
}

/** A run still doing work (or parked on a question) rather than settled. */
export function isLiveRun(run: Pick<SubagentRunRow, 'phase'>): boolean {
  if (run.phase === 'awaiting_user') return true;
  return run.phase !== 'completed' && phaseStepIndex(run.phase) >= 0;
}

export function threadHasLiveRuns(thread: Pick<SubagentRunThreadSummary, 'activeCount'>): boolean {
  return thread.activeCount > 0;
}

/** "41s", "2m 05s", "1h 03m" — compact elapsed time. */
export function formatElapsed(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return '';
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, '0')}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, '0')}m`;
}
