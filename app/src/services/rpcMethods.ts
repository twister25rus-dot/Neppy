export const CORE_RPC_METHODS = {
  configGet: 'neppy.config_get',
  configGetAgentPaths: 'neppy.config_get_agent_paths',
  configGetAgentSettings: 'neppy.config_get_agent_settings',
  configGetAnalyticsSettings: 'neppy.config_get_analytics_settings',
  configGetAutonomySettings: 'neppy.config_get_autonomy_settings',
  configGetComposioTriggerSettings: 'neppy.config_get_composio_trigger_settings',
  configGetDashboardSettings: 'neppy.config_get_dashboard_settings',
  configGetRuntimeFlags: 'neppy.config_get_runtime_flags',
  configGetMemorySyncSettings: 'neppy.config_get_memory_sync_settings',
  configGetPrivacyMode: 'neppy.config_get_privacy_mode',
  configGetSandboxSettings: 'neppy.config_get_sandbox_settings',
  configGetSearchSettings: 'neppy.config_get_search_settings',
  configSetPrivacyMode: 'neppy.config_set_privacy_mode',
  configUpdateSearchSettings: 'neppy.config_update_search_settings',
  configSetBrowserAllowAll: 'neppy.config_set_browser_allow_all',
  configUpdateAgentPaths: 'neppy.config_update_agent_paths',
  configUpdateAgentSettings: 'neppy.config_update_agent_settings',
  configUpdateAnalyticsSettings: 'neppy.config_update_analytics_settings',
  configUpdateAutonomySettings: 'neppy.config_update_autonomy_settings',
  configUpdateBrowserSettings: 'neppy.config_update_browser_settings',
  configUpdateComposioTriggerSettings: 'neppy.config_update_composio_trigger_settings',
  configUpdateLocalAiSettings: 'neppy.config_update_local_ai_settings',
  configUpdateMemorySettings: 'neppy.config_update_memory_settings',
  configUpdateMemorySyncSettings: 'neppy.config_update_memory_sync_settings',
  configUpdateModelSettings: 'neppy.config_update_model_settings',
  configUpdateRuntimeSettings: 'neppy.config_update_runtime_settings',
  configUpdateSandboxSettings: 'neppy.config_update_sandbox_settings',
  configWorkspaceOnboardingFlagExists: 'neppy.config_workspace_onboarding_flag_exists',
  configWorkspaceOnboardingFlagSet: 'neppy.config_workspace_onboarding_flag_set',
  corePing: 'core.ping',
  inferenceAgentChat: 'neppy.inference_agent_chat',
  inferenceAgentChatSimple: 'neppy.inference_agent_chat_simple',
  inferenceApplyPreset: 'neppy.inference_apply_preset',
  inferenceAssetsStatus: 'neppy.inference_assets_status',
  inferenceDiagnostics: 'neppy.inference_diagnostics',
  inferenceDeviceProfile: 'neppy.inference_device_profile',
  inferenceDownloadAsset: 'neppy.inference_download_asset',
  inferenceDownloadsProgress: 'neppy.inference_downloads_progress',
  inferenceGetClientConfig: 'neppy.inference_get_client_config',
  inferenceInstallPiper: 'neppy.inference_install_piper',
  inferenceListModels: 'neppy.inference_list_models',
  inferencePiperInstallStatus: 'neppy.inference_piper_install_status',
  inferencePresets: 'neppy.inference_presets',
  inferenceTestConnection: 'neppy.inference_test_connection',
  inferenceTranscribe: 'neppy.inference_transcribe',
  inferenceTranscribeBytes: 'neppy.inference_transcribe_bytes',
  inferenceTts: 'neppy.inference_tts',
  inferenceUpdateLocalSettings: 'neppy.inference_update_local_settings',
  inferenceUpdateModelSettings: 'neppy.inference_update_model_settings',
  providersListModels: 'neppy.inference_list_models',
  embeddingsGetSettings: 'neppy.embeddings_get_settings',
  embeddingsUpdateSettings: 'neppy.embeddings_update_settings',
  embeddingsSetApiKey: 'neppy.embeddings_set_api_key',
  embeddingsClearApiKey: 'neppy.embeddings_clear_api_key',
  embeddingsEmbed: 'neppy.embeddings_embed',
  embeddingsTestConnection: 'neppy.embeddings_test_connection',
  channelsList: 'neppy.channels_list',
  mcpClientsInstalledList: 'neppy.mcp_clients_installed_list',
  mcpClientsToolCall: 'neppy.mcp_clients_tool_call',
  toolRegistryDiagnostics: 'neppy.tool_registry_diagnostics',
  healthSnapshot: 'neppy.health_snapshot',
  healthSystemInfo: 'neppy.health_system_info',
} as const;

type CoreRpcMethod = (typeof CORE_RPC_METHODS)[keyof typeof CORE_RPC_METHODS];

export const LEGACY_METHOD_ALIASES: Record<string, CoreRpcMethod> = {
  // #3565: old desktop clients used dotted namespace/function channel calls.
  'channels.list': CORE_RPC_METHODS.channelsList,
  // MCP clients — old method names that appeared in Sentry (CORE-RUST-DR/DS/DT/DV/DW).
  // See src/core/legacy_aliases.rs for the Rust-side mirror of this table.
  'mcp_clients.list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'neppy.channels.list': CORE_RPC_METHODS.channelsList,
  'neppy.mcp_clients_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'neppy.mcp_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'neppy.mcp_servers_list': CORE_RPC_METHODS.mcpClientsInstalledList,
  'neppy.tool_registry_call': CORE_RPC_METHODS.mcpClientsToolCall,
  // #3294: old desktop bundles called the tool-registry diagnostics
  // controller with the dotted `tool_registry.diagnostics` spelling before the
  // canonical `neppy.tool_registry_diagnostics` form, so the Tool Policy
  // diagnostics panel failed with "unknown method". Keep in sync with the
  // Rust-side mirror in src/core/legacy_aliases.rs.
  'tool_registry.diagnostics': CORE_RPC_METHODS.toolRegistryDiagnostics,
  'neppy.get_analytics_settings': CORE_RPC_METHODS.configGetAnalyticsSettings,
  'neppy.get_composio_trigger_settings': CORE_RPC_METHODS.configGetComposioTriggerSettings,
  'neppy.get_dashboard_settings': CORE_RPC_METHODS.configGetDashboardSettings,
  'neppy.get_config': CORE_RPC_METHODS.configGet,
  'neppy.get_runtime_flags': CORE_RPC_METHODS.configGetRuntimeFlags,
  'neppy.ping': CORE_RPC_METHODS.corePing,
  'neppy.set_browser_allow_all': CORE_RPC_METHODS.configSetBrowserAllowAll,
  'neppy.update_analytics_settings': CORE_RPC_METHODS.configUpdateAnalyticsSettings,
  'neppy.update_autonomy_settings': CORE_RPC_METHODS.configUpdateAutonomySettings,
  'neppy.update_browser_settings': CORE_RPC_METHODS.configUpdateBrowserSettings,
  'neppy.update_composio_trigger_settings': CORE_RPC_METHODS.configUpdateComposioTriggerSettings,
  'neppy.update_local_ai_settings': CORE_RPC_METHODS.inferenceUpdateLocalSettings,
  'neppy.update_memory_settings': CORE_RPC_METHODS.configUpdateMemorySettings,
  'neppy.update_model_settings': CORE_RPC_METHODS.inferenceUpdateModelSettings,
  'neppy.update_runtime_settings': CORE_RPC_METHODS.configUpdateRuntimeSettings,
  'neppy.workspace_onboarding_flag_exists': CORE_RPC_METHODS.configWorkspaceOnboardingFlagExists,
  'neppy.workspace_onboarding_flag_set': CORE_RPC_METHODS.configWorkspaceOnboardingFlagSet,
  'neppy.local_ai_agent_chat': CORE_RPC_METHODS.inferenceAgentChat,
  'neppy.local_ai_agent_chat_simple': CORE_RPC_METHODS.inferenceAgentChatSimple,
  'neppy.local_ai_apply_preset': CORE_RPC_METHODS.inferenceApplyPreset,
  'neppy.local_ai_assets_status': CORE_RPC_METHODS.inferenceAssetsStatus,
  'neppy.local_ai_device_profile': CORE_RPC_METHODS.inferenceDeviceProfile,
  'neppy.local_ai_diagnostics': CORE_RPC_METHODS.inferenceDiagnostics,
  'neppy.local_ai_download_asset': CORE_RPC_METHODS.inferenceDownloadAsset,
  'neppy.local_ai_downloads_progress': CORE_RPC_METHODS.inferenceDownloadsProgress,
  'neppy.local_ai_install_piper': CORE_RPC_METHODS.inferenceInstallPiper,
  'neppy.local_ai_piper_install_status': CORE_RPC_METHODS.inferencePiperInstallStatus,
  'neppy.local_ai_presets': CORE_RPC_METHODS.inferencePresets,
  'neppy.local_ai_test_connection': CORE_RPC_METHODS.inferenceTestConnection,
  'neppy.local_ai_transcribe': CORE_RPC_METHODS.inferenceTranscribe,
  'neppy.local_ai_transcribe_bytes': CORE_RPC_METHODS.inferenceTranscribeBytes,
  'neppy.local_ai_tts': CORE_RPC_METHODS.inferenceTts,
  'neppy.providers_list_models': CORE_RPC_METHODS.inferenceListModels,
  'neppy.inference_embed': CORE_RPC_METHODS.embeddingsEmbed,
  health_snapshot: CORE_RPC_METHODS.healthSnapshot,
  // Dotted / bare health probes from older clients and SDK callers (#3566,
  // Sentry CORE-2C). No distinct status/get handler exists — the snapshot
  // already carries the health verdict — so all four alias to the snapshot.
  // Keep in sync with src/core/legacy_aliases.rs (drift guard enforces it).
  health: CORE_RPC_METHODS.healthSnapshot,
  'health.get': CORE_RPC_METHODS.healthSnapshot,
  'health.snapshot': CORE_RPC_METHODS.healthSnapshot,
  'health.status': CORE_RPC_METHODS.healthSnapshot,
  // `neppy.system_info` was used by older clients / SDK callers before the
  // method was namespaced as `neppy.health_system_info`.
  // Sentry CORE-RUST-G0 — https://sentry.tinyhumans.ai/organizations/tinyhumans/issues/6340/
  'neppy.system_info': CORE_RPC_METHODS.healthSystemInfo,
};

/** Pre-rebrand RPC method prefix. Still accepted on input forever; never emitted. */
const LEGACY_RPC_METHOD_PREFIX = 'openhuman.';
/** Canonical RPC method prefix. */
const RPC_METHOD_PREFIX = 'neppy.';

export function normalizeRpcMethod(method: string): string {
  let normalized = method.trim().toLowerCase();

  // `openhuman.<x>` is a permanent legacy alias for `neppy.<x>` (mirrors
  // `normalize_rpc_method` in src/core/legacy_aliases.rs). Rewrite the prefix
  // first so the alias table below, which is written in the canonical spelling,
  // covers both.
  if (normalized.startsWith(LEGACY_RPC_METHOD_PREFIX)) {
    normalized = RPC_METHOD_PREFIX + normalized.slice(LEGACY_RPC_METHOD_PREFIX.length);
  }

  if (normalized in LEGACY_METHOD_ALIASES) {
    return LEGACY_METHOD_ALIASES[normalized];
  }

  if (normalized.startsWith('neppy.auth.')) {
    return `neppy.auth_${normalized.slice('neppy.auth.'.length).split('.').join('_')}`;
  }

  return normalized;
}
