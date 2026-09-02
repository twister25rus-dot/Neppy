import { isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands/config', () => {
  const mockIsTauri = isTauri as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;
  let neppyGetAutonomySettings: typeof import('./config').neppyGetAutonomySettings;
  let neppyUpdateAutonomySettings: typeof import('./config').neppyUpdateAutonomySettings;
  let neppyUpdateLocalAiSettings: typeof import('./config').neppyUpdateLocalAiSettings;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const actual = await vi.importActual<typeof import('./config')>('./config');
    neppyGetAutonomySettings = actual.neppyGetAutonomySettings;
    neppyUpdateAutonomySettings = actual.neppyUpdateAutonomySettings;
    neppyUpdateLocalAiSettings = actual.neppyUpdateLocalAiSettings;
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe('neppyUpdateLocalAiSettings', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(neppyUpdateLocalAiSettings({ runtime_enabled: true })).rejects.toThrow(
        'Not running in Tauri'
      );
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('forwards the patch to openhuman.inference_update_local_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
        logs: [],
      });
      const patch = {
        runtime_enabled: true,
        opt_in_confirmed: true,
        provider: 'lm_studio',
        base_url: 'http://localhost:1234/v1',
        model_id: 'local-model',
        chat_model_id: 'local-model',
        usage_embeddings: true,
        usage_subconscious: false,
      };
      await neppyUpdateLocalAiSettings(patch);
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.inference_update_local_settings',
        params: patch,
      });
    });
  });

  describe('neppyUpdateAutonomySettings', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(neppyUpdateAutonomySettings({ max_actions_per_hour: 100 })).rejects.toThrow(
        'Not running in Tauri'
      );
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('forwards the patch to openhuman.config_update_autonomy_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
        logs: [],
      });
      await neppyUpdateAutonomySettings({ max_actions_per_hour: 100 });
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_update_autonomy_settings',
        params: { max_actions_per_hour: 100 },
      });
    });
  });

  describe('neppyGetAutonomySettings', () => {
    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(neppyGetAutonomySettings()).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('reads via openhuman.config_get_autonomy_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({ result: { max_actions_per_hour: 250 }, logs: [] });
      const out = await neppyGetAutonomySettings();
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_get_autonomy_settings',
      });
      expect(out.result.max_actions_per_hour).toBe(250);
    });
  });

  describe('neppyUpdateComposioTriggerSettings', () => {
    let neppyUpdateComposioTriggerSettings: typeof import('./config').neppyUpdateComposioTriggerSettings;

    beforeEach(async () => {
      const actual = await vi.importActual<typeof import('./config')>('./config');
      neppyUpdateComposioTriggerSettings = actual.neppyUpdateComposioTriggerSettings;
    });

    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(
        neppyUpdateComposioTriggerSettings({ triage_disabled: true })
      ).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('forwards the patch to openhuman.config_update_composio_trigger_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { config: {}, workspace_dir: '/tmp', config_path: '/tmp/cfg.toml' },
        logs: [],
      });
      const patch = { triage_disabled: true, triage_disabled_toolkits: ['gmail', 'slack'] };
      await neppyUpdateComposioTriggerSettings(patch);
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_update_composio_trigger_settings',
        params: patch,
      });
    });

    test('returns no-op on unknown method from stale core (#1597)', async () => {
      mockCallCoreRpc.mockRejectedValue(
        new Error('unknown method: openhuman.config_update_composio_trigger_settings')
      );
      const out = await neppyUpdateComposioTriggerSettings({ triage_disabled: true });
      expect(out).toEqual({ result: { config: {}, workspace_dir: '', config_path: '' }, logs: [] });
    });

    test('rethrows non-unknown-method errors', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('network timeout'));
      await expect(
        neppyUpdateComposioTriggerSettings({ triage_disabled: true })
      ).rejects.toThrow('network timeout');
    });
  });

  describe('neppyGetComposioTriggerSettings', () => {
    let neppyGetComposioTriggerSettings: typeof import('./config').neppyGetComposioTriggerSettings;

    beforeEach(async () => {
      const actual = await vi.importActual<typeof import('./config')>('./config');
      neppyGetComposioTriggerSettings = actual.neppyGetComposioTriggerSettings;
    });

    test('throws when not running in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(neppyGetComposioTriggerSettings()).rejects.toThrow('Not running in Tauri');
      expect(mockCallCoreRpc).not.toHaveBeenCalled();
    });

    test('reads via openhuman.config_get_composio_trigger_settings', async () => {
      mockCallCoreRpc.mockResolvedValue({
        result: { triage_disabled: false, triage_disabled_toolkits: ['slack'] },
        logs: [],
      });
      const out = await neppyGetComposioTriggerSettings();
      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_get_composio_trigger_settings',
      });
      expect(out.result.triage_disabled).toBe(false);
      expect(out.result.triage_disabled_toolkits).toEqual(['slack']);
    });

    test('returns defaults on unknown method from stale core (#1597)', async () => {
      mockCallCoreRpc.mockRejectedValue(
        new Error('unknown method: openhuman.config_get_composio_trigger_settings')
      );
      const out = await neppyGetComposioTriggerSettings();
      expect(out.result.triage_disabled).toBe(false);
      expect(out.result.triage_disabled_toolkits).toEqual([]);
    });

    test('rethrows non-unknown-method errors', async () => {
      mockCallCoreRpc.mockRejectedValue(new Error('network timeout'));
      await expect(neppyGetComposioTriggerSettings()).rejects.toThrow('network timeout');
    });
  });
});
