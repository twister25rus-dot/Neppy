import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { DebugPanels } from './DebugPanels';
import { makeCheckpoint, makeDiff, makeTask } from './testFixtures';

const getDebugDiff = vi.fn();
const listDebugTasks = vi.fn();
const listDebugCheckpoints = vi.fn();

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));
vi.mock('../../../services/api/debugModeApi', () => ({
  getDebugDiff: (...a: unknown[]) => getDebugDiff(...a),
  getDebugTask: vi.fn(),
  listDebugTasks: (...a: unknown[]) => listDebugTasks(...a),
  listDebugCheckpoints: (...a: unknown[]) => listDebugCheckpoints(...a),
  rollbackDebug: vi.fn(),
}));

describe('DebugPanels', () => {
  beforeEach(() => {
    getDebugDiff.mockReset().mockResolvedValue(makeDiff());
    listDebugTasks.mockReset().mockResolvedValue([makeTask()]);
    listDebugCheckpoints.mockReset().mockResolvedValue([makeCheckpoint()]);
  });

  it('starts on the diff tab and switches tabs', async () => {
    render(<DebugPanels />);
    await screen.findByTestId('debug-diff-summary');
    fireEvent.mouseDown(screen.getByText('debug.panels.tab.history'), { button: 0 });
    fireEvent.click(screen.getByText('debug.panels.tab.history'));
    expect(await screen.findByTestId('debug-task-row')).toBeInTheDocument();
    fireEvent.mouseDown(screen.getByText('debug.panels.tab.checkpoints'), { button: 0 });
    fireEvent.click(screen.getByText('debug.panels.tab.checkpoints'));
    expect(await screen.findByTestId('debug-checkpoint-row')).toBeInTheDocument();
  });

  it('re-fetches when refreshKey changes', async () => {
    const { rerender } = render(<DebugPanels refreshKey={0} />);
    await screen.findByTestId('debug-diff-summary');
    rerender(<DebugPanels refreshKey={1} />);
    await waitFor(() => expect(getDebugDiff).toHaveBeenCalledTimes(2));
  });
});
