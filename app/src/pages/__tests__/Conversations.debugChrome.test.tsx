/**
 * Debug Mode is a per-thread mode now (the top-bar switch), so the normal chat
 * view owns its banner and panels and the thread list owns its row badge.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { SidebarSlotOutlet, SidebarSlotProvider } from '../../components/layout/shell/SidebarSlot';
import agentProfileReducer from '../../store/agentProfileSlice';
import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import layoutReducer from '../../store/layoutSlice';
import socketReducer from '../../store/socketSlice';
import themeReducer from '../../store/themeSlice';
import threadReducer from '../../store/threadSlice';
import type { Thread } from '../../types/thread';

// ── Hoisted mock state ─────────────────────────────────────────────────────

const { mockGetThreads, mockGetThreadMessages, mockUseUsageState } = vi.hoisted(() => ({
  mockGetThreads: vi.fn().mockResolvedValue({ threads: [], count: 0 }),
  mockGetThreadMessages: vi.fn().mockResolvedValue({ messages: [], count: 0 }),
  mockUseUsageState: vi.fn(() => ({
    teamUsage: null as null | {
      cycleBudgetUsd: number;
      remainingUsd: number;
      cycleSpentUsd: number;
      cycleEndsAt: string | null;
    },
    currentPlan: null,
    currentTier: 'FREE' as 'FREE' | 'BASIC' | 'PRO',
    isFreeTier: true,
    usagePct: 0,
    isNearLimit: false,
    isAtLimit: false,
    isBudgetExhausted: false,
    shouldShowBudgetCompletedMessage: false,
    isLoading: false,
    refresh: vi.fn(),
  })),
}));

// ── Module mocks (mirror Conversations.render.test.tsx's known-good set) ────

vi.mock('../../services/chatService', () => ({
  chatCancel: vi.fn(),
  chatSend: vi.fn().mockResolvedValue(undefined),
  subscribeChatEvents: vi.fn(() => () => {}),
  useRustChat: vi.fn(() => true),
}));

vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: vi.fn().mockResolvedValue({ id: 'new-thread', labels: [] }),
    getThreads: mockGetThreads,
    getThreadMessages: mockGetThreadMessages,
    getTurnState: vi.fn().mockResolvedValue(null),
    getTaskBoard: vi.fn().mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '' }),
    putTaskBoard: vi.fn().mockResolvedValue({ threadId: 't-1', cards: [], updatedAt: '' }),
    appendMessage: vi.fn().mockResolvedValue({}),
    deleteThread: vi.fn().mockResolvedValue({ deleted: true }),
    generateTitleIfNeeded: vi.fn().mockResolvedValue({}),
    updateMessage: vi.fn().mockResolvedValue({}),
    purge: vi.fn().mockResolvedValue({}),
    updateLabels: vi.fn().mockResolvedValue({}),
    updateTitle: vi.fn().mockResolvedValue({}),
    persistReaction: vi.fn().mockResolvedValue({}),
  },
}));

vi.mock('../../services/api/agentProfilesApi', () => ({
  agentProfilesApi: {
    list: vi
      .fn()
      .mockResolvedValue({
        activeProfileId: 'default',
        profiles: [
          {
            id: 'default',
            name: 'Default',
            description: 'Default',
            agentId: 'orchestrator',
            builtIn: true,
          },
        ],
      }),
    select: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    upsert: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
    delete: vi.fn().mockResolvedValue({ activeProfileId: 'default', profiles: [] }),
  },
}));

vi.mock('../../services/api/openrouterFreeModels', () => ({ applyOpenRouterFreeModels: vi.fn() }));

vi.mock('../../hooks/useUsageState', () => ({ useUsageState: mockUseUsageState }));

// The new-window hero pulls useUser/useCoreState; stub it so the page renders
// without a CoreStateProvider.
vi.mock('../../components/chat/ChatNewWindowHero', () => ({ default: () => null }));

vi.mock('../../store/socketSelectors', () => ({
  selectSocketStatus: (state: { socket?: { byUser?: Record<string, { status: string }> } }) =>
    state.socket?.byUser?.__pending__?.status ?? 'disconnected',
}));

// useStickToBottom returns refs; mock it so layout-effects don't fire in jsdom.
vi.mock('../../hooks/useStickToBottom', () => ({
  useStickToBottom: vi.fn(() => ({ containerRef: { current: null }, endRef: { current: null } })),
}));

vi.mock('../../utils/openUrl', () => ({ openUrl: vi.fn() }));

vi.mock('../../lib/coreState/store', () => ({
  getCoreStateSnapshot: vi.fn(() => ({
    isBootstrapping: false,
    isReady: true,
    snapshot: {
      auth: { isAuthenticated: false, userId: null, user: null, profileId: null },
      sessionToken: null,
      currentUser: null,
      onboardingCompleted: true,
      chatOnboardingCompleted: true,
      analyticsEnabled: false,
      localState: {},
      runtime: {},
    },
  })),
  isWelcomeLocked: vi.fn(() => false),
  setCoreStateSnapshot: vi.fn(),
}));

// ── Helpers ────────────────────────────────────────────────────────────────

/** Build a minimal Redux store with the slices Conversations reads, optionally preloaded. */
function buildStore(preload: Record<string, unknown> = {}) {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      layout: layoutReducer,
      socket: socketReducer,
      chatRuntime: chatRuntimeReducer,
      agentProfiles: agentProfileReducer,
      theme: themeReducer,
    }),
    preloadedState: preload as never,
  });
}

/** Construct a `Thread` fixture with sensible defaults, overridable per field. */
function makeThread(overrides: Partial<Thread> = {}): Thread {
  return {
    id: 't-1',
    title: 'Test thread',
    chatId: null,
    isActive: false,
    messageCount: 0,
    lastMessageAt: '2026-01-01T00:00:00.000Z',
    createdAt: '2026-01-01T00:00:00.000Z',
    labels: ['general'],
    ...overrides,
  };
}

const emptyThreadState = {
  threads: [],
  selectedThreadId: null,
  activeThreadIds: {},
  welcomeThreadId: null,
  messagesByThreadId: {},
  messages: [],
  isLoadingThreads: false,
  isLoadingMessages: false,
  messagesError: null,
};

/** Thread-slice preload with `thread` present, selected, and holding an empty message list. */
function selectedThreadState(thread: Thread) {
  return {
    ...emptyThreadState,
    threads: [thread],
    selectedThreadId: thread.id,
    messagesByThreadId: { [thread.id]: [] },
    messages: [],
  };
}

/** Socket-slice preload that pins the pending-user connection to the given status. */
function socketState(status: 'connected' | 'disconnected') {
  return {
    byUser: { __pending__: { status, socketId: status === 'connected' ? 'socket-1' : null } },
  };
}

/** Render the normal chat page (`/chat`) around Conversations. */
async function renderChat(preload: Record<string, unknown>) {
  const store = buildStore(preload);
  const { default: Conversations } = await import('../../features/conversations/Conversations');
  await act(async () => {
    render(
      <Provider store={store}>
        <MemoryRouter initialEntries={['/chat']}>
          <SidebarSlotProvider>
            <SidebarSlotOutlet />
            <Conversations variant="page" projectThreadList />
          </SidebarSlotProvider>
        </MemoryRouter>
      </Provider>
    );
  });
  return store;
}

// The chrome has its own suite; here only whether the chat view mounts it.
vi.mock('../../features/debug/DebugThreadChrome', () => ({
  DebugThreadChrome: ({ threadId }: { threadId: string }) => (
    <div data-testid="debug-thread-chrome-stub" data-thread={threadId} />
  ),
}));

describe('Conversations: Debug Mode chrome', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    window.localStorage.clear();
    mockGetThreadMessages.mockResolvedValue({ messages: [], count: 0 });
  });

  it('renders the debug chrome above the conversation for a thread in debug mode', async () => {
    const dbg = makeThread({ id: 'dbg-1', title: 'Debug thread', mode: 'debug' });
    mockGetThreads.mockResolvedValue({ threads: [dbg], count: 1 });

    await renderChat({ thread: selectedThreadState(dbg), socket: socketState('connected') });

    const chrome = await screen.findByTestId('debug-thread-chrome-stub');
    expect(chrome).toHaveAttribute('data-thread', 'dbg-1');
  });

  it('renders nothing extra for a normal thread', async () => {
    const normal = makeThread({ id: 'c-1', title: 'Normal thread' });
    mockGetThreads.mockResolvedValue({ threads: [normal], count: 1 });

    await renderChat({ thread: selectedThreadState(normal), socket: socketState('connected') });

    expect(screen.queryByTestId('debug-thread-chrome-stub')).toBeNull();
  });

  it('lists debug threads like any other thread, with a Debug badge on the row', async () => {
    const dbg = makeThread({ id: 'dbg-1', title: 'Debug thread', mode: 'debug' });
    const normal = makeThread({ id: 'c-1', title: 'Normal thread' });
    mockGetThreads.mockResolvedValue({ threads: [dbg, normal], count: 2 });

    await renderChat({
      thread: { ...selectedThreadState(normal), threads: [dbg, normal] },
      socket: socketState('connected'),
    });

    expect(await screen.findByTestId('thread-debug-badge-dbg-1')).toHaveTextContent('Debug');
    expect(screen.queryByTestId('thread-debug-badge-c-1')).toBeNull();
    expect(screen.getAllByText('Debug thread').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Normal thread').length).toBeGreaterThan(0);
  });

  describe('parked approval cards (assistant-ui default composer)', () => {
    const approval = {
      requestId: 'req-1',
      toolName: 'shell',
      message: 'Run `shell`: ls',
      command: 'ls -la',
    };

    it('renders the approval card for a debug thread', async () => {
      const dbg = makeThread({ id: 'dbg-1', title: 'Debug thread', mode: 'debug' });
      mockGetThreads.mockResolvedValue({ threads: [dbg], count: 1 });
      const chatRuntime = {
        ...chatRuntimeReducer(undefined, { type: '@@init' }),
        pendingApprovalByThread: { 'dbg-1': [approval] },
      };

      await renderChat({
        thread: selectedThreadState(dbg),
        socket: socketState('connected'),
        chatRuntime,
      });

      expect(await screen.findByTestId('pending-approval-queue')).toBeInTheDocument();
    });

    it('renders the approval card for a normal thread', async () => {
      const normal = makeThread({ id: 'c-1', title: 'Normal thread' });
      mockGetThreads.mockResolvedValue({ threads: [normal], count: 1 });
      const chatRuntime = {
        ...chatRuntimeReducer(undefined, { type: '@@init' }),
        pendingApprovalByThread: { 'c-1': [approval] },
      };

      await renderChat({
        thread: selectedThreadState(normal),
        socket: socketState('connected'),
        chatRuntime,
      });

      expect(await screen.findByTestId('pending-approval-queue')).toBeInTheDocument();
    });

    it("does not show another thread's approval", async () => {
      const dbg = makeThread({ id: 'dbg-1', title: 'Debug thread', mode: 'debug' });
      mockGetThreads.mockResolvedValue({ threads: [dbg], count: 1 });
      const chatRuntime = {
        ...chatRuntimeReducer(undefined, { type: '@@init' }),
        pendingApprovalByThread: { other: [approval] },
      };

      await renderChat({
        thread: selectedThreadState(dbg),
        socket: socketState('connected'),
        chatRuntime,
      });

      expect(screen.queryByTestId('pending-approval-queue')).toBeNull();
    });
  });
});
