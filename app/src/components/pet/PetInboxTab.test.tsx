import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { makeInbox, makeProposal } from './petFixtures';
import PetInboxTab from './PetInboxTab';

const mockNavigate = vi.fn();
const mockDecideProposal = vi.fn();
const mockDecideApproval = vi.fn();
const mockCreateThread = vi.fn();
const mockGetThreads = vi.fn();

vi.mock('react-router-dom', async importOriginal => {
  const actual = await importOriginal<typeof import('react-router-dom')>();
  return { ...actual, useNavigate: () => mockNavigate };
});
vi.mock('../../services/api/petApi', () => ({
  decidePetProposal: (...args: unknown[]) => mockDecideProposal(...args),
}));
vi.mock('../../services/api/approvalApi', () => ({
  decideApproval: (...args: unknown[]) => mockDecideApproval(...args),
}));
vi.mock('../../services/api/threadApi', () => ({
  threadApi: {
    createNewThread: (...args: unknown[]) => mockCreateThread(...args),
    getThreads: (...args: unknown[]) => mockGetThreads(...args),
  },
}));

const approval = {
  request_id: 'req-1',
  tool_name: 'composio_execute',
  action_summary: 'Send a reply',
  args_redacted: {},
  session_id: 's1',
  created_at: '2020-01-01T09:00:00Z',
  expires_at: '2099-01-01T09:10:00Z',
};

describe('PetInboxTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCreateThread.mockResolvedValue({ id: 'thread-9', labels: [] });
    mockGetThreads.mockResolvedValue({ threads: [], count: 0 });
  });

  it('shows the empty state when nothing is waiting', () => {
    renderWithProviders(<PetInboxTab inbox={makeInbox()} onChanged={vi.fn()} />);
    expect(screen.getByTestId('pet-inbox-empty')).toBeInTheDocument();
  });

  it('accepting a proposal creates a chat and seeds the composer without sending', async () => {
    mockDecideProposal.mockResolvedValue({
      proposal: makeProposal({ state: 'accepted' }),
      chat_prompt: 'My Pet suggested this: reply to mentor',
    });
    const onChanged = vi.fn();
    renderWithProviders(
      <PetInboxTab inbox={makeInbox({ proposals: [makeProposal()] })} onChanged={onChanged} />
    );

    fireEvent.click(screen.getByTestId('pet-proposal-accept'));

    await waitFor(() => expect(mockNavigate).toHaveBeenCalled());
    expect(mockDecideProposal).toHaveBeenCalledWith('prop-1', 'accept');
    expect(mockNavigate).toHaveBeenCalledWith('/chat/thread-9', {
      state: { openThreadId: 'thread-9', composerSeed: 'My Pet suggested this: reply to mentor' },
    });
    expect(onChanged).toHaveBeenCalled();
  });

  it('shows an error and does not navigate when the decision fails', async () => {
    mockDecideProposal.mockRejectedValue(new Error('boom'));
    renderWithProviders(
      <PetInboxTab inbox={makeInbox({ proposals: [makeProposal()] })} onChanged={vi.fn()} />
    );
    fireEvent.click(screen.getByTestId('pet-proposal-accept'));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not record that decision');
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it('dismissing a proposal calls the dismiss decision', async () => {
    mockDecideProposal.mockResolvedValue({ proposal: makeProposal(), chat_prompt: null });
    const onChanged = vi.fn();
    renderWithProviders(
      <PetInboxTab inbox={makeInbox({ proposals: [makeProposal()] })} onChanged={onChanged} />
    );
    fireEvent.click(screen.getByTestId('pet-proposal-dismiss'));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(mockDecideProposal).toHaveBeenCalledWith('prop-1', 'dismiss');
    expect(mockNavigate).not.toHaveBeenCalled();
  });

  it('approve once and deny go through approval_decide', async () => {
    mockDecideApproval.mockResolvedValue(undefined);
    const onChanged = vi.fn();
    renderWithProviders(
      <PetInboxTab inbox={makeInbox({ approvals: [approval] })} onChanged={onChanged} />
    );
    expect(screen.getByText('Send a reply')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('pet-approval-approve'));
    await waitFor(() => expect(mockDecideApproval).toHaveBeenCalledWith('req-1', 'approve_once'));

    fireEvent.click(screen.getByTestId('pet-approval-deny'));
    await waitFor(() => expect(mockDecideApproval).toHaveBeenCalledWith('req-1', 'deny'));
  });

  it('ignores proposals that are no longer pending', () => {
    renderWithProviders(
      <PetInboxTab
        inbox={makeInbox({ proposals: [makeProposal({ state: 'dismissed' })] })}
        onChanged={vi.fn()}
      />
    );
    expect(screen.getByTestId('pet-inbox-empty')).toBeInTheDocument();
  });
});
