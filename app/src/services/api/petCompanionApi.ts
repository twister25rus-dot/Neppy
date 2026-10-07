import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

// ---------------------------------------------------------------------------
// Pet desktop companion RPC client (`neppy.pet_companion_*`).
//
// The wire shapes mirror the §2.9 contract of the Pet companion plan. Handlers
// return the bare JSON value, but the core may also hand back the CLI-compatible
// `{ result, logs }` envelope, so every call goes through `unwrapValue`.
//
// Privacy: only method names are logged here. Window titles, excerpts,
// suggestion bodies and hand-off prompts never reach a log line.
// ---------------------------------------------------------------------------

const log = debug('neppy:petCompanionApi');

/** 0 Observe, 1 Suggest, 2 Assist, 3 Trusted. */
export type CompanionLevel = 0 | 1 | 2 | 3;
export type CompanionChattiness = 'quiet' | 'normal' | 'eager';

/** Observation sources the user can switch on or off individually. */
export type CompanionSourceKey = 'app_window' | 'selection' | 'clipboard' | 'screen_capture';
export type CompanionSources = Record<CompanionSourceKey, boolean>;

/** Action categories a user may configure (low and medium risk). */
export type EditableCategory =
  | 'explain'
  | 'draft_text'
  | 'format_text'
  | 'save_note'
  | 'prepare_command'
  | 'open_chat'
  | 'handoff_task';

/** High-risk categories: fixed, the pet always asks before any of them. */
export type HighRiskCategory =
  | 'send_message'
  | 'delete'
  | 'purchase'
  | 'publish'
  | 'system_settings'
  | 'install'
  | 'privileged_command'
  | 'share_personal_info'
  | 'irreversible';

export type CompanionCategory = EditableCategory | HighRiskCategory;

export const EDITABLE_CATEGORIES: readonly EditableCategory[] = [
  'explain',
  'draft_text',
  'format_text',
  'save_note',
  'prepare_command',
  'open_chat',
  'handoff_task',
];

export const HIGH_RISK_CATEGORIES: readonly HighRiskCategory[] = [
  'send_message',
  'delete',
  'purchase',
  'publish',
  'system_settings',
  'install',
  'privileged_command',
  'share_personal_info',
  'irreversible',
];

/** An empty or null accelerator means the shortcut is disabled. */
export interface CompanionHotkeys {
  pause: string | null;
  ask: string | null;
  capture: string | null;
}

export interface CompanionSettings {
  /** The companion itself. Off until the user accepts the consent dialog. */
  enabled: boolean;
  level: CompanionLevel;
  sources: CompanionSources;
  /** Autonomous screen capture: at most one OCR read per this many seconds. */
  screen_min_interval_secs: number;
  /** Cloud chat model writes suggestions (default). False means local models only. */
  allow_cloud_model: boolean;
  /** Per-category level. High-risk categories are listed but always ask. */
  category_levels: Partial<Record<CompanionCategory, number>>;
  /** Bundle ids or app names the pet never looks at. */
  excluded_apps: string[];
  /** Case-insensitive substrings, or `re:`-prefixed regular expressions. */
  excluded_title_patterns: string[];
  chattiness: CompanionChattiness;
  retention_days: number;
  hotkeys: CompanionHotkeys;
  /** Sources the core reports as not available in this version, shown disabled. */
  unavailable_sources?: string[];
}

/** Flat patch. Lists are replaced wholesale; `category_levels` and `sources` merge. */
export type CompanionSettingsPatch = Partial<
  Omit<CompanionSettings, 'hotkeys' | 'unavailable_sources'>
> & { hotkeys?: Partial<CompanionHotkeys> };

export type CompanionState = 'off' | 'observing' | 'paused' | 'suspended';
export type SuspendedReason =
  | 'no_indicator'
  | 'permission_missing'
  | 'unsupported_platform'
  | 'quiet_hours'
  | 'helper_unavailable'
  | 'sensor_error';

export type DropReason =
  | 'paused'
  | 'no_indicator'
  | 'excluded_app'
  | 'title_rule'
  | 'secure_field'
  | 'concealed_clipboard'
  | 'sensitive_content'
  | 'source_off'
  | 'permission_missing';

export type ObservationKind =
  | 'app_switch'
  | 'title_change'
  | 'selection'
  | 'clipboard'
  | 'capture'
  | 'ask';

/** What the pet saw, already scrubbed. Never raw text. */
export interface ObservationSummary {
  at: string;
  kind: ObservationKind;
  app_name: string;
  bundle_id?: string | null;
  /** Scrubbed, at most 80 characters. */
  title_excerpt?: string | null;
  dropped?: DropReason | null;
}

export type PermissionState = 'granted' | 'denied' | 'unknown' | 'unsupported';

export interface CompanionMetrics {
  samples_total?: number;
  events_accepted?: number;
  drops_by_reason?: Partial<Record<DropReason, number>>;
  suggestions_created?: number;
  llm_calls?: number;
  ocr_calls?: number;
  sample_latency_p95_ms?: number;
}

export interface CompanionStatus {
  state: CompanionState;
  suspended_reason?: SuspendedReason | null;
  /**
   * True only while the autonomous capture loop is armed: permission granted,
   * not paused, not excluded. Drives the "observing screen" indicator.
   */
  screen_capture_active?: boolean;
  platform_supported: boolean;
  lease_active?: boolean;
  paused_until?: string | null;
  effective_level?: CompanionLevel;
  tier_cap?: CompanionLevel;
  permissions: {
    accessibility: PermissionState;
    screen_recording: PermissionState;
    helper?: 'ready' | 'unavailable' | 'unknown';
  };
  recent: ObservationSummary[];
  metrics: CompanionMetrics;
}

export type SuggestionTrigger = 'proactive' | 'ask' | 'capture';
export type SuggestionState = 'new' | 'shown' | 'acted' | 'saved' | 'dismissed' | 'expired';
export type SuggestionAction =
  | 'explain'
  | 'draft'
  | 'copy_text'
  | 'save_note'
  | 'open_chat'
  | 'prepare_command'
  | 'handoff'
  | 'dismiss'
  | 'mute_kind'
  | 'mute_app';

export interface SuggestionHandoff {
  thread_id: string;
  status: 'running' | 'done' | 'failed';
  /** Scrubbed, at most 600 characters. */
  result_excerpt?: string | null;
}

export interface CompanionSuggestion {
  id: string;
  created_at: string;
  trigger: SuggestionTrigger;
  kind: string;
  category: CompanionCategory;
  app_name: string;
  title_excerpt?: string | null;
  context_excerpt?: string | null;
  headline: string;
  /** Model output. Untrusted: render as plain text only. */
  body?: string | null;
  state: SuggestionState;
  score?: number;
  actions: SuggestionAction[];
  handoff?: SuggestionHandoff | null;
}

export interface CompanionActionLog {
  id: string;
  at: string;
  suggestion_id?: string | null;
  category: CompanionCategory;
  decision: 'auto' | 'confirmed' | 'refused_high_risk' | 'blocked_policy';
  level: number;
  outcome: 'ok' | 'error';
}

export interface CompanionData {
  suggestions: CompanionSuggestion[];
  actions: CompanionActionLog[];
  counts: { suggestions: number; actions: number };
}

export interface CompanionActResult {
  suggestion: CompanionSuggestion;
  chat_prompt?: string | null;
  copy_text?: string | null;
  command_text?: string | null;
}

export interface CompanionDeleteResult {
  deleted_suggestions: number;
  deleted_actions: number;
  deleted_notes: number;
}

export type PermissionKind = 'accessibility' | 'screen_recording';

export interface CompanionPermissionResult {
  state: PermissionState;
  opened_settings: boolean;
}

export type CompanionSource = 'ui' | 'tray' | 'hotkey';

const unwrapValue = <T>(raw: unknown): T => {
  if (raw && typeof raw === 'object' && !Array.isArray(raw) && 'result' in raw) {
    return (raw as { result: T }).result;
  }
  return raw as T;
};

async function call<T>(fn: string, params: Record<string, unknown> = {}): Promise<T> {
  log('rpc %s', fn);
  const raw = await callCoreRpc<unknown>({ method: `neppy.pet_companion_${fn}`, params });
  return unwrapValue<T>(raw);
}

export const getCompanionSettings = (): Promise<CompanionSettings> =>
  call<CompanionSettings>('get');

export const updateCompanionSettings = (
  patch: CompanionSettingsPatch
): Promise<CompanionSettings> => call<CompanionSettings>('update', { ...patch });

const normalizeStatus = (status: CompanionStatus): CompanionStatus => ({
  ...status,
  permissions: status?.permissions ?? { accessibility: 'unknown', screen_recording: 'unknown' },
  recent: status?.recent ?? [],
  metrics: status?.metrics ?? {},
});

export const getCompanionStatus = async (): Promise<CompanionStatus> =>
  normalizeStatus(await call<CompanionStatus>('status'));

/** Pause observation now. `minutes` omitted means until the user resumes. */
export const pauseCompanion = async (
  minutes?: number,
  source: CompanionSource = 'ui'
): Promise<CompanionStatus> =>
  normalizeStatus(
    await call<CompanionStatus>('pause', { ...(minutes ? { minutes } : {}), source })
  );

export const resumeCompanion = async (source: CompanionSource = 'ui'): Promise<CompanionStatus> =>
  normalizeStatus(await call<CompanionStatus>('resume', { source }));

export const fetchCompanionSuggestions = async (
  opts: { limit?: number; before?: string } = {}
): Promise<CompanionSuggestion[]> => {
  const rows = await call<CompanionSuggestion[]>('suggestions', { ...opts });
  return Array.isArray(rows) ? rows : [];
};

export const actOnCompanionSuggestion = (
  id: string,
  action: SuggestionAction,
  text?: string
): Promise<CompanionActResult> =>
  call<CompanionActResult>('suggestion_act', { id, action, ...(text ? { text } : {}) });

export const getCompanionData = async (): Promise<CompanionData> => {
  const data = await call<CompanionData>('data');
  return {
    suggestions: data?.suggestions ?? [],
    actions: data?.actions ?? [],
    counts: data?.counts ?? { suggestions: 0, actions: 0 },
  };
};

export const deleteCompanionData = (opts: {
  suggestionId?: string;
  all?: boolean;
  includeSavedNotes?: boolean;
}): Promise<CompanionDeleteResult> =>
  call<CompanionDeleteResult>('data_delete', {
    ...(opts.suggestionId ? { suggestion_id: opts.suggestionId } : {}),
    ...(opts.all ? { all: true } : {}),
    ...(opts.includeSavedNotes ? { include_saved_notes: true } : {}),
  });

/** Ask the OS for a permission. The core shows the system prompt or opens Settings. */
export const requestCompanionPermission = (kind: PermissionKind) =>
  call<CompanionPermissionResult>('request_permission', { kind });

// ---------------------------------------------------------------------------
// Socket `pet:companion` event payloads.
// ---------------------------------------------------------------------------

export type CompanionSocketEvent =
  | {
      type: 'state';
      state: CompanionState;
      paused?: boolean;
      suspended_reason?: SuspendedReason | null;
      screen_capture_active?: boolean;
    }
  | { type: 'suggestion'; suggestion: CompanionSuggestion }
  | { type: 'suggestion_update'; suggestion: CompanionSuggestion };
