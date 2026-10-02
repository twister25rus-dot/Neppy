import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  actOnCompanionSuggestion,
  deleteCompanionData,
  fetchCompanionSuggestions,
  getCompanionData,
  getCompanionSettings,
  getCompanionStatus,
  pauseCompanion,
  requestCompanionPermission,
  resumeCompanion,
  updateCompanionSettings,
} from './petCompanionApi';

const mockCallCoreRpc = vi.fn();

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('petCompanionApi', () => {
  beforeEach(() => mockCallCoreRpc.mockReset());

  it('getCompanionSettings calls pet_companion_get and returns the bare value', async () => {
    mockCallCoreRpc.mockResolvedValue({ enabled: false });
    const settings = await getCompanionSettings();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_companion_get',
      params: {},
    });
    expect(settings.enabled).toBe(false);
  });

  it('unwraps the { result, logs } envelope', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { enabled: true }, logs: [] });
    expect((await getCompanionSettings()).enabled).toBe(true);
  });

  it('updateCompanionSettings sends a flat patch', async () => {
    mockCallCoreRpc.mockResolvedValue({ enabled: true });
    await updateCompanionSettings({ enabled: true, allow_cloud_model: false });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_companion_update',
      params: { enabled: true, allow_cloud_model: false },
    });
  });

  it('getCompanionStatus fills missing collections', async () => {
    mockCallCoreRpc.mockResolvedValue({ state: 'off', platform_supported: true });
    const status = await getCompanionStatus();
    expect(status.recent).toEqual([]);
    expect(status.metrics).toEqual({});
    expect(status.permissions.accessibility).toBe('unknown');
  });

  it('pauseCompanion sends minutes only when given, always the source', async () => {
    mockCallCoreRpc.mockResolvedValue({ state: 'paused' });
    await pauseCompanion();
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_companion_pause',
      params: { source: 'ui' },
    });
    await pauseCompanion(60);
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_companion_pause',
      params: { minutes: 60, source: 'ui' },
    });
  });

  it('resumeCompanion sends the source', async () => {
    mockCallCoreRpc.mockResolvedValue({ state: 'observing' });
    await resumeCompanion();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_companion_resume',
      params: { source: 'ui' },
    });
  });

  it('fetchCompanionSuggestions tolerates a non-array result', async () => {
    mockCallCoreRpc.mockResolvedValue(null);
    expect(await fetchCompanionSuggestions({ limit: 5 })).toEqual([]);
  });

  it('actOnCompanionSuggestion includes text only when provided', async () => {
    mockCallCoreRpc.mockResolvedValue({ suggestion: { id: 's1' } });
    await actOnCompanionSuggestion('s1', 'dismiss');
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_companion_suggestion_act',
      params: { id: 's1', action: 'dismiss' },
    });
    await actOnCompanionSuggestion('s1', 'handoff', 'do it');
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_companion_suggestion_act',
      params: { id: 's1', action: 'handoff', text: 'do it' },
    });
  });

  it('getCompanionData fills defaults', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    const data = await getCompanionData();
    expect(data.counts).toEqual({ suggestions: 0, actions: 0 });
  });

  it('deleteCompanionData maps options to wire params', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    await deleteCompanionData({ all: true });
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_companion_data_delete',
      params: { all: true },
    });
    await deleteCompanionData({ all: true, includeSavedNotes: true });
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_companion_data_delete',
      params: { all: true, include_saved_notes: true },
    });
    await deleteCompanionData({ suggestionId: 's9' });
    expect(mockCallCoreRpc).toHaveBeenLastCalledWith({
      method: 'openhuman.pet_companion_data_delete',
      params: { suggestion_id: 's9' },
    });
  });

  it('requestCompanionPermission passes the kind', async () => {
    mockCallCoreRpc.mockResolvedValue({ state: 'denied', opened_settings: true });
    const res = await requestCompanionPermission('screen_recording');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.pet_companion_request_permission',
      params: { kind: 'screen_recording' },
    });
    expect(res.opened_settings).toBe(true);
  });
});
