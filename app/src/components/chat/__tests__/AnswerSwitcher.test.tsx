import { fireEvent, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import type { ThreadMessage } from '../../../types/thread';
import AnswerSwitcher from '../AnswerSwitcher';

const setActiveAnswer = vi.fn().mockResolvedValue(undefined);
vi.mock('../../../services/api/threadApi', () => ({
  threadApi: {
    setActiveAnswer: (...args: unknown[]) => setActiveAnswer(...args),
    getThreadMessages: vi.fn().mockResolvedValue({ messages: [] }),
  },
}));

const msg = (
  id: string,
  sender: ThreadMessage['sender'],
  extraMetadata: Record<string, unknown> = {}
): ThreadMessage => ({
  id,
  content: `content ${id}`,
  type: 'text',
  extraMetadata,
  sender,
  createdAt: '2026-09-11T00:00:00Z',
});

const withMessages = (messages: ThreadMessage[]) => ({
  preloadedState: {
    thread: { selectedThreadId: 't1', messagesByThreadId: { t1: messages } },
  } as never,
});

describe('AnswerSwitcher', () => {
  it('stays out of the way when a question has one answer', () => {
    const messages = [msg('u1', 'user'), msg('a1', 'agent', { variantOf: 'u1' })];

    renderWithProviders(<AnswerSwitcher messageId="a1" />, withMessages(messages));

    expect(screen.queryByTestId('answer-switcher')).not.toBeInTheDocument();
  });

  it('shows the position among the answers', () => {
    const messages = [
      msg('u1', 'user'),
      msg('a1', 'agent', { variantOf: 'u1', variantTurn: 'turn-1' }),
      msg('a2', 'agent', { variantOf: 'u1', variantTurn: 'turn-2' }),
    ];

    renderWithProviders(<AnswerSwitcher messageId="a2" />, withMessages(messages));

    expect(screen.getByTestId('answer-switcher-position')).toHaveTextContent('2/2');
    expect(screen.getByLabelText('Next answer')).toBeDisabled();
  });

  it('switches through the core, which also evicts the cached session', async () => {
    const messages = [
      msg('u1', 'user'),
      msg('a1', 'agent', { variantOf: 'u1', variantTurn: 'turn-1' }),
      msg('a2', 'agent', { variantOf: 'u1', variantTurn: 'turn-2' }),
    ];

    renderWithProviders(<AnswerSwitcher messageId="a2" />, withMessages(messages));
    fireEvent.click(screen.getByLabelText('Previous answer'));

    await waitFor(() => {
      expect(setActiveAnswer).toHaveBeenCalledWith('t1', 'u1', 'turn-1');
    });
  });
});
