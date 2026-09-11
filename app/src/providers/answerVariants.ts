import type { ThreadMessage } from '../types/thread';

/**
 * Answer variants on the display side.
 *
 * Regenerating keeps the previous answer, so a question can have several. The
 * transcript shows one of them and a switcher moves between them; the core
 * applies the same rule when it seeds the model, so what the model reads is
 * what is on screen.
 *
 * These rules are deliberately duplicated from `memory::conversations::variants`
 * rather than fetched: this is a pure projection of messages Redux already
 * holds, and a round trip per render to learn which of two answers to draw
 * would be absurd. The metadata keys are the contract between the two, so keep
 * the names and the precedence in step — newest wins absent a choice, and a
 * choice naming a message that is gone falls back rather than blanking.
 */

/** On an assistant message: the id of the user message it answers. */
export const VARIANT_OF = 'variantOf';
/** On an assistant message: the turn that produced it. */
export const VARIANT_TURN = 'variantTurn';
/** On a user message: which answer is the chosen one, by turn id. */
export const ACTIVE_VARIANT = 'activeVariant';

function metaString(message: ThreadMessage, key: string): string | undefined {
  const value = message.extraMetadata?.[key];
  if (typeof value !== 'string') return undefined;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : undefined;
}

/** The question this message answers, when it is a variant. */
export function variantOf(message: ThreadMessage): string | undefined {
  return metaString(message, VARIANT_OF);
}

/**
 * The turn that produced this message, falling back to its own id.
 *
 * The unit of an answer is the turn: one turn can append several assistant
 * messages, and all of them belong to the same answer.
 */
export function variantTurn(message: ThreadMessage): string {
  return metaString(message, VARIANT_TURN) ?? message.id;
}

/** The distinct answers to a question, as turn ids in the order produced. */
export function variantTurnsFor(messages: readonly ThreadMessage[], questionId: string): string[] {
  const turns: string[] = [];
  for (const message of messages) {
    if (variantOf(message) !== questionId) continue;
    const turn = variantTurn(message);
    if (!turns.includes(turn)) turns.push(turn);
  }
  return turns;
}

/** The turn id of the answer in effect for a question. */
export function activeTurnFor(
  messages: readonly ThreadMessage[],
  questionId: string
): string | undefined {
  const turns = variantTurnsFor(messages, questionId);
  if (turns.length === 0) return undefined;
  const chosen = messages.find(m => m.id === questionId && m.sender === 'user');
  const choice = chosen ? metaString(chosen, ACTIVE_VARIANT) : undefined;
  if (choice && turns.includes(choice)) return choice;
  return turns[turns.length - 1];
}

/** Whether this message belongs to the answer currently in effect. */
export function isActiveAnswer(
  messages: readonly ThreadMessage[],
  message: ThreadMessage
): boolean {
  const question = variantOf(message);
  if (!question) return true;
  return activeTurnFor(messages, question) === variantTurn(message);
}

/**
 * Where this answer sits among its siblings, for the switcher. `null` when the
 * question has only one answer — there is nothing to switch between, and a
 * "1/1" control is noise.
 */
export function answerPosition(
  messages: readonly ThreadMessage[],
  message: ThreadMessage
): { questionId: string; turns: string[]; index: number; count: number } | null {
  const questionId = variantOf(message);
  if (!questionId) return null;
  const turns = variantTurnsFor(messages, questionId);
  if (turns.length < 2) return null;
  const index = turns.indexOf(variantTurn(message));
  if (index < 0) return null;
  return { questionId, turns, index, count: turns.length };
}
