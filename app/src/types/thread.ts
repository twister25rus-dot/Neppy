/**
 * Per-thread operating mode. `chat` is one assistant with full tools;
 * `orchestration` is a supervisor coordinating specialist agents. Threads
 * created before modes existed carry no value and read as `chat`.
 */
export type ThreadMode = 'chat' | 'orchestration';

export const DEFAULT_THREAD_MODE: ThreadMode = 'chat';

/** Normalise a possibly-missing / unknown wire value to a valid mode. */
export function resolveThreadMode(value: unknown): ThreadMode {
  return value === 'orchestration' ? 'orchestration' : DEFAULT_THREAD_MODE;
}

export interface Thread {
  id: string;
  title: string;
  chatId: number | null;
  isActive: boolean;
  messageCount: number;
  lastMessageAt: string;
  createdAt: string;
  parentThreadId?: string;
  labels: string[];
  personalityId?: string | null;
  /** Absent on cores that predate modes; read through `resolveThreadMode`. */
  mode?: ThreadMode;
}

export interface ThreadSetModeData {
  thread: Thread;
  previousMode: ThreadMode;
  /** `false` when the thread was already in the requested mode (no-op). */
  changed: boolean;
}

export interface ThreadMessage {
  id: string;
  content: string;
  type: string;
  extraMetadata: Record<string, unknown>;
  sender: 'user' | 'agent';
  createdAt: string;
}

export interface ThreadsListData {
  threads: Thread[];
  count: number;
}

export interface ThreadMessagesData {
  messages: ThreadMessage[];
  count: number;
}

export interface ThreadDeleteData {
  deleted: boolean;
}

export interface PurgeResultData {
  messagesDeleted: number;
  agentThreadsDeleted: number;
  agentMessagesDeleted: number;
}
