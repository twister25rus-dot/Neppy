import type { DebugStatus, DebugTask, DebugTaskStatus } from '../../services/api/debugModeApi';

/** Commit subject prefix; a code convention, deliberately not translated. */
export const COMMIT_PREFIX = 'feat(debug): ';
const COMMIT_REQUEST_CHARS = 60;

/** Last path segment of the repository root, for either separator style. */
export function repoBasename(projectRoot: string): string {
  const parts = projectRoot.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? projectRoot;
}

export function shortSha(sha: string | null | undefined): string {
  return sha ? sha.slice(0, 7) : '';
}

/** Number of changed paths in the working tree (every category, summed). */
export function dirtyCount(status: DebugStatus): number {
  const d = status.dirty;
  return d.modified.length + d.added.length + d.deleted.length + d.untracked.length;
}

/** `feat(debug): <first 60 chars of the request>`, whitespace collapsed. */
export function defaultCommitMessage(request: string): string {
  const oneLine = request.replace(/\s+/g, ' ').trim();
  return `${COMMIT_PREFIX}${Array.from(oneLine).slice(0, COMMIT_REQUEST_CHARS).join('')}`.trimEnd();
}

export function isTaskActive(status: DebugTaskStatus): boolean {
  return status === 'planning' || status === 'editing' || status === 'validating';
}

export function canCommitTask(task: DebugTask): boolean {
  return (
    !isTaskActive(task.status) &&
    task.status !== 'rolled_back' &&
    !task.commit &&
    task.files_changed.length > 0
  );
}

export function canRollbackTask(task: DebugTask): boolean {
  return !isTaskActive(task.status) && task.status !== 'rolled_back' && Boolean(task.checkpoint_id);
}

export function errorText(error: unknown): string {
  if (typeof error === 'string' && error.trim()) return error;
  if (error instanceof Error && error.message.trim()) return error.message;
  if (error && typeof error === 'object' && 'message' in error) {
    const m = (error as { message?: unknown }).message;
    if (typeof m === 'string' && m.trim()) return m;
  }
  return '';
}
