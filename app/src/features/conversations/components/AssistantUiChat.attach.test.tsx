/**
 * The composer's hidden file input must survive chat re-renders.
 *
 * Regression: `Conversations` hands `AssistantUiChat` a fresh `onAttachFiles`
 * function on every render, and the `ComposerAddAttachment` slot (which used to
 * own the `<input type="file">`) was rebuilt from it, so React saw a new
 * component type and replaced the input. A re-render while the OS file picker
 * was open (streaming, polling, store updates) left the chosen files on a
 * detached input whose `onChange` never reached React, so nothing was attached.
 */
import { combineReducers, configureStore } from '@reduxjs/toolkit';
import { fireEvent, render } from '@testing-library/react';
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

function buildStore() {
  return configureStore({
    reducer: combineReducers({
      thread: threadReducer,
      chatRuntime: chatRuntimeReducer,
      theme: themeReducer,
    }),
  });
}

function Chat({
  onAttachFiles,
}: {
  onAttachFiles: (files: FileList | File[] | null) => Promise<void>;
}) {
  return (
    <AssistantUiChat
      threadGoal={{ open: vi.fn() } as never}
      model={null}
      onModelChange={vi.fn()}
      sampling={{ effort: null } as never}
      onSamplingChange={vi.fn()}
      inputValue=""
      onInputValueChange={vi.fn()}
      attachments={[]}
      onAttachFiles={onAttachFiles}
      onRemoveAttachment={vi.fn()}
      maxAttachments={4}
      attachmentsEnabled
      attachmentInteractionBlocked={false}
      onAttachmentOnlySend={vi.fn()}
      preset="auto"
      onPresetChange={vi.fn()}
    />
  );
}

const findInput = (container: HTMLElement) =>
  container.querySelector<HTMLInputElement>('input[type="file"]');

describe('composer file input', () => {
  it('keeps the same input element when the parent re-renders with a new onAttachFiles', async () => {
    const store = buildStore();
    const first = vi.fn().mockResolvedValue(undefined);
    const { container, rerender } = render(
      <Provider store={store}>
        <Chat onAttachFiles={first} />
      </Provider>
    );
    await vi.waitFor(() => expect(findInput(container)).not.toBeNull());
    const before = findInput(container);

    const second = vi.fn().mockResolvedValue(undefined);
    rerender(
      <Provider store={store}>
        <Chat onAttachFiles={second} />
      </Provider>
    );

    const after = findInput(container);
    expect(after).not.toBeNull();
    expect(after).toBe(before);
    expect(before?.isConnected).toBe(true);
  });

  it('forwards the chosen files to the latest onAttachFiles', async () => {
    const store = buildStore();
    const first = vi.fn().mockResolvedValue(undefined);
    const { container, rerender } = render(
      <Provider store={store}>
        <Chat onAttachFiles={first} />
      </Provider>
    );
    await vi.waitFor(() => expect(findInput(container)).not.toBeNull());
    const input = findInput(container) as HTMLInputElement;

    const latest = vi.fn().mockResolvedValue(undefined);
    rerender(
      <Provider store={store}>
        <Chat onAttachFiles={latest} />
      </Provider>
    );

    const file = new File(['hello'], 'note.txt', { type: 'text/plain' });
    fireEvent.change(input, { target: { files: [file] } });

    expect(first).not.toHaveBeenCalled();
    expect(latest).toHaveBeenCalledTimes(1);
    const passed = latest.mock.calls[0][0] as FileList;
    expect(passed.length).toBe(1);
    expect(passed[0]).toBe(file);
  });
});
