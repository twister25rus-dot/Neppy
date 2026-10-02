import React from 'react';

import type { PendingApproval } from '../../store/chatRuntimeSlice';
import ApprovalRequestCard from './ApprovalRequestCard';
import IntegrationConnectCard from './IntegrationConnectCard';

interface Props {
  threadId: string;
  /** Parked requests for the thread, oldest first (the slice's queue order). */
  approvals: PendingApproval[];
}

/**
 * Renders every parked ApprovalGate request for a thread, newest first. The
 * core can park several concurrent approvals in one thread, and each card owns
 * its own decision and removes only itself from the queue. A single approval
 * renders exactly as before.
 *
 * `composio_connect` parks on the same gate but needs a Connect button + OAuth
 * poll rather than approve/deny (#3993). Cards are keyed by requestId so each
 * parked approval keeps its own local state (phase, field values, poll timers)
 * instead of inheriting another request's (#4062).
 */
const PendingApprovalQueue: React.FC<Props> = ({ threadId, approvals }) => {
  if (approvals.length === 0) return null;
  return (
    <div className="mb-2 flex flex-col gap-2" data-testid="pending-approval-queue">
      {[...approvals]
        .reverse()
        .map(approval =>
          approval.toolName === 'composio_connect' ? (
            <IntegrationConnectCard
              key={approval.requestId}
              threadId={threadId}
              approval={approval}
            />
          ) : (
            <ApprovalRequestCard key={approval.requestId} threadId={threadId} approval={approval} />
          )
        )}
    </div>
  );
};

export default PendingApprovalQueue;
