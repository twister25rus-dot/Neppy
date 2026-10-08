/**
 * Editing in the MIDDLE of the composer text must keep the caret where the user
 * is typing. Regression: every native keystroke threw the caret to the end.
 *
 * The browser inserts plain text natively and Lexical learns about it from a
 * MutationObserver; `Thread`'s `onInputCapture` mirrored `textContent` into the
 * composer runtime from a *microtask*, which runs before Lexical's own update
 * listener has told `SyncPlugin` about the change. `SyncPlugin` then saw a
 * runtime write it did not originate and rebuilt the editor (`root.selectEnd()`).
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { act, render, screen } from '@testing-library/react';
import { useEffect, useState } from 'react';
import { Provider } from 'react-redux';
import { describe, expect, it, vi } from 'vitest';

import chatRuntimeReducer from '../../../store/chatRuntimeSlice';
import themeReducer from '../../../store/themeSlice';
import threadReducer from '../../../store/threadSlice';
import { AssistantUiChat } from './AssistantUiChat';

vi.mock('../../../lib/commands/useRegisterAction', () => ({ useRegisterAction: vi.fn() }));
vi.mock('../../../lib/commands/useSlashCommands', () => ({ useSlashCommands: () => [] }));
vi.mock('./ThreadGoalChip', () => ({
  ThreadGoalEditorPanel: () => null,
  ThreadGoalFooterTrigger: () => null,
}));
vi.mock('../../../components/assistant-ui/model-selector', () => ({
  ModelQualityPill: () => null,
}));

const hostValue = { current: '' };

function Host() {
  const [value, setValue] = useState('');
  useEffect(() => {
    hostValue.current = value;
  }, [value]);
  return (
    <AssistantUiChat
      threadGoal={{ open: vi.fn() } as never}
      model={null}
      onModelChange={vi.fn()}
      sampling={{ effort: null } as never}
      onSamplingChange={vi.fn()}
      inputValue={value}
      onInputValueChange={setValue}
      attachments={[]}
      onAttachFiles={vi.fn().mockResolvedValue(undefined)}
      onRemoveAttachment={vi.fn()}
      maxAttachments={4}
      attachmentsEnabled={false}
      attachmentInteractionBlocked={false}
      onAttachmentOnlySend={vi.fn()}
      preset="auto"
      onPresetChange={vi.fn()}
    />
  );
}

function buildStore() {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
  });
}

const flush = () => act(async () => new Promise<void>(resolve => setTimeout(resolve, 0)));

describe('composer caret', () => {
  it('keeps the caret in the middle of the text after a native insertion', async () => {
    render(
      <Provider store={buildStore()}>
        <Host />
      </Provider>
    );
    const editor = (await screen.findByTestId('chat-message-input')) as HTMLElement;
    editor.focus();

    // Seed "Hello world" the way the editor itself does.
    await act(async () => {
      const p = editor.querySelector('p') ?? editor;
      p.textContent = 'Hello world';
      editor.dispatchEvent(new InputEvent('input', { bubbles: true, data: 'Hello world' }));
    });
    await flush();
    expect(hostValue.current).toBe('Hello world');

    // The user puts the caret after "Hello" and types "X": the browser edits the
    // text node in place, then fires `input`.
    const textNode = Array.from(editor.querySelectorAll('*'))
      .flatMap(el => Array.from(el.childNodes))
      .find(n => n.nodeType === Node.TEXT_NODE && n.textContent === 'Hello world') as Text;
    expect(textNode).toBeTruthy();
    const range = document.createRange();
    range.setStart(textNode, 5);
    range.collapse(true);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);

    await act(async () => {
      textNode.data = 'HelloX world';
      const r = document.createRange();
      r.setStart(textNode, 6);
      r.collapse(true);
      sel.removeAllRanges();
      sel.addRange(r);
      editor.dispatchEvent(new InputEvent('input', { bubbles: true, data: 'X' }));
    });
    await flush();

    expect(hostValue.current).toBe('HelloX world');
    const after = window.getSelection()!;
    expect(after.anchorNode?.textContent).toBe('HelloX world');
    expect(after.anchorOffset).toBe(6);
  });
});
