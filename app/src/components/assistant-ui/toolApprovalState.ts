import { useContext, useSyncExternalStore } from 'react';
import { ReactReduxContext } from 'react-redux';

import type { PendingApproval } from '../../store/chatRuntimeSlice';

/**
 * How a still-running tool row relates to the thread's ApprovalGate queue.
 *
 * - `waiting`: this call is the one parked for the user's decision.
 * - `queued`: another call in the thread is parked, so this one sits behind it.
 * - `none`: no pending approval touches this row.
 *
 * The core's `approval_request` event carries the tool name and redacted args
 * but no tool-call id, so the match is by tool name alone (the redacted args
 * cannot be compared reliably against the row's own).
 */
export type ToolApprovalRowState = 'none' | 'waiting' | 'queued';

export function toolApprovalRowState(
  approvals: readonly PendingApproval[] | undefined,
  toolName: string,
  isRunning: boolean
): ToolApprovalRowState {
  if (!isRunning || !approvals || approvals.length === 0) return 'none';
  return approvals.some(a => a.toolName === toolName) ? 'waiting' : 'queued';
}

type RootStateLike = {
  thread?: { selectedThreadId?: string | null };
  chatRuntime?: { pendingApprovalByThread?: Record<string, PendingApproval[]> };
};

/**
 * Tool-row state against the selected thread's pending approvals. Safe outside
 * a Redux Provider (renders `none`), so the presentational tool kit stays
 * usable in isolation.
 */
export function useToolApprovalRowState(
  toolName: string,
  isRunning: boolean
): ToolApprovalRowState {
  const store = useContext(ReactReduxContext)?.store;
  const getSnapshot = (): ToolApprovalRowState => {
    if (!store || !isRunning) return 'none';
    const state = store.getState() as RootStateLike;
    const threadId = state.thread?.selectedThreadId;
    if (!threadId) return 'none';
    return toolApprovalRowState(
      state.chatRuntime?.pendingApprovalByThread?.[threadId],
      toolName,
      isRunning
    );
  };
  return useSyncExternalStore(
    onChange => (store ? store.subscribe(onChange) : () => {}),
    getSnapshot,
    () => 'none'
  );
}

/** True while the selected thread has any call parked on the ApprovalGate. */
export function useThreadHasPendingApproval(): boolean {
  const store = useContext(ReactReduxContext)?.store;
  const getSnapshot = (): boolean => {
    if (!store) return false;
    const state = store.getState() as RootStateLike;
    const threadId = state.thread?.selectedThreadId;
    if (!threadId) return false;
    return (state.chatRuntime?.pendingApprovalByThread?.[threadId]?.length ?? 0) > 0;
  };
  return useSyncExternalStore(
    onChange => (store ? store.subscribe(onChange) : () => {}),
    getSnapshot,
    () => false
  );
}
