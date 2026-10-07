import { describe, expect, it } from 'vitest';

import type { Thread } from '../../../types/thread';
import {
  DEBUG_TAB_VALUE,
  GENERAL_TAB_VALUE,
  isDebugThread,
  isThreadVisibleInTab,
  SUBCONSCIOUS_TAB_VALUE,
  TASKS_TAB_VALUE,
} from './threadFilter';

function thread(overrides: Partial<Thread>): Thread {
  return {
    id: overrides.id ?? 't',
    title: overrides.title ?? 'Untitled',
    chatId: null,
    isActive: true,
    messageCount: 0,
    lastMessageAt: '2026-05-15T10:00:00Z',
    createdAt: '2026-05-15T09:00:00Z',
    parentThreadId: overrides.parentThreadId,
    labels: overrides.labels ?? [],
  };
}

describe('Debug threads', () => {
  const byMode = { ...thread({ id: 'd1' }), mode: 'debug' as const };
  const byLabel = thread({ id: 'd2', labels: ['mode:debug'] });
  const normal = thread({ id: 'c1' });

  it('is recognised by mode or by the core mode label', () => {
    expect(isDebugThread(byMode)).toBe(true);
    expect(isDebugThread(byLabel)).toBe(true);
    expect(isDebugThread(normal)).toBe(false);
  });

  it('is hidden from every normal tab', () => {
    for (const tab of [GENERAL_TAB_VALUE, TASKS_TAB_VALUE, SUBCONSCIOUS_TAB_VALUE, 'mode:debug']) {
      expect(isThreadVisibleInTab(byMode, tab)).toBe(false);
      expect(isThreadVisibleInTab(byLabel, tab)).toBe(false);
    }
    expect(isThreadVisibleInTab(normal, GENERAL_TAB_VALUE)).toBe(true);
  });

  it('is the only thing the Debug pseudo-tab shows', () => {
    expect(isThreadVisibleInTab(byMode, DEBUG_TAB_VALUE)).toBe(true);
    expect(isThreadVisibleInTab(byLabel, DEBUG_TAB_VALUE)).toBe(true);
    expect(isThreadVisibleInTab(normal, DEBUG_TAB_VALUE)).toBe(false);
  });
});

describe('isThreadVisibleInTab', () => {
  describe('General bucket', () => {
    it('keeps general and legacy work-labeled threads', () => {
      expect(isThreadVisibleInTab(thread({ labels: [GENERAL_TAB_VALUE] }), GENERAL_TAB_VALUE)).toBe(
        true
      );
      expect(isThreadVisibleInTab(thread({ labels: ['work', 'urgent'] }), GENERAL_TAB_VALUE)).toBe(
        true
      );
    });

    it('keeps unlabeled and unknown-label threads as the fallback bucket', () => {
      expect(isThreadVisibleInTab(thread({ labels: [] }), GENERAL_TAB_VALUE)).toBe(true);
      expect(isThreadVisibleInTab(thread({ labels: ['briefing'] }), GENERAL_TAB_VALUE)).toBe(true);
      expect(isThreadVisibleInTab(thread({ labels: ['notification'] }), GENERAL_TAB_VALUE)).toBe(
        true
      );
      expect(isThreadVisibleInTab(thread({ labels: ['custom'] }), GENERAL_TAB_VALUE)).toBe(true);
    });

    it('excludes threads that belong to explicit non-General buckets', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [SUBCONSCIOUS_TAB_VALUE] }), GENERAL_TAB_VALUE)
      ).toBe(false);
      expect(isThreadVisibleInTab(thread({ labels: [TASKS_TAB_VALUE] }), GENERAL_TAB_VALUE)).toBe(
        false
      );
      expect(isThreadVisibleInTab(thread({ parentThreadId: 'parent' }), GENERAL_TAB_VALUE)).toBe(
        false
      );
    });

    it('excludes meeting threads from the General bucket (folded into Tasks)', () => {
      expect(isThreadVisibleInTab(thread({ labels: ['meetings'] }), GENERAL_TAB_VALUE)).toBe(false);
      expect(isThreadVisibleInTab(thread({ labels: ['Meetings'] }), GENERAL_TAB_VALUE)).toBe(false);
    });
  });

  describe('Subconscious bucket', () => {
    it('keeps canonical and legacy subconscious-generated threads', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [SUBCONSCIOUS_TAB_VALUE] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(true);
      expect(
        isThreadVisibleInTab(thread({ labels: ['from_reflection'] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(true);
      expect(
        isThreadVisibleInTab(thread({ labels: ['subconscious_tick'] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(true);
    });

    it('excludes ordinary and task threads', () => {
      expect(
        isThreadVisibleInTab(thread({ labels: [GENERAL_TAB_VALUE] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(false);
      expect(
        isThreadVisibleInTab(thread({ labels: [TASKS_TAB_VALUE] }), SUBCONSCIOUS_TAB_VALUE)
      ).toBe(false);
    });
  });

  describe('Tasks bucket', () => {
    it('keeps task-board, legacy agent-task, and legacy worker-labeled threads', () => {
      expect(isThreadVisibleInTab(thread({ labels: [TASKS_TAB_VALUE] }), TASKS_TAB_VALUE)).toBe(
        true
      );
      expect(isThreadVisibleInTab(thread({ labels: ['agent-task'] }), TASKS_TAB_VALUE)).toBe(true);
      expect(isThreadVisibleInTab(thread({ labels: ['worker'] }), TASKS_TAB_VALUE)).toBe(true);
    });

    it('keeps parented worker/sub-agent threads regardless of labels', () => {
      expect(
        isThreadVisibleInTab(
          thread({ parentThreadId: 'parent', labels: [GENERAL_TAB_VALUE] }),
          TASKS_TAB_VALUE
        )
      ).toBe(true);
    });

    it('keeps meeting-labeled threads (meetings folded into tasks)', () => {
      expect(isThreadVisibleInTab(thread({ labels: ['meetings'] }), TASKS_TAB_VALUE)).toBe(true);
      expect(isThreadVisibleInTab(thread({ labels: ['Meetings'] }), TASKS_TAB_VALUE)).toBe(true);
    });

    it('excludes ordinary and subconscious threads', () => {
      expect(isThreadVisibleInTab(thread({ labels: [GENERAL_TAB_VALUE] }), TASKS_TAB_VALUE)).toBe(
        false
      );
      expect(
        isThreadVisibleInTab(thread({ labels: [SUBCONSCIOUS_TAB_VALUE] }), TASKS_TAB_VALUE)
      ).toBe(false);
    });
  });
});
