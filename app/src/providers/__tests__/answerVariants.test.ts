import { describe, expect, it } from 'vitest';

import {
  ACTIVE_VARIANT,
  activeTurnFor,
  answerPosition,
  isActiveAnswer,
  VARIANT_OF,
  VARIANT_TURN,
  variantTurnsFor,
} from '../answerVariants';
import type { ThreadMessage } from '../../types/thread';

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

/** One question, two answers, the second from a regenerate. */
const twoAnswers = (): ThreadMessage[] => [
  msg('u1', 'user'),
  msg('a1', 'agent', { [VARIANT_OF]: 'u1', [VARIANT_TURN]: 'turn-1' }),
  msg('a2', 'agent', { [VARIANT_OF]: 'u1', [VARIANT_TURN]: 'turn-2' }),
];

describe('answer variants', () => {
  it('leaves an ordinary thread alone', () => {
    const messages = [msg('u1', 'user'), msg('a1', 'agent')];

    expect(messages.every(m => isActiveAnswer(messages, m))).toBe(true);
    expect(answerPosition(messages, messages[1]!)).toBeNull();
  });

  it('shows the newest answer until one is chosen', () => {
    const messages = twoAnswers();

    expect(activeTurnFor(messages, 'u1')).toBe('turn-2');
    expect(isActiveAnswer(messages, messages[1]!)).toBe(false);
    expect(isActiveAnswer(messages, messages[2]!)).toBe(true);
  });

  it('honours an explicit choice', () => {
    const messages = twoAnswers();
    messages[0] = msg('u1', 'user', { [ACTIVE_VARIANT]: 'turn-1' });

    expect(activeTurnFor(messages, 'u1')).toBe('turn-1');
    expect(isActiveAnswer(messages, messages[1]!)).toBe(true);
  });

  it('falls back when the choice names an answer that is gone', () => {
    const messages = twoAnswers();
    messages[0] = msg('u1', 'user', { [ACTIVE_VARIANT]: 'turn-gone' });

    expect(activeTurnFor(messages, 'u1')).toBe('turn-2');
  });

  it('counts a segmented answer once', () => {
    const messages = [
      msg('u1', 'user'),
      msg('s1', 'agent', { [VARIANT_OF]: 'u1', [VARIANT_TURN]: 'turn-1' }),
      msg('s2', 'agent', { [VARIANT_OF]: 'u1', [VARIANT_TURN]: 'turn-1' }),
      msg('r1', 'agent', { [VARIANT_OF]: 'u1', [VARIANT_TURN]: 'turn-2' }),
    ];

    expect(variantTurnsFor(messages, 'u1')).toEqual(['turn-1', 'turn-2']);
    expect(answerPosition(messages, messages[1]!)).toMatchObject({ index: 0, count: 2 });
    // Every segment of the chosen answer stays visible together.
    messages[0] = msg('u1', 'user', { [ACTIVE_VARIANT]: 'turn-1' });
    expect(messages.filter(m => isActiveAnswer(messages, m)).map(m => m.id)).toEqual([
      'u1',
      's1',
      's2',
    ]);
  });

  it('offers no switcher for a question with a single answer', () => {
    const messages = [msg('u1', 'user'), msg('a1', 'agent', { [VARIANT_OF]: 'u1' })];

    expect(answerPosition(messages, messages[1]!)).toBeNull();
  });

  it('reports position and count for the switcher', () => {
    const messages = twoAnswers();

    expect(answerPosition(messages, messages[2]!)).toMatchObject({
      questionId: 'u1',
      index: 1,
      count: 2,
    });
  });
});
