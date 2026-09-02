import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  bootstrapLocalAiWithRecommendedPreset,
  ensureRecommendedLocalAiPresetIfNeeded,
} from '../localAiBootstrap';

vi.mock('../tauriCommands', () => ({
  neppyLocalAiApplyPreset: vi.fn(),
  neppyLocalAiPresets: vi.fn(),
}));

describe('localAiBootstrap', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('applies the recommended preset when no tier is selected', async () => {
    const tauriCommands = await import('../tauriCommands');
    vi.mocked(tauriCommands.neppyLocalAiPresets).mockResolvedValue({
      presets: [],
      recommended_tier: 'ram_2_4gb',
      current_tier: 'ram_2_4gb',
      selected_tier: null,
      device: {
        total_ram_bytes: 32 * 1024 * 1024 * 1024,
        cpu_count: 8,
        cpu_brand: 'Test CPU',
        os_name: 'macOS',
        os_version: '15',
        has_gpu: true,
        gpu_description: 'Test GPU',
      },
    });
    vi.mocked(tauriCommands.neppyLocalAiApplyPreset).mockResolvedValue({
      applied_tier: 'ram_2_4gb',
      chat_model_id: 'gemma3:1b-it-qat',
      vision_model_id: '',
      embedding_model_id: 'all-minilm:latest',
      quantization: 'qat',
    });
    const result = await bootstrapLocalAiWithRecommendedPreset(false, '[test]');

    expect(tauriCommands.neppyLocalAiPresets).toHaveBeenCalledOnce();
    expect(tauriCommands.neppyLocalAiApplyPreset).toHaveBeenCalledWith('ram_2_4gb');
    expect(result.preset.hadSelectedTier).toBe(false);
    expect(result.preset.appliedTier).toBe('ram_2_4gb');
  });

  it('skips preset application when a tier is already selected', async () => {
    const tauriCommands = await import('../tauriCommands');
    vi.mocked(tauriCommands.neppyLocalAiPresets).mockResolvedValue({
      presets: [],
      recommended_tier: 'ram_2_4gb',
      current_tier: 'ram_2_4gb',
      selected_tier: 'ram_2_4gb',
      device: {
        total_ram_bytes: 32 * 1024 * 1024 * 1024,
        cpu_count: 8,
        cpu_brand: 'Test CPU',
        os_name: 'macOS',
        os_version: '15',
        has_gpu: true,
        gpu_description: 'Test GPU',
      },
    });

    const result = await ensureRecommendedLocalAiPresetIfNeeded('[test]');

    expect(tauriCommands.neppyLocalAiApplyPreset).not.toHaveBeenCalled();
    expect(result.hadSelectedTier).toBe(true);
    expect(result.selectedTier).toBe('ram_2_4gb');
  });
});
