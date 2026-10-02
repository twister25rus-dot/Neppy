import type {
  CompanionActionLog,
  CompanionData,
  CompanionSettings,
  CompanionStatus,
  CompanionSuggestion,
  ObservationSummary,
} from '../../../services/api/petCompanionApi';

/** Test-only builders for desktop companion wire objects. */
export const makeCompanionSettings = (
  over: Partial<CompanionSettings> = {}
): CompanionSettings => ({
  enabled: false,
  level: 1,
  sources: { app_window: true, selection: true, clipboard: true, screen_capture: true },
  screen_min_interval_secs: 30,
  allow_cloud_model: true,
  category_levels: {
    explain: 2,
    draft_text: 2,
    format_text: 1,
    save_note: 2,
    prepare_command: 1,
    open_chat: 1,
    handoff_task: 1,
    send_message: 1,
    delete: 1,
    purchase: 1,
    publish: 1,
    system_settings: 1,
    install: 1,
    privileged_command: 1,
    share_personal_info: 1,
    irreversible: 1,
  },
  excluded_apps: ['com.1password.1password'],
  excluded_title_patterns: ['incognito', 'private browsing'],
  chattiness: 'normal',
  retention_days: 7,
  hotkeys: {
    pause: 'CmdOrCtrl+Alt+Shift+P',
    ask: 'CmdOrCtrl+Alt+Shift+Space',
    capture: 'CmdOrCtrl+Alt+Shift+S',
  },
  unavailable_sources: ['browser_content'],
  ...over,
});

export const makeObservation = (over: Partial<ObservationSummary> = {}): ObservationSummary => ({
  at: '2020-01-01T09:00:00Z',
  kind: 'app_switch',
  app_name: 'Terminal',
  bundle_id: 'com.apple.Terminal',
  title_excerpt: 'zsh',
  dropped: null,
  ...over,
});

export const makeCompanionStatus = (over: Partial<CompanionStatus> = {}): CompanionStatus => ({
  state: 'observing',
  suspended_reason: null,
  screen_capture_active: false,
  platform_supported: true,
  lease_active: true,
  paused_until: null,
  effective_level: 1,
  tier_cap: 2,
  permissions: { accessibility: 'granted', screen_recording: 'granted', helper: 'ready' },
  recent: [],
  metrics: { samples_total: 12, events_accepted: 3, drops_by_reason: {}, llm_calls: 0 },
  ...over,
});

export const makeSuggestion = (over: Partial<CompanionSuggestion> = {}): CompanionSuggestion => ({
  id: 'sug-1',
  created_at: '2020-01-01T09:05:00Z',
  trigger: 'proactive',
  kind: 'build_error',
  category: 'explain',
  app_name: 'Terminal',
  title_excerpt: 'cargo build',
  context_excerpt: 'error[E0308]: mismatched types',
  headline: 'Want help with this build error?',
  body: 'The function returns a String but you passed a &str.',
  state: 'new',
  score: 70,
  actions: ['explain', 'copy_text', 'save_note', 'open_chat', 'handoff', 'dismiss', 'mute_kind'],
  handoff: null,
  ...over,
});

export const makeActionLog = (over: Partial<CompanionActionLog> = {}): CompanionActionLog => ({
  id: 'act-1',
  at: '2020-01-01T09:06:00Z',
  suggestion_id: 'sug-1',
  category: 'explain',
  decision: 'confirmed',
  level: 1,
  outcome: 'ok',
  ...over,
});

export const makeCompanionData = (over: Partial<CompanionData> = {}): CompanionData => ({
  suggestions: [makeSuggestion()],
  actions: [makeActionLog()],
  counts: { suggestions: 1, actions: 1 },
  ...over,
});
