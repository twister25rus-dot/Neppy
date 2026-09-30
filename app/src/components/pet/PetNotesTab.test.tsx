import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { makeNote } from './petFixtures';
import PetNotesTab from './PetNotesTab';

const mockFetchNotes = vi.fn();
const mockDismiss = vi.fn();

vi.mock('../../services/api/petApi', () => ({
  fetchPetNotes: (...args: unknown[]) => mockFetchNotes(...args),
  dismissPetNote: (...args: unknown[]) => mockDismiss(...args),
}));

describe('PetNotesTab', () => {
  beforeEach(() => vi.clearAllMocks());

  it('shows the empty state', async () => {
    mockFetchNotes.mockResolvedValue([]);
    renderWithProviders(
      <PetNotesTab selectedId={null} onSelect={vi.fn()} refreshKey={0} onChanged={vi.fn()} />
    );
    expect(await screen.findByTestId('pet-notes-empty')).toBeInTheDocument();
  });

  it('shows an error with a retry when loading fails', async () => {
    mockFetchNotes.mockRejectedValueOnce(new Error('down'));
    mockFetchNotes.mockResolvedValueOnce([makeNote()]);
    renderWithProviders(
      <PetNotesTab selectedId={null} onSelect={vi.fn()} refreshKey={0} onChanged={vi.fn()} />
    );
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not reach your pet');
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await screen.findByTestId('pet-note-row')).toBeInTheDocument();
  });

  it('selects a note and renders its body as plain text', async () => {
    mockFetchNotes.mockResolvedValue([
      makeNote({ body: '<b>not bold</b> Plan for Friday.', proposed_action: 'Email the mentor' }),
    ]);
    const onSelect = vi.fn();
    const { rerender } = renderWithProviders(
      <PetNotesTab selectedId={null} onSelect={onSelect} refreshKey={0} onChanged={vi.fn()} />
    );
    expect(await screen.findByTestId('pet-note-select-prompt')).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('pet-note-row'));
    expect(onSelect).toHaveBeenCalledWith('note-1');

    rerender(
      <PetNotesTab selectedId="note-1" onSelect={onSelect} refreshKey={0} onChanged={vi.fn()} />
    );
    const body = await screen.findByTestId('pet-note-body');
    expect(body.textContent).toBe('<b>not bold</b> Plan for Friday.');
    expect(body.querySelector('b')).toBeNull();
    expect(screen.getByText('Email the mentor')).toBeInTheDocument();
    expect(screen.queryByTestId('pet-note-injection-warning')).toBeNull();
  });

  it('warns when a note was flagged as a possible injection', async () => {
    mockFetchNotes.mockResolvedValue([makeNote({ injection_flagged: true })]);
    renderWithProviders(
      <PetNotesTab selectedId="note-1" onSelect={vi.fn()} refreshKey={0} onChanged={vi.fn()} />
    );
    expect(await screen.findByTestId('pet-note-injection-warning')).toHaveTextContent('untrusted');
  });

  it('dismisses the selected note', async () => {
    mockFetchNotes.mockResolvedValue([makeNote()]);
    mockDismiss.mockResolvedValue(makeNote({ state: 'dismissed' }));
    const onChanged = vi.fn();
    renderWithProviders(
      <PetNotesTab selectedId="note-1" onSelect={vi.fn()} refreshKey={0} onChanged={onChanged} />
    );
    fireEvent.click(await screen.findByTestId('pet-note-dismiss'));
    await waitFor(() => expect(mockDismiss).toHaveBeenCalledWith('note-1'));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
    expect(screen.queryByTestId('pet-note-dismiss')).toBeNull();
  });

  it('refetches with the chosen state filter', async () => {
    mockFetchNotes.mockResolvedValue([]);
    renderWithProviders(
      <PetNotesTab selectedId={null} onSelect={vi.fn()} refreshKey={0} onChanged={vi.fn()} />
    );
    await screen.findByTestId('pet-notes-empty');
    fireEvent.change(screen.getByTestId('pet-notes-filter'), { target: { value: 'queued' } });
    await waitFor(() =>
      expect(mockFetchNotes).toHaveBeenLastCalledWith({ limit: 100, state: 'queued' })
    );
  });
});
