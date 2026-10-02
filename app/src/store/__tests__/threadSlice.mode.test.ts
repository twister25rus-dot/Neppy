import { configureStore } from '@reduxjs/toolkit';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { threadApi } from '../../services/api/threadApi';
import { resolveThreadMode, type Thread } from '../../types/thread';
import threadReducer, { applyThreadMode, loadThreads, setThreadMode } from '../threadSlice';

vi.mock('../../services/api/threadApi', () => ({
  threadApi: { getThreads: vi.fn(), setMode: vi.fn() },
}));

const mockedApi = vi.mocked(threadApi);

function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't-1',
    title: 'Untitled',
    chatId: null,
    isActive: false,
    messageCount: 0,
    lastMessageAt: '2026-01-01T00:00:00.000Z',
    createdAt: '2026-01-01T00:00:00.000Z',
    labels: [],
    ...overrides,
  };
}

async function storeWith(threads: Thread[]) {
  const store = configureStore({ reducer: { thread: threadReducer } });
  mockedApi.getThreads.mockResolvedValue({ threads, count: threads.length });
  await store.dispatch(loadThreads());
  return store;
}

describe('thread mode', () => {
  beforeEach(() => vi.clearAllMocks());

  it('treats a missing or unknown wire value as chat', () => {
    expect(resolveThreadMode(undefined)).toBe('chat');
    expect(resolveThreadMode(null)).toBe('chat');
    expect(resolveThreadMode('something-new')).toBe('chat');
    expect(resolveThreadMode('orchestration')).toBe('orchestration');
  });

  it('flips optimistically and adopts the persisted thread on success', async () => {
    const store = await storeWith([makeThread()]);
    mockedApi.setMode.mockResolvedValue({
      thread: makeThread({ mode: 'orchestration', title: 'Renamed' }),
      previousMode: 'chat',
      changed: true,
    });

    const pending = store.dispatch(
      setThreadMode({ threadId: 't-1', mode: 'orchestration', previous: 'chat' })
    );
    // Optimistic: visible before the RPC resolves.
    expect(store.getState().thread.threads[0].mode).toBe('orchestration');
    await pending;

    expect(mockedApi.setMode).toHaveBeenCalledWith('t-1', 'orchestration', undefined);
    expect(store.getState().thread.threads[0].mode).toBe('orchestration');
    expect(store.getState().thread.threads[0].title).toBe('Renamed');
    // Same thread, same id: nothing was recreated.
    expect(store.getState().thread.threads).toHaveLength(1);
  });

  it('reverts to the previous mode when the core rejects the change', async () => {
    const store = await storeWith([makeThread({ mode: 'chat' })]);
    mockedApi.setMode.mockRejectedValue(new Error('unknown thread mode'));

    await store.dispatch(
      setThreadMode({ threadId: 't-1', mode: 'orchestration', previous: 'chat' })
    );

    expect(store.getState().thread.threads[0].mode).toBe('chat');
  });

  it('applyThreadMode reflects an external change and ignores unknown threads', async () => {
    const store = await storeWith([makeThread({ id: 'a' }), makeThread({ id: 'b' })]);
    store.dispatch(applyThreadMode({ threadId: 'a', mode: 'orchestration' }));
    store.dispatch(applyThreadMode({ threadId: 'zzz', mode: 'orchestration' }));
    const [a, b] = store.getState().thread.threads;
    expect(a.mode).toBe('orchestration');
    expect(resolveThreadMode(b.mode)).toBe('chat');
  });
});
