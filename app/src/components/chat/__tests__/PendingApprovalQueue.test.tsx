import { configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { callCoreRpc } from '../../../services/coreRpcClient';
import chatRuntimeReducer, {
  type PendingApproval,
  setPendingApprovalForThread,
} from '../../../store/chatRuntimeSlice';
import PendingApprovalQueue from '../PendingApprovalQueue';

vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

const THREAD = 't1';
const first: PendingApproval = {
  requestId: 'req-1',
  toolName: 'shell',
  message: 'Run first',
  command: 'ls first',
};
const second: PendingApproval = {
  requestId: 'req-2',
  toolName: 'file_write',
  message: 'Run second',
  command: 'ls second',
};

function QueueFromStore({ store }: { store: ReturnType<typeof makeStore> }) {
  const approvals = store.getState().chatRuntime.pendingApprovalByThread[THREAD] ?? [];
  return <PendingApprovalQueue threadId={THREAD} approvals={approvals} />;
}

function makeStore() {
  return configureStore({ reducer: { chatRuntime: chatRuntimeReducer } });
}

function renderQueue(approvals: PendingApproval[]) {
  const store = makeStore();
  for (const approval of approvals) {
    store.dispatch(setPendingApprovalForThread({ threadId: THREAD, approval }));
  }
  const utils = render(
    <Provider store={store}>
      <QueueFromStore store={store} />
    </Provider>
  );
  const rerenderQueue = () =>
    utils.rerender(
      <Provider store={store}>
        <QueueFromStore store={store} />
      </Provider>
    );
  return { store, rerenderQueue };
}

describe('PendingApprovalQueue', () => {
  beforeEach(() => {
    vi.mocked(callCoreRpc).mockReset();
  });

  it('renders nothing for an empty queue', () => {
    const { container } = render(
      <Provider store={makeStore()}>
        <PendingApprovalQueue threadId={THREAD} approvals={[]} />
      </Provider>
    );
    expect(container).toBeEmptyDOMElement();
  });

  it('renders a single approval like before', () => {
    renderQueue([first]);
    expect(screen.getAllByRole('alertdialog')).toHaveLength(1);
    expect(screen.getByText('ls first')).toBeInTheDocument();
  });

  it('renders every concurrent approval, newest first', () => {
    renderQueue([first, second]);
    const dialogs = screen.getAllByRole('alertdialog');
    expect(dialogs).toHaveLength(2);
    expect(dialogs[0]).toHaveTextContent('ls second');
    expect(dialogs[1]).toHaveTextContent('ls first');
  });

  it('decides each approval independently and removes only its own card', async () => {
    vi.mocked(callCoreRpc).mockResolvedValue({});
    const { store, rerenderQueue } = renderQueue([first, second]);

    // Deny the newer one (top card).
    const topDeny = screen.getAllByText('Deny')[0];
    fireEvent.click(topDeny);
    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.approval_decide',
        params: { request_id: 'req-2', decision: 'deny' },
      });
    });
    await waitFor(() => {
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toEqual([first]);
    });
    rerenderQueue();
    expect(screen.getAllByRole('alertdialog')).toHaveLength(1);

    // The earlier approval is still decidable.
    fireEvent.click(screen.getByText('Approve'));
    await waitFor(() => {
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.approval_decide',
        params: { request_id: 'req-1', decision: 'approve_once' },
      });
    });
    await waitFor(() => {
      expect(store.getState().chatRuntime.pendingApprovalByThread[THREAD]).toBeUndefined();
    });
  });
});
