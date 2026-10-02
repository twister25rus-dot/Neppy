import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import CompanionDataDialog from './CompanionDataDialog';
import { makeActionLog, makeCompanionData, makeSuggestion } from './companionFixtures';

const mockGet = vi.fn();
const mockDelete = vi.fn();
vi.mock('../../../services/api/petCompanionApi', () => ({
  getCompanionData: (...a: unknown[]) => mockGet(...a),
  deleteCompanionData: (...a: unknown[]) => mockDelete(...a),
}));

describe('CompanionDataDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGet.mockResolvedValue(makeCompanionData());
    mockDelete.mockResolvedValue({ deleted_suggestions: 1, deleted_actions: 1, deleted_notes: 0 });
  });

  it('lists retained suggestions and the action log', async () => {
    renderWithProviders(<CompanionDataDialog onClose={vi.fn()} />);
    expect(await screen.findByTestId('companion-data-suggestion')).toHaveTextContent(
      'Want help with this build error?'
    );
    const action = screen.getByTestId('companion-data-action');
    expect(action).toHaveTextContent('Explain');
    expect(action).toHaveTextContent('You confirmed');
  });

  it('deletes one suggestion', async () => {
    const onChanged = vi.fn();
    mockGet
      .mockResolvedValueOnce(
        makeCompanionData({
          suggestions: [makeSuggestion({ id: 's1' }), makeSuggestion({ id: 's2' })],
          counts: { suggestions: 2, actions: 0 },
          actions: [],
        })
      )
      .mockResolvedValue(
        makeCompanionData({
          suggestions: [makeSuggestion({ id: 's2' })],
          counts: { suggestions: 1, actions: 0 },
          actions: [],
        })
      );
    renderWithProviders(<CompanionDataDialog onClose={vi.fn()} onChanged={onChanged} />);
    const buttons = await screen.findAllByTestId('companion-delete-one');
    fireEvent.click(buttons[0]);
    await waitFor(() => expect(mockDelete).toHaveBeenCalledWith({ suggestionId: 's1' }));
    await waitFor(() => expect(screen.getAllByTestId('companion-data-suggestion')).toHaveLength(1));
    expect(onChanged).toHaveBeenCalled();
  });

  it('requires confirmation before deleting everything', async () => {
    renderWithProviders(<CompanionDataDialog onClose={vi.fn()} />);
    await screen.findByTestId('companion-data-suggestion');
    fireEvent.click(screen.getByTestId('companion-delete-all'));
    expect(mockDelete).not.toHaveBeenCalled();

    fireEvent.click(await screen.findByTestId('confirm-dialog-confirm'));
    await waitFor(() =>
      expect(mockDelete).toHaveBeenCalledWith({ all: true, includeSavedNotes: false })
    );
  });

  it('can also delete the notes the pet saved', async () => {
    renderWithProviders(<CompanionDataDialog onClose={vi.fn()} />);
    await screen.findByTestId('companion-data-suggestion');
    fireEvent.click(screen.getByTestId('companion-delete-all'));
    fireEvent.click(await screen.findByTestId('companion-delete-notes'));
    fireEvent.click(screen.getByTestId('confirm-dialog-confirm'));
    await waitFor(() =>
      expect(mockDelete).toHaveBeenCalledWith({ all: true, includeSavedNotes: true })
    );
  });

  it('cancelling the confirmation deletes nothing', async () => {
    renderWithProviders(<CompanionDataDialog onClose={vi.fn()} />);
    await screen.findByTestId('companion-data-suggestion');
    fireEvent.click(screen.getByTestId('companion-delete-all'));
    await screen.findByTestId('confirm-dialog-confirm');
    fireEvent.click(screen.getAllByRole('button', { name: 'Cancel' })[0]);
    expect(mockDelete).not.toHaveBeenCalled();
  });

  it('shows the empty state when nothing is stored', async () => {
    mockGet.mockResolvedValue(
      makeCompanionData({ suggestions: [], actions: [], counts: { suggestions: 0, actions: 0 } })
    );
    renderWithProviders(<CompanionDataDialog onClose={vi.fn()} />);
    expect(await screen.findByText('Nothing is stored.')).toBeInTheDocument();
  });

  it('shows an error when a delete fails', async () => {
    mockDelete.mockRejectedValue(new Error('nope'));
    mockGet.mockResolvedValue(
      makeCompanionData({ actions: [makeActionLog({ decision: 'refused_high_risk' })] })
    );
    renderWithProviders(<CompanionDataDialog onClose={vi.fn()} />);
    fireEvent.click(await screen.findByTestId('companion-delete-one'));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not delete');
  });
});
