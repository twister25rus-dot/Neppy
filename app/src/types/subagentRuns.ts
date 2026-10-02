/**
 * Wire types for `openhuman.subagent_runs_history` (camelCase): a read-only
 * projection of the run ledger across all threads. See the core's
 * Chat / Orchestration modes contract.
 */

/** Execution phase of a run or thread. Live runs report their role, settled
 * runs report their outcome. */
export type AgentRunPhase =
  | 'researching'
  | 'planning'
  | 'implementing'
  | 'testing'
  | 'reviewing'
  | 'awaiting_user'
  | 'completed'
  | 'failed'
  | 'cancelled';

export type AgentRunKind =
  | 'subagent'
  | 'worker_thread'
  | 'background_agent'
  | 'team_member'
  | 'workflow_child';

export interface SubagentRunRow {
  runId: string;
  threadId: string | null;
  threadTitle: string | null;
  threadMode: 'chat' | 'orchestration' | null;
  agentId: string;
  kind: AgentRunKind | string;
  /** Raw ledger status (running, completed, failed, interrupted, ...). */
  status: string;
  phase: AgentRunPhase;
  summary: string | null;
  error: string | null;
  startedAt: string | null;
  updatedAt: string | null;
  completedAt: string | null;
  elapsedMs: number | null;
  model: string | null;
  toolCount: number | null;
  costUsd: number | null;
}

export interface SubagentRunThreadSummary {
  threadId: string;
  threadTitle: string | null;
  threadMode: 'chat' | 'orchestration' | null;
  runCount: number;
  activeCount: number;
  failedCount: number;
  phase: AgentRunPhase;
  lastUpdatedAt: string | null;
}

export interface SubagentRunsHistory {
  count: number;
  runs: SubagentRunRow[];
  threads: SubagentRunThreadSummary[];
}

export interface SubagentRunsHistoryParams {
  limit?: number;
  threadId?: string;
  mode?: 'chat' | 'orchestration';
  status?: string;
  onlyThreaded?: boolean;
}
