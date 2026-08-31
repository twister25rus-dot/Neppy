import { create } from "zustand";

/**
 * Client-local last-read tracking for surfaces the backend keeps no read state
 * for (channels, groups). The relay deletes direct messages on receipt and there
 * is no server-side read marker for channel/group messages, so "unread" here is
 * derived by comparing a surface's last-activity timestamp against the last time
 * the user viewed it. Keyed by channelId / groupId.
 */
type MessageReadsState = {
	/** ISO timestamp of when each surface was last viewed, keyed by id. */
	lastReadAt: Record<string, string>;
	/** Records that a surface was viewed at the given time (defaults to now). */
	markRead: (id: string, at?: string) => void;
	/** Clears all read positions (e.g. on wallet disconnect). */
	reset: () => void;
};

export const useMessageReadsStore = create<MessageReadsState>()((set) => ({
	lastReadAt: {},
	markRead: (id, at): void => {
		const timestamp = at ?? new Date().toISOString();
		set((state) => {
			const existing = state.lastReadAt[id];
			// Never move the marker backwards.
			if (existing && existing >= timestamp) {
				return state;
			}
			return { lastReadAt: { ...state.lastReadAt, [id]: timestamp } };
		});
	},
	reset: (): void => {
		set({ lastReadAt: {} });
	},
}));

/**
 * Whether a surface has activity the user has not seen: it has a last-activity
 * timestamp and either was never opened or has been active since it was opened.
 */
export function hasUnread(
	lastReadAt: Record<string, string>,
	id: string,
	lastActivityAt: string | undefined
): boolean {
	if (!lastActivityAt) {
		return false;
	}
	const seenAt = lastReadAt[id];
	return !seenAt || lastActivityAt > seenAt;
}
