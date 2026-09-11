import type { PresetId } from '../../components/chat/ChatPresetPill';
import { callCoreRpc } from '../coreRpcClient';

/**
 * The local-model run preset, read and written where the core keeps it.
 *
 * Core config rather than browser state on purpose: the chat bar and the
 * settings panel are two views of one setting, and a value cached per-surface
 * is how they end up disagreeing about what is selected.
 */
export async function getLocalModelPreset(): Promise<PresetId> {
  const result = await callCoreRpc<{ local_model_preset?: string } | null>({
    method: 'openhuman.inference_get_client_config',
  });
  const value = result?.local_model_preset;
  return isPresetId(value) ? value : 'auto';
}

export async function setLocalModelPreset(preset: PresetId): Promise<void> {
  await callCoreRpc({
    method: 'openhuman.config_update_model_settings',
    params: { local_model_preset: preset },
  });
}

const PRESET_IDS = ['auto', 'fast', 'balanced', 'deep', 'long_context', 'maximum_quality'] as const;

function isPresetId(value: unknown): value is PresetId {
  return typeof value === 'string' && (PRESET_IDS as readonly string[]).includes(value);
}
