import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import { store } from '../../../../store';
import type { SubagentActivity, ToolTimelineEntry } from '../../../../store/chatRuntimeSlice';
import type { ThreadMode } from '../../../../types/thread';
import { ThreadModeProvider } from '../../threadModeContext';
import { SubagentActivityDisclosure } from '../SubagentActivityBlock';
import { ToolTimelineBlock } from '../ToolTimelineBlock';

const subagent: SubagentActivity = { taskId: 't1', agentId: 'researcher', toolCalls: [] };

function entry(status: ToolTimelineEntry['status']): ToolTimelineEntry {
  return { id: 'sub-1', name: 'subagent:researcher', round: 1, seq: 0, status, subagent };
}

function renderBlock(mode: ThreadMode | null, status: ToolTimelineEntry['status']) {
  const block = <ToolTimelineBlock entries={[entry(status)]} turnActive={status === 'running'} />;
  return render(
    <Provider store={store}>
      {mode ? <ThreadModeProvider mode={mode}>{block}</ThreadModeProvider> : block}
    </Provider>
  );
}

/** The disclosure that wraps the sub-agent activity body. */
function rowState(): string | null {
  const body = screen.getByTestId('subagent-activity');
  return body.closest('[data-state]')?.getAttribute('data-state') ?? null;
}

describe('sub-agent detail by thread mode', () => {
  it('keeps a running sub-agent collapsed in Chat mode (one assistant)', () => {
    renderBlock('chat', 'running');
    expect(rowState()).toBe('closed');
  });

  it('expands a running sub-agent in Orchestration mode', () => {
    renderBlock('orchestration', 'running');
    expect(rowState()).toBe('open');
  });

  it('keeps a settled sub-agent expanded in Orchestration mode, collapsed in Chat', () => {
    const { unmount } = renderBlock('orchestration', 'success');
    expect(rowState()).toBe('open');
    unmount();
    renderBlock('chat', 'success');
    expect(rowState()).toBe('closed');
  });

  it('leaves surfaces with no conversation mode on their original behaviour', () => {
    renderBlock(null, 'running');
    // Latest running row auto-expands, as before modes existed.
    expect(rowState()).toBe('open');
  });

  it('lets the user open collapsed detail in Chat mode', async () => {
    renderBlock('chat', 'running');
    const trigger = screen
      .getAllByRole('button')
      .find(b => b.getAttribute('aria-expanded') === 'false');
    expect(trigger).toBeDefined();
    await userEvent.click(trigger!);
    expect(rowState()).toBe('open');
  });
});

describe('SubagentActivityDisclosure', () => {
  const renderDisclosure = (mode: ThreadMode | null) =>
    render(
      <Provider store={store}>
        <SubagentActivityDisclosure subagent={subagent} mode={mode} />
      </Provider>
    );

  it('wraps detail in a closed disclosure in Chat mode', () => {
    renderDisclosure('chat');
    const details = screen.getByTestId('subagent-disclosure') as HTMLDetailsElement;
    expect(details.open).toBe(false);
    expect(screen.getByText('Helper details')).toBeInTheDocument();
  });

  it.each([['orchestration' as const], [null]])('renders detail directly for %s', mode => {
    renderDisclosure(mode);
    expect(screen.queryByTestId('subagent-disclosure')).toBeNull();
    expect(screen.getByTestId('subagent-activity')).toBeInTheDocument();
  });
});
