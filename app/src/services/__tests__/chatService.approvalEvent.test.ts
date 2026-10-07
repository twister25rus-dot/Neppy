import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { type ChatApprovalRequestEvent, subscribeChatEvents } from '../chatService';
import { socketService } from '../socketService';

function fakeSocket() {
  const handlers = new Map<string, (payload: unknown) => void>();
  return {
    id: 'sock-1',
    on: vi.fn((event: string, cb: (payload: unknown) => void) => {
      handlers.set(event, cb);
    }),
    off: vi.fn((event: string) => {
      handlers.delete(event);
    }),
    emit: (event: string, payload: unknown) => handlers.get(event)?.(payload),
    events: () => [...handlers.keys()],
  };
}

describe('approval_request event contract', () => {
  let socket: ReturnType<typeof fakeSocket>;

  beforeEach(() => {
    socket = fakeSocket();
    vi.spyOn(socketService, 'getSocket').mockReturnValue(
      socket as unknown as ReturnType<typeof socketService.getSocket>
    );
    vi.spyOn(socketService, 'on').mockImplementation((event, cb) =>
      socket.on(event, cb as (payload: unknown) => void)
    );
    vi.spyOn(socketService, 'off').mockImplementation(event => socket.off(event));
  });

  afterEach(() => vi.restoreAllMocks());

  it('listens on the exact event name the core emits', () => {
    // The core side: `ApprovalSurfaceSubscriber` in src/neppy/web_chat/event_bus.rs
    // publishes `event: "approval_request"`. If either side is renamed this fails.
    const rust = readFileSync(
      resolve(__dirname, '../../../../src/neppy/web_chat/event_bus.rs'),
      'utf8'
    );
    const emitted = /event: "(approval_request)"\.to_string\(\)/.exec(rust)?.[1];
    expect(emitted).toBe('approval_request');

    subscribeChatEvents({ onApprovalRequest: vi.fn() });
    expect(socket.events()).toContain(emitted);
  });

  it('delivers a debug-thread approval payload unchanged to the listener', () => {
    const onApprovalRequest = vi.fn();
    subscribeChatEvents({ onApprovalRequest });
    const event: ChatApprovalRequestEvent = {
      thread_id: 'dbg-1',
      request_id: 'req-1',
      tool_name: 'shell',
      message: 'Run `shell`: ls',
      args: { command: 'ls' },
    } as ChatApprovalRequestEvent;
    socket.emit('approval_request', event);
    expect(onApprovalRequest).toHaveBeenCalledWith(event);
  });
});
