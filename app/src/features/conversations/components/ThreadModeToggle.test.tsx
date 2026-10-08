import { configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import threadReducer, { applyThreadMode, loadThreads } from '../../../store/threadSlice';
import { ThreadModeToggle } from './ThreadModeToggle';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const thread = (overrides: Record<string, unknown> = {}) => ({
  id: 't-1',
  title: 'T',
  chatId: null,
  isActive: false,
  messageCount: 0,
  lastMessageAt: '',
  createdAt: '',
  labels: [],
  ...overrides,
});

async function setup(threadOverrides: Record<string, unknown> = {}) {
  const store = configureStore({ reducer: { thread: threadReducer } });
  vi.mocked(callCoreRpc).mockImplementation(async ({ method }) => {
    if (method === 'neppy.threads_list') {
      return { data: { threads: [thread(threadOverrides)], count: 1 } };
    }
    throw new Error(`unexpected ${method}`);
  });
  await store.dispatch(loadThreads());
  vi.mocked(callCoreRpc).mockReset();
  render(
    <Provider store={store}>
      <MemoryRouter>
        <ThreadModeToggle threadId="t-1" />
      </MemoryRouter>
    </Provider>
  );
  return store;
}

describe('ThreadModeToggle', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  it('defaults to Chat for a thread with no persisted mode', async () => {
    await setup();
    expect(screen.getByTestId('thread-mode-chat')).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByTestId('thread-mode-orchestration')).toHaveAttribute(
      'aria-checked',
      'false'
    );
  });

  it('explains each mode in a tooltip', async () => {
    await setup();
    expect(screen.getByTestId('thread-mode-chat')).toHaveAttribute(
      'title',
      'One assistant, full tools'
    );
    expect(screen.getByTestId('thread-mode-orchestration')).toHaveAttribute(
      'title',
      'A supervisor coordinates specialist agents'
    );
  });

  it('persists a switch via threads_set_mode on the same thread', async () => {
    const store = await setup();
    vi.mocked(callCoreRpc).mockResolvedValue({
      data: { thread: thread({ mode: 'orchestration' }), previousMode: 'chat', changed: true },
    });

    fireEvent.click(screen.getByTestId('thread-mode-orchestration'));

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'neppy.threads_set_mode',
      params: { thread_id: 't-1', mode: 'orchestration', source: 'composer_toggle' },
    });
    await waitFor(() =>
      expect(screen.getByTestId('thread-mode-orchestration')).toHaveAttribute(
        'aria-checked',
        'true'
      )
    );
    expect(store.getState().thread.threads).toHaveLength(1);
  });

  it('does nothing when the active mode is clicked again', async () => {
    await setup();
    fireEvent.click(screen.getByTestId('thread-mode-chat'));
    expect(callCoreRpc).not.toHaveBeenCalled();
  });

  it('reflects a mode change made elsewhere (thread_mode_changed)', async () => {
    const store = await setup();
    act(() => {
      store.dispatch(applyThreadMode({ threadId: 't-1', mode: 'orchestration' }));
    });
    expect(screen.getByTestId('thread-mode-orchestration')).toHaveAttribute('aria-checked', 'true');
  });

  it('reverts and shows an error when the core refuses the switch', async () => {
    await setup({ mode: 'orchestration' });
    vi.mocked(callCoreRpc).mockImplementation(async () => {
      throw new Error('nope');
    });

    fireEvent.click(screen.getByTestId('thread-mode-chat'));

    expect(await screen.findByRole('alert')).toHaveTextContent('Could not change the mode.');
    expect(screen.getByTestId('thread-mode-orchestration')).toHaveAttribute('aria-checked', 'true');
  });

  it('offers Chat, Orchestration and Debug', async () => {
    await setup();
    expect(screen.getAllByRole('radio')).toHaveLength(3);
    expect(screen.getByTestId('thread-mode-debug')).toHaveAttribute('aria-checked', 'false');
  });

  it('shows Debug selected for a thread that is in Debug mode', async () => {
    await setup({ mode: 'debug', labels: ['mode:debug'] });
    expect(screen.getByTestId('thread-mode-debug')).toHaveAttribute('aria-checked', 'true');
  });

  it('checks the source repository before switching to Debug', async () => {
    await setup();
    vi.mocked(callCoreRpc).mockImplementation(async ({ method }) => {
      if (method === 'neppy.threads_set_mode') {
        return { data: { thread: thread({ mode: 'debug' }), previousMode: 'chat', changed: true } };
      }
      return { data: {} };
    });

    fireEvent.click(screen.getByTestId('thread-mode-debug'));

    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'neppy.threads_set_mode',
        params: { thread_id: 't-1', mode: 'debug', source: 'composer_toggle' },
      })
    );
    const methods = vi.mocked(callCoreRpc).mock.calls.map(([req]) => req.method);
    expect(methods.indexOf('neppy.threads_set_mode')).toBeGreaterThan(0);
  });

  it('stays out of Debug and explains why when there is no source repository', async () => {
    await setup();
    vi.mocked(callCoreRpc).mockImplementation(async () => {
      throw new Error('no repo');
    });

    fireEvent.click(screen.getByTestId('thread-mode-debug'));

    expect(await screen.findByTestId('thread-mode-debug-notice')).toBeInTheDocument();
    expect(screen.getByTestId('thread-mode-chat')).toHaveAttribute('aria-checked', 'true');
    const methods = vi.mocked(callCoreRpc).mock.calls.map(([req]) => req.method);
    expect(methods).not.toContain('neppy.threads_set_mode');
  });

  it('switches out of Debug straight to the chosen tab', async () => {
    await setup({ mode: 'debug', labels: ['mode:debug'] });
    vi.mocked(callCoreRpc).mockResolvedValue({
      data: { thread: thread({ mode: 'orchestration' }), previousMode: 'debug', changed: true },
    });

    fireEvent.click(screen.getByTestId('thread-mode-orchestration'));

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'neppy.threads_set_mode',
      params: { thread_id: 't-1', mode: 'orchestration', source: 'composer_toggle' },
    });
  });

  it('renders the large hero variant on request', async () => {
    const store = configureStore({ reducer: { thread: threadReducer } });
    render(
      <Provider store={store}>
        <MemoryRouter>
          <ThreadModeToggle threadId="t-1" size="lg" />
        </MemoryRouter>
      </Provider>
    );
    expect(screen.getByTestId('thread-mode-toggle')).toHaveAttribute('data-size', 'lg');
  });

  it('renders nothing without a thread', () => {
    const store = configureStore({ reducer: { thread: threadReducer } });
    const { container } = render(
      <Provider store={store}>
        <ThreadModeToggle threadId={null} />
      </Provider>
    );
    expect(container).toBeEmptyDOMElement();
  });
});
