import { vi } from 'vitest';

import { makeCompanionSettings, makeCompanionStatus } from './companionFixtures';
import type { UseCompanion } from './useCompanion';

/** Test-only: a `UseCompanion` whose actions are spies. */
export function makeFakeCompanion(over: Partial<UseCompanion> = {}): UseCompanion {
  return {
    settings: makeCompanionSettings({ enabled: true }),
    status: makeCompanionStatus(),
    suggestions: [],
    loading: false,
    loadError: false,
    displayState: 'observing',
    refresh: vi.fn().mockResolvedValue(undefined),
    saveSettings: vi.fn().mockResolvedValue(makeCompanionSettings({ enabled: true })),
    pause: vi.fn().mockResolvedValue(undefined),
    resume: vi.fn().mockResolvedValue(undefined),
    requestPermission: vi.fn().mockResolvedValue(undefined),
    act: vi.fn(),
    ...over,
  };
}
