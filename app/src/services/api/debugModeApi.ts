import { invoke } from '@tauri-apps/api/core';
import debug from 'debug';

import { isTauri } from '../../utils/tauriCommands/common';
import { callCoreRpc } from '../coreRpcClient';

// ---------------------------------------------------------------------------
// Debug Mode RPC client (`neppy.debug_mode_*`).
//
// Wire shapes mirror `src/neppy/agent/debug_mode/types.rs`. Handlers return the
// bare JSON value, but the core may also hand back the CLI-compatible
// `{ result, logs }` envelope, so every call goes through `unwrapValue`.
//
// Privacy: only method names and ids are logged. Commit messages, task
// requests, file paths and diff text never reach a log line.
// ---------------------------------------------------------------------------

const log = debug('neppy:debugModeApi');

export interface DirtyFiles {
  modified: string[];
  added: string[];
  deleted: string[];
  untracked: string[];
}

export interface DebugStatus {
  project_root: string;
  /** `null` when HEAD is detached. */
  branch: string | null;
  /** `null` on an unborn branch. */
  head: string | null;
  dirty: DirtyFiles;
  task_active: boolean;
  active_task_id: string | null;
}

export type DebugTaskStatus =
  | 'planning'
  | 'editing'
  | 'validating'
  | 'pass'
  | 'partial'
  | 'failed'
  | 'rolled_back';

export interface DebugValidationRecord {
  check_id: string | null;
  command: string[];
  exit_code: number | null;
  passed: boolean;
  timed_out: boolean;
  duration_ms: number;
  at: string;
  output_tail: string;
}

export interface DebugTask {
  id: string;
  request: string;
  created_at: string;
  updated_at: string;
  status: DebugTaskStatus;
  files_changed: string[];
  validation: DebugValidationRecord[];
  summary: string | null;
  checkpoint_id: string | null;
  branch: string | null;
  commit: string | null;
}

export interface DebugCheckpoint {
  id: string;
  description: string;
  task_id: string | null;
  project_root: string;
  created_at: string;
  head: string;
  branch: string | null;
  snapshot_sha: string;
  index_tree: string;
  dirty_files: string[];
  untracked_files: string[];
}

export interface DebugDiffFile {
  path: string;
  /** `null` for binary files. */
  added: number | null;
  deleted: number | null;
}

export interface DebugDiff {
  /** "HEAD" or the checkpoint id the working tree was compared against. */
  base: string;
  text: string;
  truncated: boolean;
  summary: { modified: number; created: number; deleted: number };
  files: DebugDiffFile[];
  untracked: string[];
}

export interface RollbackResult {
  checkpoint_id: string;
  /** Roll forward again with this checkpoint. */
  pre_rollback_checkpoint_id: string;
  restored: string[];
  removed: string[];
  head_moved: boolean;
}

export interface DebugCommitResult {
  task_id: string;
  commit: string;
  branch: string | null;
  files: string[];
}

export interface AuditEntry {
  ts: string;
  op: string;
  target: string;
  outcome: string;
}

/**
 * Persisted Debug Mode permissions (`neppy.debug_mode_settings_*`). Mirrors the
 * core's `DebugModeSettings`. Dangerous capabilities default to off; the core
 * validates every patch (e.g. `max_repair_iterations` 1..20) and rejects bad
 * input with an RPC error.
 */
export interface DebugModeSettings {
  enabled: boolean;
  /** `null` = auto-detect the app's own source repository. */
  project_root: string | null;
  auto_checkpoint: boolean;
  auto_repair: boolean;
  /** 1..20. */
  max_repair_iterations: number;
  run_tests_after_changes: boolean;
  run_build_after_changes: boolean;
  allow_dependency_install: boolean;
  allow_external_filesystem: boolean;
  /** Absolute paths, only honoured while `allow_external_filesystem` is on. */
  external_paths: string[];
  allow_system_commands: boolean;
  allow_git_commit: boolean;
  allow_git_push: boolean;
  dangerous_commands_require_confirmation: boolean;
}

const unwrapValue = <T>(raw: unknown): T => {
  if (raw && typeof raw === 'object' && !Array.isArray(raw) && 'result' in raw) {
    return (raw as { result: T }).result;
  }
  return raw as T;
};

async function call<T>(fn: string, params: Record<string, unknown> = {}): Promise<T> {
  log('rpc %s', fn);
  const raw = await callCoreRpc<unknown>({ method: `neppy.debug_mode_${fn}`, params });
  return unwrapValue<T>(raw);
}

/** Project root, branch, HEAD, dirty files. Rejects when no source repo resolves. */
export const getDebugStatus = (): Promise<DebugStatus> => call<DebugStatus>('status');

/** Newest first. */
export const listDebugTasks = (limit?: number): Promise<DebugTask[]> =>
  call<DebugTask[]>('task_list', limit === undefined ? {} : { limit });

export const getDebugTask = (taskId: string): Promise<DebugTask> =>
  call<DebugTask>('task_get', { task_id: taskId });

/** Working tree vs HEAD, or vs `checkpointId` when given. */
export const getDebugDiff = (checkpointId?: string): Promise<DebugDiff> =>
  call<DebugDiff>('diff', checkpointId ? { checkpoint_id: checkpointId } : {});

/** Newest first. */
export const listDebugCheckpoints = (limit?: number): Promise<DebugCheckpoint[]> =>
  call<DebugCheckpoint[]>('checkpoint_list', limit === undefined ? {} : { limit });

/**
 * Restore the working tree from a checkpoint (sends `confirm: true`; callers
 * must have asked the user first). A pre-rollback checkpoint is saved first, so
 * this is reversible.
 */
export const rollbackDebug = (checkpointId: string): Promise<RollbackResult> =>
  call<RollbackResult>('rollback', { checkpoint_id: checkpointId, confirm: true });

/**
 * Commit exactly the files the task changed (sends `confirm: true`; callers must
 * have asked the user first). Never pushes. The core refuses when unrelated
 * changes are already staged.
 */
export const commitDebugTask = (taskId: string, message: string): Promise<DebugCommitResult> =>
  call<DebugCommitResult>('commit', { task_id: taskId, message, confirm: true });

export const tailDebugAudit = (limit?: number): Promise<AuditEntry[]> =>
  call<AuditEntry[]>('audit_tail', limit === undefined ? {} : { limit });

export const getDebugSettings = (): Promise<DebugModeSettings> =>
  call<DebugModeSettings>('settings_get');

/**
 * Apply a partial update and return the full, validated settings. Send only the
 * fields that changed. Rejects with the core's validation message on bad input.
 */
export const updateDebugSettings = (
  patch: Partial<DebugModeSettings>
): Promise<DebugModeSettings> => call<DebugModeSettings>('settings_update', { patch });

// ---------------------------------------------------------------------------
// Local install: build the source into a Neppy.app and swap it into
// /Applications without GitHub or the updater. Wire shapes mirror
// `src/neppy/agent/debug_mode/local_install.rs`.
// ---------------------------------------------------------------------------

export type LocalInstallPhase = 'idle' | 'building' | 'ready' | 'failed' | 'installing';

export interface LocalInstallStatus {
  phase: LocalInstallPhase;
  /** Version baked into the new bundle, when known. */
  version: string | null;
  started_at: string | null;
  finished_at: string | null;
  bundle_path: string | null;
  /** Build failure tail; empty unless `phase` is `failed`. */
  error: string;
  /** Tail of the build output (absent when idle). */
  log_tail?: string;
}

/** The installer helper's verdict from its last run. */
export interface LocalInstallResult {
  status: 'installed' | 'restored' | 'failed';
  version: string;
  backup: string;
  ts: string;
  reason: string;
  /** True once acknowledged: the one-time notice has been shown. */
  seen: boolean;
}

/**
 * Start the background build. The core refuses while a build runs and when the
 * active debug task changed critical files with no passed candidate.
 */
export const startLocalInstallBuild = (): Promise<LocalInstallStatus> =>
  call<LocalInstallStatus>('install_local_build');

export const getLocalInstallStatus = (): Promise<LocalInstallStatus> =>
  call<LocalInstallStatus>('install_local_status');

/**
 * Hand the built bundle to the detached installer helper (sends `confirm:
 * true`; callers must have asked the user first). The app must quit afterwards:
 * see {@link quitApp}.
 */
export const applyLocalInstall = (): Promise<LocalInstallStatus> =>
  call<LocalInstallStatus>('install_local_apply', { confirm: true });

/** The installer's last verdict (`null` when none). `acknowledge` marks it seen. */
export const getLocalInstallResult = (acknowledge = false): Promise<LocalInstallResult | null> =>
  call<LocalInstallResult | null>('install_local_result', acknowledge ? { acknowledge: true } : {});

/** Quit the desktop app (the Tauri `app_quit` command). No-op outside Tauri. */
export async function quitApp(): Promise<void> {
  if (!isTauri()) {
    log('quitApp skipped: not running in Tauri');
    return;
  }
  log('quitApp: invoking app_quit');
  await invoke<void>('app_quit');
}

// ---------------------------------------------------------------------------
// Publish release: runs `scripts/release-neppy.sh <version>` on this machine
// (builds and signs, pushes, tags, creates the GitHub release). UI/RPC only,
// never an agent tool. Wire shapes mirror
// `src/neppy/agent/debug_mode/release.rs`.
// ---------------------------------------------------------------------------

/** Stable blocker codes from `release_preflight`; translated by the UI. */
export type ReleaseBlocker =
  | 'not_release_branch'
  | 'dirty'
  | 'behind'
  | 'no_signing_key'
  | 'gh_not_ready'
  | 'release_running'
  | 'nothing_to_release';

export interface ReleasePreflight {
  project_root: string;
  /** Current branch. */
  branch: string;
  /** Branch releases are cut from (`NEPPY_RELEASE_BRANCH` or `main`). */
  release_branch: string;
  /** `git status --porcelain` is empty. */
  clean: boolean;
  /** `origin/<release_branch>` is not an ancestor of HEAD. */
  behind: boolean;
  /** Commits in HEAD that are not in `origin/<release_branch>`. */
  ahead_commits: number;
  /** `version` from `app/package.json`. */
  current_version: string;
  /** `current_version` with the patch component incremented. */
  suggested_version: string;
  signing_key_present: boolean;
  gh_ready: boolean;
  fetch_error: string | null;
  blockers: ReleaseBlocker[];
  /** Latest `v*` tag, best effort. */
  last_tag: string | null;
}

export type ReleasePhase = 'idle' | 'running' | 'succeeded' | 'failed';

export interface ReleaseRecord {
  phase: ReleasePhase;
  version: string | null;
  started_at: string | null;
  finished_at: string | null;
  exit_code: number | null;
  /** Last ~6 KB of the release log, secret-like lines masked. */
  log_tail: string;
  /** `v<version>` on success. */
  tag: string | null;
  release_url: string | null;
  /** Short failure reason; empty otherwise. */
  error: string;
}

export const getReleasePreflight = (): Promise<ReleasePreflight> =>
  call<ReleasePreflight>('release_preflight');

/**
 * Start the release script. The caller must have asked the user to confirm the
 * exact version first; the core re-validates the version and re-runs preflight.
 */
export const startRelease = (version: string): Promise<ReleaseRecord> =>
  call<ReleaseRecord>('release_start', { version });

export const getReleaseStatus = (): Promise<ReleaseRecord> => call<ReleaseRecord>('release_status');
