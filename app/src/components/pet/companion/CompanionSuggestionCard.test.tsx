import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import { makeSuggestion } from './companionFixtures';
import CompanionSuggestionCard from './CompanionSuggestionCard';

const writeText = vi.fn();

const setup = (over = {}, onAct = vi.fn().mockResolvedValue({ suggestion: makeSuggestion() })) => {
  const onOpenChat = vi.fn().mockResolvedValue(undefined);
  renderWithProviders(
    <ul>
      <CompanionSuggestionCard
        suggestion={makeSuggestion(over)}
        onAct={onAct}
        onOpenChat={onOpenChat}
      />
    </ul>
  );
  return { onAct, onOpenChat };
};

describe('CompanionSuggestionCard', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    writeText.mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  });

  it('does not touch the clipboard until Copy is clicked', async () => {
    const { onAct } = setup();
    expect(writeText).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('companion-action-copy_text'));
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith('The function returns a String but you passed a &str.');
    await waitFor(() => expect(onAct).toHaveBeenCalledWith('copy_text', undefined));
    expect(await screen.findByRole('status')).toHaveTextContent('Copied');
  });

  it('renders model text as plain text, never as markup', () => {
    setup({ body: '<img src=x onerror=alert(1)> **bold**' });
    const body = screen.getByTestId('companion-suggestion-body');
    expect(body.querySelector('img')).toBeNull();
    expect(body).toHaveTextContent('<img src=x onerror=alert(1)> **bold**');
  });

  it('shows an editable hand-off prompt and sends text only after confirm', async () => {
    const { onAct } = setup();
    fireEvent.click(screen.getByTestId('companion-action-handoff'));
    const prompt = await screen.findByTestId('companion-handoff-prompt');
    expect(onAct).not.toHaveBeenCalled();
    expect(prompt).toHaveValue(
      'Want help with this build error?\n\nThe function returns a String but you passed a &str.'
    );

    fireEvent.change(prompt, { target: { value: 'Fix the build and explain it' } });
    expect(onAct).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('companion-handoff-confirm'));
    await waitFor(() =>
      expect(onAct).toHaveBeenCalledWith('handoff', 'Fix the build and explain it')
    );
    await waitFor(() => expect(screen.queryByTestId('companion-handoff-form')).toBeNull());
  });

  it('cancelling a hand-off sends nothing', async () => {
    const { onAct } = setup();
    fireEvent.click(screen.getByTestId('companion-action-handoff'));
    await screen.findByTestId('companion-handoff-form');
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByTestId('companion-handoff-form')).toBeNull();
    expect(onAct).not.toHaveBeenCalled();
  });

  it('disables the hand-off confirm for an empty prompt', async () => {
    setup();
    fireEvent.click(screen.getByTestId('companion-action-handoff'));
    fireEvent.change(await screen.findByTestId('companion-handoff-prompt'), {
      target: { value: '   ' },
    });
    expect(screen.getByTestId('companion-handoff-confirm')).toBeDisabled();
  });

  it('shows hand-off progress with a link to the thread', () => {
    setup({ handoff: { thread_id: 't-42', status: 'done', result_excerpt: 'Build fixed.' } });
    const status = screen.getByTestId('companion-handoff-status');
    expect(status).toHaveTextContent('Done');
    expect(status).toHaveTextContent('Build fixed.');
    expect(screen.getByRole('link', { name: 'Open task' })).toHaveAttribute('href', '/chat/t-42');
  });

  it('saves a note and confirms it', async () => {
    const { onAct } = setup();
    fireEvent.click(screen.getByTestId('companion-action-save_note'));
    await waitFor(() => expect(onAct).toHaveBeenCalledWith('save_note', undefined));
    expect(await screen.findByRole('status')).toHaveTextContent('Saved to your notes');
  });

  it('shows a prepared command as text and never runs it', async () => {
    const onAct = vi
      .fn()
      .mockResolvedValue({ suggestion: makeSuggestion(), command_text: 'cargo clean' });
    setup({ actions: ['prepare_command', 'dismiss'] }, onAct);
    fireEvent.click(screen.getByTestId('companion-action-prepare_command'));
    expect(await screen.findByTestId('companion-command')).toHaveTextContent('cargo clean');
    expect(screen.getByTestId('companion-command')).toHaveTextContent('not run');
  });

  it('hands open-in-chat to the parent', async () => {
    const { onOpenChat } = setup();
    fireEvent.click(screen.getByTestId('companion-action-open_chat'));
    await waitFor(() => expect(onOpenChat).toHaveBeenCalledTimes(1));
  });

  it('dismisses and mutes through the core', async () => {
    const { onAct } = setup({ actions: ['dismiss', 'mute_kind', 'mute_app'] });
    fireEvent.click(screen.getByTestId('companion-action-dismiss'));
    await waitFor(() => expect(onAct).toHaveBeenCalledWith('dismiss', undefined));
    fireEvent.click(screen.getByTestId('companion-action-mute_kind'));
    await waitFor(() => expect(onAct).toHaveBeenCalledWith('mute_kind', undefined));
    fireEvent.click(screen.getByTestId('companion-action-mute_app'));
    await waitFor(() => expect(onAct).toHaveBeenCalledWith('mute_app', undefined));
  });

  it('shows an error when an action fails', async () => {
    const onAct = vi.fn().mockRejectedValue(new Error('boom'));
    setup({}, onAct);
    fireEvent.click(screen.getByTestId('companion-action-save_note'));
    expect(await screen.findByRole('alert')).toHaveTextContent('That did not work');
  });

  it('only offers actions the core listed, plus Dismiss', () => {
    setup({ actions: ['explain'] });
    expect(screen.getByTestId('companion-action-explain')).toBeInTheDocument();
    expect(screen.queryByTestId('companion-action-handoff')).toBeNull();
    expect(screen.getByTestId('companion-action-dismiss')).toBeInTheDocument();
  });
});
