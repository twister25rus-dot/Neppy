import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { CheckpointList } from './CheckpointList';
import { makeCheckpoint } from './testFixtures';

const listDebugCheckpoints = vi.fn();
const rollbackDebug = vi.fn();

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));
vi.mock('../../../services/api/debugModeApi', () => ({
  listDebugCheckpoints: (...a: unknown[]) => listDebugCheckpoints(...a),
  rollbackDebug: (...a: unknown[]) => rollbackDebug(...a),
}));

describe('CheckpointList', () => {
  beforeEach(() => {
    listDebugCheckpoints.mockReset();
    rollbackDebug.mockReset();
  });

  it('shows loading, then empty', async () => {
    listDebugCheckpoints.mockResolvedValue([]);
    render(<CheckpointList />);
    expect(screen.getByText('debug.panels.loading')).toBeInTheDocument();
    expect(await screen.findByText('debug.panels.checkpoints.empty')).toBeInTheDocument();
    expect(listDebugCheckpoints).toHaveBeenCalledWith(30);
  });

  it('shows an error', async () => {
    listDebugCheckpoints.mockRejectedValue(new Error('bad'));
    render(<CheckpointList />);
    expect(await screen.findByRole('alert')).toHaveTextContent('debug.panels.checkpoints.error');
  });

  it('lists description, branch and short head', async () => {
    listDebugCheckpoints.mockResolvedValue([makeCheckpoint()]);
    render(<CheckpointList />);
    const row = await screen.findByTestId('debug-checkpoint-row');
    expect(row).toHaveTextContent('Before fixing login');
    expect(row).toHaveTextContent('main');
    expect(row).toHaveTextContent('abcdef1');
    expect(row).not.toHaveTextContent('abcdef12');
  });

  it('does not roll back when the dialog is cancelled', async () => {
    listDebugCheckpoints.mockResolvedValue([makeCheckpoint()]);
    render(<CheckpointList />);
    fireEvent.click(await screen.findByTestId('debug-checkpoint-rollback'));
    expect(screen.getByText('debug.panels.checkpoints.confirmTitle')).toBeInTheDocument();
    fireEvent.click(screen.getByText('common.cancel'));
    expect(rollbackDebug).not.toHaveBeenCalled();
  });

  it('rolls back once on confirm and shows counts and the pre-rollback id', async () => {
    listDebugCheckpoints.mockResolvedValue([makeCheckpoint()]);
    rollbackDebug.mockResolvedValue({
      checkpoint_id: 'cp1',
      pre_rollback_checkpoint_id: 'pre9',
      restored: ['a', 'b'],
      removed: ['c'],
      head_moved: false,
    });
    render(<CheckpointList />);
    fireEvent.click(await screen.findByTestId('debug-checkpoint-rollback'));
    fireEvent.click(screen.getByTestId('confirm-dialog-confirm'));
    const result = await screen.findByTestId('debug-rollback-result');
    expect(rollbackDebug).toHaveBeenCalledTimes(1);
    expect(rollbackDebug).toHaveBeenCalledWith('cp1');
    expect(result).toHaveTextContent('debug.panels.checkpoints.rollbackCounts');
    await waitFor(() => expect(listDebugCheckpoints).toHaveBeenCalledTimes(2));
  });

  it('shows a rollback error inline and keeps the dialog open', async () => {
    listDebugCheckpoints.mockResolvedValue([makeCheckpoint()]);
    rollbackDebug.mockRejectedValue(new Error('dirty index'));
    render(<CheckpointList />);
    fireEvent.click(await screen.findByTestId('debug-checkpoint-rollback'));
    fireEvent.click(screen.getByTestId('confirm-dialog-confirm'));
    expect(await screen.findByText('debug.panels.checkpoints.rollbackError')).toBeInTheDocument();
    expect(screen.getByText('debug.panels.checkpoints.confirmTitle')).toBeInTheDocument();
  });
});
