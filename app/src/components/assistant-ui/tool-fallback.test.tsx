import type { ToolCallMessagePartProps } from '@assistant-ui/react';
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import chatRuntimeReducer, { setPendingApprovalForThread } from '../../store/chatRuntimeSlice';
import threadReducer from '../../store/threadSlice';
import { ToolFallback } from './tool-fallback';
import { ToolGroupContent, ToolGroupRoot, ToolGroupTrigger } from './tool-group';

function renderTool(over: Partial<ToolCallMessagePartProps> = {}) {
  const props = {
    type: 'tool-call',
    toolName: 'shell',
    toolCallId: 'call-1',
    args: {},
    argsText: '{}',
    status: { type: 'complete' },
    addResult: () => {},
    resume: () => {},
    respondToApproval: () => {},
    ...over,
  } as unknown as ToolCallMessagePartProps;
  const Fallback = ToolFallback as unknown as React.FC<ToolCallMessagePartProps>;
  return render(<Fallback {...props} />);
}

const rowButton = () => screen.getByRole('button', { name: /Used tool/ });
const chevron = () =>
  rowButton().querySelector('[data-slot="tool-fallback-trigger-chevron"]') as SVGElement;

describe('ToolFallback disclosure', () => {
  it('toggles on click with aria-expanded and a chevron that follows the state', () => {
    renderTool({ result: 'hello out' });
    expect(rowButton()).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('hello out')).not.toBeInTheDocument();
    // Closed: pointing right, open: the class that clears the rotation keys off
    // Radix's `data-state` (the Base UI `data-open` attribute is never set).
    expect(chevron().getAttribute('class')).toContain('-rotate-90');
    expect(chevron().getAttribute('class')).toContain('group-data-[state=open]/trigger:rotate-0');
    expect(rowButton()).toHaveAttribute('data-state', 'closed');

    fireEvent.click(rowButton());
    expect(rowButton()).toHaveAttribute('aria-expanded', 'true');
    expect(rowButton()).toHaveAttribute('data-state', 'open');
    expect(screen.getByText('hello out')).toBeInTheDocument();

    fireEvent.click(rowButton());
    expect(rowButton()).toHaveAttribute('aria-expanded', 'false');
  });

  it('is a real button, so Enter and Space reach it natively', () => {
    renderTool({ result: 'x' });
    expect(rowButton().tagName).toBe('BUTTON');
    expect(rowButton()).toHaveAttribute('type', 'button');
  });

  it('shows arguments and the output of a completed tool event', () => {
    renderTool({
      args: { command: 'git status' },
      argsText: JSON.stringify({ command: 'git status' }, null, 2),
      result: 'On branch main\nnothing to commit',
    });
    fireEvent.click(rowButton());
    expect(screen.getByText('Arguments')).toBeInTheDocument();
    expect(screen.getByText(/"command": "git status"/)).toBeInTheDocument();
    expect(screen.getByText('Output')).toBeInTheDocument();
    expect(screen.getByText(/On branch main/)).toBeInTheDocument();
  });

  it('says "No output" for a finished tool that returned nothing', () => {
    renderTool({ result: '' });
    fireEvent.click(rowButton());
    expect(screen.getByText('No output')).toBeInTheDocument();
  });

  it('renders a failure as an error with its text', () => {
    renderTool({ result: 'permission denied', isError: true });
    fireEvent.click(rowButton());
    expect(screen.getByText('Error')).toBeInTheDocument();
    expect(screen.getByText('permission denied')).toBeInTheDocument();
    expect(screen.queryByText('Output')).not.toBeInTheDocument();
  });

  it('shows Running... while the call has no result yet', () => {
    renderTool({ status: { type: 'running' }, result: undefined });
    fireEvent.click(rowButton());
    expect(screen.getByRole('status')).toHaveTextContent('Running…');
    expect(screen.queryByText('No output')).not.toBeInTheDocument();
  });

  it('truncates long output behind a show more control', () => {
    const long = `${'a'.repeat(4000)}TAILMARK`;
    renderTool({ result: long });
    fireEvent.click(rowButton());
    expect(screen.queryByText(/TAILMARK/)).not.toBeInTheDocument();
    const more = screen.getByRole('button', { name: 'Show more' });
    expect(more).toHaveAttribute('aria-expanded', 'false');

    fireEvent.click(more);
    expect(screen.getByText(/TAILMARK/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Show less' })).toHaveAttribute(
      'aria-expanded',
      'true'
    );
  });
});

describe('ToolGroup header', () => {
  it('toggles its list with a chevron that follows the state', () => {
    render(
      <ToolGroupRoot variant="ghost">
        <ToolGroupTrigger count={3} />
        <ToolGroupContent>
          <span>row contents</span>
        </ToolGroupContent>
      </ToolGroupRoot>
    );
    const header = screen.getByRole('button', { name: '3 tool calls' });
    expect(header).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('row contents')).not.toBeInTheDocument();
    const chevron = header.querySelector('[data-slot="tool-group-trigger-chevron"]');
    expect(chevron?.getAttribute('class')).toContain('group-data-[state=open]/trigger:rotate-0');

    fireEvent.click(header);
    expect(header).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('row contents')).toBeInTheDocument();
  });

  it('uses the singular label for one call', () => {
    render(
      <ToolGroupRoot>
        <ToolGroupTrigger count={1} />
      </ToolGroupRoot>
    );
    expect(screen.getByRole('button', { name: '1 tool call' })).toBeInTheDocument();
  });
});

describe('ToolFallback approval states', () => {
  function renderWithApproval(
    toolName: string,
    approvals: Array<{ toolName: string; requestId: string }>
  ) {
    const store = configureStore({
      reducer: combineReducers({ thread: threadReducer, chatRuntime: chatRuntimeReducer }),
      preloadedState: {
        thread: { ...threadReducer(undefined, { type: '@@init' }), selectedThreadId: 't-1' },
      } as never,
    });
    for (const a of approvals) {
      store.dispatch(
        setPendingApprovalForThread({
          threadId: 't-1',
          approval: { requestId: a.requestId, toolName: a.toolName, message: 'm' },
        })
      );
    }
    const props = {
      type: 'tool-call',
      toolName,
      toolCallId: 'call-1',
      args: { command: 'ls' },
      argsText: '{}',
      status: { type: 'running' },
      addResult: () => {},
      resume: () => {},
      respondToApproval: () => {},
    } as unknown as ToolCallMessagePartProps;
    const Fallback = ToolFallback as unknown as React.FC<ToolCallMessagePartProps>;
    const view = render(
      <Provider store={store}>
        <Fallback {...props} />
      </Provider>
    );
    return { store, ...view };
  }

  it('shows "Waiting for your approval" in amber, with no spinner, for the parked call', () => {
    renderWithApproval('shell', [{ toolName: 'shell', requestId: 'r1' }]);
    const row = screen.getByRole('button', { name: /Waiting for your approval/ });
    const icon = row.querySelector('[data-slot="tool-fallback-trigger-icon"]') as SVGElement;
    expect(icon.getAttribute('class')).toContain('text-amber-500');
    expect(icon.getAttribute('class')).not.toContain('animate-spin');
    expect(row.querySelector('[data-slot="tool-fallback-trigger-shimmer"]')).toBeNull();
    expect(
      row
        .querySelector('[data-slot="tool-fallback-trigger-label"]')
        ?.getAttribute('data-approval-state')
    ).toBe('waiting');
  });

  it('shows "Queued" for a running call behind a parked call of another tool', () => {
    renderWithApproval('read_file', [{ toolName: 'shell', requestId: 'r1' }]);
    const row = screen.getByRole('button', { name: /Queued/ });
    const icon = row.querySelector('[data-slot="tool-fallback-trigger-icon"]') as SVGElement;
    expect(icon.getAttribute('class')).not.toContain('animate-spin');
  });

  it('keeps the normal running spinner when nothing is parked, and returns to it once answered', () => {
    const { store } = renderWithApproval('shell', []);
    const row = screen.getByRole('button', { name: /Used tool/ });
    expect(
      row.querySelector('[data-slot="tool-fallback-trigger-icon"]')?.getAttribute('class')
    ).toContain('animate-spin');

    act(() => {
      store.dispatch(
        setPendingApprovalForThread({
          threadId: 't-1',
          approval: { requestId: 'r9', toolName: 'shell', message: 'm' },
        })
      );
    });
    expect(screen.getByRole('button', { name: /Waiting for your approval/ })).toBeInTheDocument();
  });
});
