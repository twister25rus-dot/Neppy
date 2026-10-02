import { describe, expect, it } from 'vitest';

import type { SubagentRunRow } from '../../../types/subagentRuns';
import {
  formatElapsed,
  isLiveRun,
  matchesStatusFilter,
  normalizePhase,
  PHASE_STEPS,
  phaseLabelKey,
  stepStates,
} from '../agentRunPhases';

const run = (phase: SubagentRunRow['phase']): SubagentRunRow => ({
  runId: 'r',
  threadId: 't',
  threadTitle: null,
  threadMode: 'orchestration',
  agentId: 'researcher',
  kind: 'subagent',
  status: 'running',
  phase,
  summary: null,
  error: null,
  startedAt: null,
  updatedAt: null,
  completedAt: null,
  elapsedMs: null,
  model: null,
  toolCount: null,
  costUsd: null,
});

describe('agentRunPhases', () => {
  it('orders the simplified story as the user reads it', () => {
    expect(PHASE_STEPS).toEqual([
      'researching',
      'planning',
      'implementing',
      'testing',
      'reviewing',
      'completed',
    ]);
  });

  it('marks earlier steps done, the phase current, later steps upcoming', () => {
    expect(stepStates('implementing')).toEqual([
      'done',
      'done',
      'current',
      'upcoming',
      'upcoming',
      'upcoming',
    ]);
    expect(stepStates('researching')[0]).toBe('current');
  });

  it('fills every step when completed', () => {
    expect(stepStates('completed').every(s => s === 'done')).toBe(true);
  });

  it('never claims progress for off-path outcomes', () => {
    for (const phase of ['failed', 'cancelled', 'awaiting_user'] as const) {
      expect(stepStates(phase).every(s => s === 'upcoming')).toBe(true);
    }
  });

  it('degrades an unknown wire phase instead of crashing', () => {
    expect(normalizePhase('reviewing')).toBe('reviewing');
    expect(normalizePhase('brand-new-phase')).toBe('implementing');
    expect(normalizePhase(undefined)).toBe('implementing');
  });

  it('classifies live vs settled runs (awaiting_user is still live)', () => {
    expect(isLiveRun(run('testing'))).toBe(true);
    expect(isLiveRun(run('awaiting_user'))).toBe(true);
    expect(isLiveRun(run('completed'))).toBe(false);
    expect(isLiveRun(run('failed'))).toBe(false);
    expect(isLiveRun(run('cancelled'))).toBe(false);
  });

  it('maps status filter chips onto phases', () => {
    expect(matchesStatusFilter(run('planning'), 'running')).toBe(true);
    expect(matchesStatusFilter(run('completed'), 'running')).toBe(false);
    expect(matchesStatusFilter(run('completed'), 'completed')).toBe(true);
    expect(matchesStatusFilter(run('failed'), 'failed')).toBe(true);
    expect(matchesStatusFilter(run('cancelled'), 'failed')).toBe(true);
    expect(matchesStatusFilter(run('failed'), 'all')).toBe(true);
  });

  it('formats elapsed time compactly and tolerates missing values', () => {
    expect(formatElapsed(41_200)).toBe('41s');
    expect(formatElapsed(125_000)).toBe('2m 05s');
    expect(formatElapsed(3_780_000)).toBe('1h 03m');
    expect(formatElapsed(null)).toBe('');
    expect(formatElapsed(-5)).toBe('');
  });

  it('builds the i18n key per phase', () => {
    expect(phaseLabelKey('awaiting_user')).toBe('orchestrationRuns.phase.awaiting_user');
  });
});
