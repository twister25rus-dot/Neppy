import { createContext, type ReactNode, useContext } from 'react';

import { useAppSelector } from '../../store/hooks';
import { resolveThreadMode, type ThreadMode } from '../../types/thread';

/**
 * The Chat / Orchestration mode of the thread the transcript below is showing.
 *
 * `null` means "no provider": surfaces that render the timeline outside a
 * conversation (the workflow copilot panel, dev previews) keep their original
 * sub-agent behaviour. Inside a conversation the value drives how much
 * sub-agent detail is shown by default — collapsed in Chat (the user sees one
 * assistant), expanded in Orchestration (the user is supervising a team).
 */
const ThreadModeContext = createContext<ThreadMode | null>(null);

export function ThreadModeProvider({ mode, children }: { mode: ThreadMode; children: ReactNode }) {
  return <ThreadModeContext.Provider value={mode}>{children}</ThreadModeContext.Provider>;
}

/** Provider that follows the selected thread's persisted mode in Redux. */
export function SelectedThreadModeProvider({ children }: { children: ReactNode }) {
  const mode = useAppSelector(state => {
    const id = state.thread.selectedThreadId;
    return resolveThreadMode(id ? state.thread.threads.find(t => t.id === id)?.mode : undefined);
  });
  return <ThreadModeProvider mode={mode}>{children}</ThreadModeProvider>;
}

export function useThreadModeContext(): ThreadMode | null {
  return useContext(ThreadModeContext);
}
