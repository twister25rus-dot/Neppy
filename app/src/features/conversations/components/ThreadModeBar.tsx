import { OrchestrationProgress } from './OrchestrationProgress';
import { ThreadModeToggle } from './ThreadModeToggle';

/**
 * Header slot above the composer: the Chat / Orchestration switch for the open
 * conversation, and (Orchestration only) its execution-history strip.
 */
export function ThreadModeBar({
  threadId,
  onOpenAllRuns,
}: {
  threadId: string | null;
  onOpenAllRuns?: () => void;
}) {
  if (!threadId) return null;
  return (
    <div className="pb-2" data-testid="thread-mode-bar">
      <div className="mb-1.5 px-1">
        <ThreadModeToggle threadId={threadId} />
      </div>
      <OrchestrationProgress threadId={threadId} onOpenAllRuns={onOpenAllRuns} />
    </div>
  );
}

export default ThreadModeBar;
