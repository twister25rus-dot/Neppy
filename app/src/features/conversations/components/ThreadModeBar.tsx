import { useAppSelector } from '../../../store/hooks';
import { OrchestrationProgress } from './OrchestrationProgress';
import { ThreadModeToggle } from './ThreadModeToggle';

/**
 * Header slot above the composer: the Chat / Orchestration / Debug tabs for the
 * open conversation, and (Orchestration only) its execution-history strip. On a
 * new, empty thread the tabs are the large centred hero variant that sits
 * between the welcome heading and the composer; once messages exist they shrink
 * to a compact row so the transcript keeps the space.
 */
export function ThreadModeBar({
  threadId,
  onOpenAllRuns,
}: {
  threadId: string | null;
  onOpenAllRuns?: () => void;
}) {
  const isEmpty = useAppSelector(state =>
    threadId ? (state.thread.messagesByThreadId?.[threadId]?.length ?? 0) === 0 : true
  );
  if (!threadId) return null;
  return (
    <div className={isEmpty ? 'pb-6' : 'pb-2'} data-testid="thread-mode-bar">
      <div className={isEmpty ? 'flex justify-center' : 'mb-1.5 px-1'}>
        <ThreadModeToggle threadId={threadId} size={isEmpty ? 'lg' : 'sm'} />
      </div>
      <OrchestrationProgress threadId={threadId} onOpenAllRuns={onOpenAllRuns} />
    </div>
  );
}

export default ThreadModeBar;
