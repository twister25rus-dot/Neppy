import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { makeFeed, makeInbox, makePet } from './petFixtures';
import { PET_POLL_MS, usePet } from './usePet';

const mockGetPet = vi.fn();
const mockFeed = vi.fn();
const mockInbox = vi.fn();
const mockOn = vi.fn();
const mockOff = vi.fn();

vi.mock('../../services/api/petApi', () => ({
  getPet: (...args: unknown[]) => mockGetPet(...args),
  fetchPetFeed: (...args: unknown[]) => mockFeed(...args),
  fetchPetInbox: (...args: unknown[]) => mockInbox(...args),
}));
vi.mock('../../services/socketService', () => ({
  socketService: {
    on: (...args: unknown[]) => mockOn(...args),
    off: (...args: unknown[]) => mockOff(...args),
  },
}));

function notificationHandler(): (...args: unknown[]) => void {
  const call = mockOn.mock.calls.find(c => c[0] === 'core_notification');
  if (!call) throw new Error('core_notification listener not registered');
  return call[1] as (...args: unknown[]) => void;
}

describe('usePet', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetPet.mockResolvedValue(makePet());
    mockFeed.mockResolvedValue(makeFeed());
    mockInbox.mockResolvedValue(makeInbox());
  });
  afterEach(() => vi.useRealTimers());

  it('loads the profile, feed and inbox', async () => {
    const { result } = renderHook(() => usePet());
    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.pet?.name).toBe('Pip');
    expect(result.current.feed).not.toBeNull();
    expect(result.current.inbox).not.toBeNull();
    expect(result.current.loadError).toBe(false);
  });

  it('flags an error and recovers on the next refresh', async () => {
    mockGetPet.mockRejectedValueOnce(new Error('down'));
    const { result } = renderHook(() => usePet());
    await waitFor(() => expect(result.current.loadError).toBe(true));
    expect(result.current.pet).toBeNull();
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.loadError).toBe(false);
    expect(result.current.pet).not.toBeNull();
  });

  it('refreshes on a pet- core notification and ignores others', async () => {
    const { result } = renderHook(() => usePet());
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(mockGetPet).toHaveBeenCalledTimes(1);

    act(() => notificationHandler()({ id: 'cron-job:1' }));
    act(() => notificationHandler()({}));
    expect(mockGetPet).toHaveBeenCalledTimes(1);

    act(() => notificationHandler()({ id: 'pet-digest:abc' }));
    await waitFor(() => expect(mockGetPet).toHaveBeenCalledTimes(2));
  });

  it('polls every 30 seconds and stops after unmount', async () => {
    vi.useFakeTimers();
    const { unmount } = renderHook(() => usePet());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(mockGetPet).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(PET_POLL_MS);
    });
    expect(mockGetPet).toHaveBeenCalledTimes(2);

    unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(PET_POLL_MS * 3);
    });
    expect(mockGetPet).toHaveBeenCalledTimes(2);
    expect(mockOff).toHaveBeenCalledWith('core_notification', expect.any(Function));
  });
});
