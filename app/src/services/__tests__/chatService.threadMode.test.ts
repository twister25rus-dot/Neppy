import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { chatSend, type ChatThreadModeChangedEvent, subscribeChatEvents } from '../chatService';
import { callCoreRpc } from '../coreRpcClient';
import { socketService } from '../socketService';

vi.mock('../coreRpcClient', async () => {
  const actual = await vi.importActual<typeof import('../coreRpcClient')>('../coreRpcClient');
  return { ...actual, callCoreRpc: vi.fn() };
});

function fakeSocket() {
  const handlers = new Map<string, (payload: unknown) => void>();
  return {
    id: 'sock-1',
    on: vi.fn((event: string, cb: (payload: unknown) => void) => {
      handlers.set(event, cb);
    }),
    off: vi.fn((event: string, _cb?: (payload: unknown) => void) => {
      handlers.delete(event);
    }),
    emit: (event: string, payload: unknown) => handlers.get(event)?.(payload),
    has: (event: string) => handlers.has(event),
  };
}

describe('thread mode over the chat service', () => {
  let socket: ReturnType<typeof fakeSocket>;

  beforeEach(() => {
    socket = fakeSocket();
    vi.spyOn(socketService, 'getSocket').mockReturnValue(
      socket as unknown as ReturnType<typeof socketService.getSocket>
    );
    vi.spyOn(socketService, 'on').mockImplementation((event, cb) =>
      socket.on(event, cb as (payload: unknown) => void)
    );
    vi.spyOn(socketService, 'off').mockImplementation((event, cb) =>
      socket.off(event, cb as (payload: unknown) => void)
    );
    vi.mocked(callCoreRpc).mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('delivers thread_mode_changed to onThreadModeChanged and unsubscribes cleanly', () => {
    const onThreadModeChanged = vi.fn();
    const unsubscribe = subscribeChatEvents({ onThreadModeChanged });
    expect(socket.has('thread_mode_changed')).toBe(true);

    const event: ChatThreadModeChangedEvent = {
      thread_id: 'thread-1',
      args: { from: 'chat', to: 'orchestration', source: 'rpc' },
    };
    socket.emit('thread_mode_changed', event);
    expect(onThreadModeChanged).toHaveBeenCalledWith(event);

    unsubscribe();
    expect(socket.has('thread_mode_changed')).toBe(false);
  });

  it('does not subscribe when no listener asks for it', () => {
    subscribeChatEvents({});
    expect(socket.has('thread_mode_changed')).toBe(false);
  });

  it('sends the mode with the turn only when one is given', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue({ request_id: 'req-1' });
    await chatSend({ threadId: 't', message: 'hi', mode: 'orchestration' });
    expect(vi.mocked(callCoreRpc).mock.calls[0][0].params).toMatchObject({
      thread_id: 't',
      mode: 'orchestration',
    });

    await chatSend({ threadId: 't', message: 'hi' });
    expect(vi.mocked(callCoreRpc).mock.calls[1][0].params).toMatchObject({ mode: undefined });
  });
});
