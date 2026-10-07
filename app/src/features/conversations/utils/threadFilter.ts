import type { Thread } from '../../../types/thread';

export const GENERAL_TAB_VALUE = 'general';
export const SUBCONSCIOUS_TAB_VALUE = 'subconscious';
export const TASKS_TAB_VALUE = 'tasks';
/** Core label stamped on a thread when its mode is `debug`. */
const DEBUG_MODE_LABEL = 'mode:debug';
const LEGACY_SUBCONSCIOUS_LABELS = ['from_reflection', 'subconscious_tick'];
const LEGACY_TASK_LABELS = ['agent-task', 'worker'];
/** Labels that identify meeting transcript threads (now folded into Tasks). */
const MEETINGS_LABELS = ['meetings', 'Meetings'];

function hasAnyLabel(thread: Thread, labels: readonly string[]): boolean {
  return Boolean(thread.labels?.some(label => labels.includes(label)));
}

/**
 * True for a thread whose mode is `debug` (the top-bar Debug switch). The
 * persisted `mode` is authoritative; the core's label is only a fallback for a
 * payload that predates the field, so an optimistic switch back to Chat is not
 * overruled by a label that has not been refreshed yet.
 */
export function isDebugThread(thread: Thread): boolean {
  if (thread.mode) return thread.mode === 'debug';
  return hasAnyLabel(thread, [DEBUG_MODE_LABEL]);
}

function isSubconsciousThread(thread: Thread): boolean {
  return hasAnyLabel(thread, [SUBCONSCIOUS_TAB_VALUE, ...LEGACY_SUBCONSCIOUS_LABELS]);
}

function isTaskThread(thread: Thread): boolean {
  return Boolean(
    thread.parentThreadId ||
    hasAnyLabel(thread, [TASKS_TAB_VALUE, ...LEGACY_TASK_LABELS, ...MEETINGS_LABELS])
  );
}

/**
 * Pure, side-effect-free thread filter shared between
 * `Conversations.tsx` (which renders the sidebar list) and the test
 * suite.
 *
 * Rules:
 *   - Tasks includes task-board threads, legacy worker/sub-agent threads,
 *     and meeting transcript threads.
 *   - Subconscious includes new and legacy reflection/tick-generated threads.
 *   - Debug-mode threads are ordinary threads here and follow the rules below
 *     (they show up in General unless labelled otherwise).
 *   - General is the fallback bucket for everything else.
 */
export function isThreadVisibleInTab(thread: Thread, selectedLabel: string): boolean {
  const isSubconscious = isSubconsciousThread(thread);
  const isTask = isTaskThread(thread);
  if (selectedLabel === SUBCONSCIOUS_TAB_VALUE) return isSubconscious;
  if (selectedLabel === TASKS_TAB_VALUE) return isTask;
  if (selectedLabel === GENERAL_TAB_VALUE) {
    return !isSubconscious && !isTask;
  }
  return Boolean(thread.labels?.includes(selectedLabel));
}
