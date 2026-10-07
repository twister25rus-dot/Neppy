import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { fireEvent, render, screen } from '@testing-library/react';
import { Provider } from 'react-redux';
import { describe, expect, it } from 'vitest';

import chatRuntimeReducer from '../../store/chatRuntimeSlice';
import themeReducer from '../../store/themeSlice';
import threadReducer from '../../store/threadSlice';
import { Thread } from './thread';

const store = () =>
  configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
  });

function Harness({ messages, isRunning }: { messages: ThreadMessageLike[]; isRunning?: boolean }) {
  const runtime = useExternalStoreRuntime({
    messages,
    isRunning: isRunning ?? false,
    convertMessage: (m: ThreadMessageLike) => m,
    onNew: async () => {},
  });
  return (
    <Provider store={store()}>
      <AssistantRuntimeProvider runtime={runtime}>
        <Thread />
      </AssistantRuntimeProvider>
    </Provider>
  );
}

const settled: ThreadMessageLike[] = [
  { id: 'u1', role: 'user', content: [{ type: 'text', text: 'why?' }] },
  {
    id: 'a1',
    role: 'assistant',
    content: [
      { type: 'reasoning', text: 'weighing the options' },
      { type: 'text', text: 'Because.' },
    ],
  },
];

const trigger = () => screen.getByRole('button', { name: /Reasoning/ });
const chevron = () =>
  trigger().querySelector('[data-slot="reasoning-trigger-chevron"]') as SVGElement;

describe('Reasoning block', () => {
  it('renders a collapsed "Reasoning" row for a settled answer and expands on click', () => {
    render(<Harness messages={settled} />);
    expect(trigger()).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('weighing the options')).not.toBeInTheDocument();
    expect(screen.getByText('Because.')).toBeInTheDocument();

    fireEvent.click(trigger());
    expect(trigger()).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('weighing the options')).toBeInTheDocument();
  });

  it('keys the chevron rotation and animations off Radix data-state, not Base UI data-open', () => {
    render(<Harness messages={settled} />);
    const cls = chevron().getAttribute('class') ?? '';
    expect(cls).toContain('-rotate-90');
    expect(cls).toContain('group-data-[state=open]/trigger:rotate-0');
    expect(cls).not.toMatch(/data-open|data-panel-open/);

    fireEvent.click(trigger());
    expect(trigger()).toHaveAttribute('data-state', 'open');
    const content = document.querySelector('[data-slot="reasoning-content"]') as HTMLElement;
    expect(content.className).toContain('data-[state=open]:animate-collapsible-down');
    expect(content.className).not.toMatch(/(^|\s)data-(open|closed):/);
    const text = document.querySelector('[data-slot="reasoning-text"]') as HTMLElement;
    expect(text.className).toContain('group-data-[state=open]/collapsible-content:animate-in');
    expect(text.className).not.toContain('group-data-open');
  });

  it('holds the block open while the reasoning part is streaming', async () => {
    const live: ThreadMessageLike[] = [
      { id: 'u1', role: 'user', content: [{ type: 'text', text: 'why?' }] },
      {
        id: 'a1',
        role: 'assistant',
        status: { type: 'running' },
        content: [{ type: 'reasoning', text: 'thinking live', status: { type: 'running' } }],
      },
    ];
    render(<Harness messages={live} isRunning />);
    expect(trigger()).toHaveAttribute('aria-expanded', 'true');
    expect(await screen.findByText(/thinking live/)).toBeInTheDocument();
  });
});
