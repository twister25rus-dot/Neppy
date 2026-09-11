import { useState } from 'react';
import { LuChevronLeft, LuChevronRight } from 'react-icons/lu';

import { answerPosition } from '../../providers/answerVariants';
import { threadApi } from '../../services/api/threadApi';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import { loadThreadMessages } from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';

/** Stable empty array so the selector does not return a new one each render. */
const EMPTY_MESSAGES: ThreadMessage[] = [];

interface AnswerSwitcherProps {
  /** Id of the assistant message being drawn; resolved against the store. */
  messageId: string;
  className?: string;
}

/**
 * `‹ 2/3 ›` for a question that has been answered more than once.
 *
 * Renders nothing when there is only one answer — a permanent "1/1" is noise.
 *
 * Switching goes through the core rather than local state: choosing an answer
 * also evicts the thread's cached agent session, because a turn resumes from
 * the session it already holds. Without that the transcript would change and
 * the model would keep reasoning from the other answer.
 */
export default function AnswerSwitcher({ messageId, className }: AnswerSwitcherProps) {
  const dispatch = useAppDispatch();
  const threadId = useAppSelector(state => state.thread.selectedThreadId);
  const messages = useAppSelector(state =>
    state.thread.selectedThreadId
      ? (state.thread.messagesByThreadId[state.thread.selectedThreadId] ?? EMPTY_MESSAGES)
      : EMPTY_MESSAGES
  );
  const [switching, setSwitching] = useState(false);

  const message = messages.find(m => m.id === messageId);
  const position = message ? answerPosition(messages, message) : null;
  if (!position || !threadId) return null;

  const { questionId, turns, index, count } = position;

  const go = async (nextIndex: number) => {
    const turn = turns[nextIndex];
    if (!turn || switching) return;
    setSwitching(true);
    try {
      await threadApi.setActiveAnswer(threadId, questionId, turn);
      // Re-read rather than patching locally: the selection lives on the
      // question's metadata, and the store is the one that knows what was
      // written.
      await dispatch(loadThreadMessages(threadId));
    } finally {
      setSwitching(false);
    }
  };

  return (
    <div
      className={`flex items-center gap-0.5 text-xs text-content-muted ${className ?? ''}`}
      data-testid="answer-switcher">
      <button
        type="button"
        className="rounded p-0.5 hover:text-content disabled:opacity-40"
        aria-label="Previous answer"
        data-analytics-id="chat-answer-previous"
        disabled={index === 0 || switching}
        onClick={() => void go(index - 1)}>
        <LuChevronLeft className="h-3.5 w-3.5" />
      </button>
      <span aria-live="polite" data-testid="answer-switcher-position">
        {index + 1}/{count}
      </span>
      <button
        type="button"
        className="rounded p-0.5 hover:text-content disabled:opacity-40"
        aria-label="Next answer"
        data-analytics-id="chat-answer-next"
        disabled={index === count - 1 || switching}
        onClick={() => void go(index + 1)}>
        <LuChevronRight className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
