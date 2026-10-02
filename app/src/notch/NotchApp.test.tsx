import { render, screen, waitFor } from '@testing-library/react';
import { act } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { connectCoreSocket } from '../services/coreSocket';
import NotchApp from './NotchApp';

// Key-passthrough i18n that honours the inline fallback the component passes
// (`t('notch.listening', 'Listening…')`).
vi.mock('../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string, fallback?: string) => fallback ?? key }),
}));

vi.mock('../services/coreSocket', () => ({ connectCoreSocket: vi.fn() }));

// Minimal Socket.IO stand-in: records handlers so a test can replay events.
class MockSocket {
  static last: MockSocket | null = null;
  handlers = new Map<string, (payload: unknown) => void>();
  connected = false;
  id = 'notch-test-socket';
  connect = vi.fn(() => {
    this.connected = true;
    return this;
  });
  disconnect = vi.fn(() => {
    this.connected = false;
    return this;
  });
  on = vi.fn((event: string, handler: (payload: unknown) => void) => {
    this.handlers.set(event, handler);
    return this;
  });
  fire(event: string, payload: unknown) {
    const handler = this.handlers.get(event);
    if (!handler) throw new Error(`no handler registered for ${event}`);
    act(() => handler(payload));
  }
  constructor() {
    MockSocket.last = this;
  }
}

describe('NotchApp', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    MockSocket.last = null;
    (window as { __OPENHUMAN_NOTCH_CORE_URL__?: string }).__OPENHUMAN_NOTCH_CORE_URL__ =
      'http://127.0.0.1:9999';
    vi.mocked(connectCoreSocket).mockImplementation(
      async () => new MockSocket() as unknown as Awaited<ReturnType<typeof connectCoreSocket>>
    );
  });

  afterEach(() => {
    delete (window as { __OPENHUMAN_NOTCH_CORE_URL__?: string }).__OPENHUMAN_NOTCH_CORE_URL__;
  });

  const renderAndConnect = async () => {
    render(<NotchApp />);
    // Idle baseline pill.
    expect(screen.getByText('Ready')).toBeInTheDocument();
    // Wait for the async connect to register its socket handlers.
    await waitFor(() => expect(MockSocket.last).not.toBeNull());
    await waitFor(() =>
      expect(MockSocket.last?.on).toHaveBeenCalledWith('overlay:attention', expect.any(Function))
    );
    return MockSocket.last as MockSocket;
  };

  it('connects using the preloaded core URL and shows the idle pill', async () => {
    const socket = await renderAndConnect();
    expect(connectCoreSocket).toHaveBeenCalledTimes(1);
    expect(socket.connect).toHaveBeenCalled();
  });

  it('renders Listening on a dictation press', async () => {
    const socket = await renderAndConnect();
    socket.fire('dictation:toggle', { type: 'pressed' });
    expect(await screen.findByText('Listening…')).toBeInTheDocument();
  });

  it('renders the transcript text on dictation:transcription', async () => {
    const socket = await renderAndConnect();
    socket.fire('dictation:transcription', { text: 'play some music' });
    expect(await screen.findByText('play some music')).toBeInTheDocument();
  });

  it('renders an overlay:attention message', async () => {
    const socket = await renderAndConnect();
    socket.fire('overlay:attention', { message: 'Opening Music', ttl_ms: 5000 });
    expect(await screen.findByText('Opening Music')).toBeInTheDocument();
  });

  it('connects via the notch:core-url event when no URL was preloaded', async () => {
    delete (window as { __OPENHUMAN_NOTCH_CORE_URL__?: string }).__OPENHUMAN_NOTCH_CORE_URL__;
    render(<NotchApp />);
    expect(screen.getByText('Ready')).toBeInTheDocument();
    expect(connectCoreSocket).not.toHaveBeenCalled();

    act(() =>
      window.dispatchEvent(
        new CustomEvent('notch:core-url', { detail: { url: 'http://127.0.0.1:8888' } })
      )
    );
    await waitFor(() => expect(connectCoreSocket).toHaveBeenCalledTimes(1));
  });

  describe('desktop companion', () => {
    it('shows a distinct baseline for observing, observing screen and paused', async () => {
      const socket = await renderAndConnect();
      expect(screen.queryByTestId('notch-pet-icon')).toBeNull();

      socket.fire('pet:companion', { type: 'state', state: 'observing', paused: false });
      expect(await screen.findByText('Pet watching')).toBeInTheDocument();
      expect(screen.getByTestId('notch-pet-icon').tagName.toLowerCase()).toBe('span');

      socket.fire('pet:companion', {
        type: 'state',
        state: 'observing',
        screen_capture_active: true,
      });
      expect(await screen.findByText('Pet watching screen')).toBeInTheDocument();
      expect(screen.getByTestId('notch-pet-icon').tagName.toLowerCase()).toBe('svg');

      socket.fire('pet:companion', { type: 'state', state: 'paused', paused: true });
      expect(await screen.findByText('Pet paused')).toBeInTheDocument();

      socket.fire('pet:companion', { type: 'state', state: 'off' });
      expect(await screen.findByText('Ready')).toBeInTheDocument();
    });

    it('treats a suspended companion as not observing', async () => {
      const socket = await renderAndConnect();
      socket.fire('pet:companion', { type: 'state', state: 'observing' });
      await screen.findByText('Pet watching');
      socket.fire('pet:companion', { type: 'state', state: 'suspended' });
      expect(await screen.findByText('Ready')).toBeInTheDocument();
    });

    it('shows a suggestion headline with the open hint, then returns to the baseline', async () => {
      const socket = await renderAndConnect();
      socket.fire('pet:companion', { type: 'state', state: 'observing' });
      await screen.findByText('Pet watching');

      // Capture the dismiss timer instead of waiting 8 real seconds.
      const timers: Array<{ cb: () => void; ms: number }> = [];
      const spy = vi.spyOn(window, 'setTimeout').mockImplementation(((
        cb: () => void,
        ms: number
      ) => {
        timers.push({ cb, ms });
        return timers.length;
      }) as never);
      try {
        socket.fire('pet:companion', {
          type: 'suggestion',
          suggestion: { headline: 'Want help with this build error?' },
        });
        expect(screen.getByText('Want help with this build error?')).toBeInTheDocument();
        expect(screen.getByTestId('notch-hint')).toHaveTextContent('⌥⇧⌘Space to open');

        expect(timers.at(-1)?.ms).toBe(8000);
        act(() => timers.at(-1)?.cb());
        expect(screen.getByText('Pet watching')).toBeInTheDocument();
        expect(screen.queryByTestId('notch-hint')).toBeNull();
      } finally {
        spy.mockRestore();
      }
    });

    it('ignores a suggestion without a headline', async () => {
      const socket = await renderAndConnect();
      socket.fire('pet:companion', { type: 'suggestion', suggestion: {} });
      expect(screen.getByText('Ready')).toBeInTheDocument();
      expect(screen.queryByTestId('notch-hint')).toBeNull();
    });
  });
});
