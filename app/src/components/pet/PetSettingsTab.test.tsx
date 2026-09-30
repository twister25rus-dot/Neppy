import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { makePet } from './petFixtures';
import PetSettingsTab from './PetSettingsTab';

const mockUpdate = vi.fn();
const mockAddGoal = vi.fn();
const mockRemoveGoal = vi.fn();

vi.mock('../../services/api/petApi', () => ({
  updatePet: (...args: unknown[]) => mockUpdate(...args),
  addPetGoal: (...args: unknown[]) => mockAddGoal(...args),
  removePetGoal: (...args: unknown[]) => mockRemoveGoal(...args),
}));

describe('PetSettingsTab', () => {
  beforeEach(() => vi.clearAllMocks());

  it('keeps Save disabled until something changes', () => {
    renderWithProviders(<PetSettingsTab pet={makePet()} onSaved={vi.fn()} onChanged={vi.fn()} />);
    expect(screen.getByTestId('pet-save')).toBeDisabled();
  });

  it('sends a minimal patch containing only the changed fields', async () => {
    const saved = makePet({ name: 'Bean' });
    mockUpdate.mockResolvedValue(saved);
    const onSaved = vi.fn();
    renderWithProviders(<PetSettingsTab pet={makePet()} onSaved={onSaved} onChanged={vi.fn()} />);

    fireEvent.change(screen.getByTestId('pet-name-input'), { target: { value: '  Bean ' } });
    fireEvent.click(screen.getByTestId('pet-save'));

    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ name: 'Bean' }));
    expect(onSaved).toHaveBeenCalledWith(saved);
    expect(await screen.findByText('Saved')).toBeInTheDocument();
  });

  it('blocks an invalid time without calling the core', () => {
    renderWithProviders(<PetSettingsTab pet={makePet()} onSaved={vi.fn()} onChanged={vi.fn()} />);
    // jsdom lets a non-time string through a time input's change event via the value setter.
    fireEvent.change(screen.getByTestId('pet-digest-time-input'), { target: { value: '25:99' } });
    fireEvent.change(screen.getByTestId('pet-name-input'), { target: { value: 'Pip2' } });
    fireEvent.click(screen.getByTestId('pet-save'));
    expect(screen.getByRole('alert')).toHaveTextContent('HH:MM');
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('blocks an empty name', () => {
    renderWithProviders(<PetSettingsTab pet={makePet()} onSaved={vi.fn()} onChanged={vi.fn()} />);
    fireEvent.change(screen.getByTestId('pet-name-input'), { target: { value: '   ' } });
    fireEvent.click(screen.getByTestId('pet-save'));
    expect(screen.getByRole('alert')).toHaveTextContent('needs a name');
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('enables the pet with a single-field patch and reports save failures', async () => {
    mockUpdate.mockRejectedValue(new Error('invalid'));
    renderWithProviders(
      <PetSettingsTab pet={makePet({ enabled: false })} onSaved={vi.fn()} onChanged={vi.fn()} />
    );
    fireEvent.click(screen.getByTestId('pet-enabled-switch'));
    fireEvent.click(screen.getByTestId('pet-save'));
    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ enabled: true }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not save');
  });

  it('toggles a source and sends the new list', async () => {
    mockUpdate.mockResolvedValue(makePet());
    renderWithProviders(<PetSettingsTab pet={makePet()} onSaved={vi.fn()} onChanged={vi.fn()} />);
    fireEvent.click(screen.getByTestId('pet-source-web'));
    fireEvent.click(screen.getByTestId('pet-save'));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith({ sources: ['memory', 'tasks', 'composio'] })
    );
  });

  it('is honest that connected apps are only seen through memory', () => {
    renderWithProviders(<PetSettingsTab pet={makePet()} onSaved={vi.fn()} onChanged={vi.fn()} />);
    expect(screen.getByText(/does not open your mail or calendar directly/)).toBeInTheDocument();
    expect(screen.getByText(/only what is synced into memory/)).toBeInTheDocument();
  });

  it('adds a goal and refreshes', async () => {
    mockAddGoal.mockResolvedValue({ id: 'g1', text: 'Ship it', created_at: '' });
    const onChanged = vi.fn();
    renderWithProviders(<PetSettingsTab pet={makePet()} onSaved={vi.fn()} onChanged={onChanged} />);
    fireEvent.change(screen.getByTestId('pet-goal-input'), { target: { value: ' Ship it ' } });
    fireEvent.click(screen.getByTestId('pet-goal-add'));
    await waitFor(() => expect(mockAddGoal).toHaveBeenCalledWith('Ship it'));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it('removes a goal', async () => {
    mockRemoveGoal.mockResolvedValue({ removed: true });
    const onChanged = vi.fn();
    renderWithProviders(
      <PetSettingsTab
        pet={makePet({ goals: [{ id: 'g1', text: 'Ship it', created_at: '' }] })}
        onSaved={vi.fn()}
        onChanged={onChanged}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(mockRemoveGoal).toHaveBeenCalledWith('g1'));
    await waitFor(() => expect(onChanged).toHaveBeenCalled());
  });

  it('stops accepting goals at the limit of 20', () => {
    const goals = Array.from({ length: 20 }, (_, i) => ({
      id: `g${i}`,
      text: `Goal ${i}`,
      created_at: '',
    }));
    renderWithProviders(
      <PetSettingsTab pet={makePet({ goals })} onSaved={vi.fn()} onChanged={vi.fn()} />
    );
    expect(screen.getByTestId('pet-goal-input')).toBeDisabled();
    expect(screen.getByTestId('pet-goal-add')).toBeDisabled();
    expect(screen.getByText(/limit of 20 goals/)).toBeInTheDocument();
  });

  it('shows a goal error when the core rejects the change', async () => {
    mockAddGoal.mockRejectedValue(new Error('invalid'));
    renderWithProviders(<PetSettingsTab pet={makePet()} onSaved={vi.fn()} onChanged={vi.fn()} />);
    fireEvent.change(screen.getByTestId('pet-goal-input'), { target: { value: 'x' } });
    fireEvent.click(screen.getByTestId('pet-goal-add'));
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not update your goals');
  });
});
