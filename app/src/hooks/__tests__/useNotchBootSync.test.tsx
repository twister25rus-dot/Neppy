import { renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  neppyGetVoiceServerSettings,
  syncNotchVisibility,
  type VoiceServerSettings,
} from '../../utils/tauriCommands';
import { useNotchBootSync } from '../useNotchBootSync';

vi.mock('../../utils/tauriCommands', () => ({
  neppyGetVoiceServerSettings: vi.fn(),
  syncNotchVisibility: vi.fn(),
}));

const settings = (alwaysOn: boolean) => ({
  result: { always_on_enabled: alwaysOn } as unknown as VoiceServerSettings,
  logs: [],
});

describe('useNotchBootSync', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(syncNotchVisibility).mockResolvedValue(undefined);
  });

  it('does nothing while still bootstrapping', () => {
    renderHook(() => useNotchBootSync(true));
    expect(neppyGetVoiceServerSettings).not.toHaveBeenCalled();
    expect(syncNotchVisibility).not.toHaveBeenCalled();
  });

  it('syncs the notch to the persisted always-on flag once the core is ready', async () => {
    vi.mocked(neppyGetVoiceServerSettings).mockResolvedValue(settings(true));
    renderHook(() => useNotchBootSync(false));
    await waitFor(() => expect(syncNotchVisibility).toHaveBeenCalledWith(true));
    expect(neppyGetVoiceServerSettings).toHaveBeenCalledTimes(1);
  });

  it('only syncs once per boot across re-renders', async () => {
    vi.mocked(neppyGetVoiceServerSettings).mockResolvedValue(settings(false));
    const { rerender } = renderHook(({ b }) => useNotchBootSync(b), { initialProps: { b: false } });
    await waitFor(() => expect(syncNotchVisibility).toHaveBeenCalledWith(false));
    rerender({ b: false });
    expect(neppyGetVoiceServerSettings).toHaveBeenCalledTimes(1);
  });

  it('swallows failures (cosmetic) so boot is never blocked', async () => {
    vi.mocked(neppyGetVoiceServerSettings).mockRejectedValue(new Error('core offline'));
    renderHook(() => useNotchBootSync(false));
    // The settings fetch is attempted, the rejection is caught (no throw), and
    // notch visibility is never toggled.
    await waitFor(() => expect(neppyGetVoiceServerSettings).toHaveBeenCalled());
    expect(syncNotchVisibility).not.toHaveBeenCalled();
  });
});
