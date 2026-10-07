import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { TaskHistory } from './TaskHistory';
import { makeDiff, makeTask } from './testFixtures';

const listDebugTasks = vi.fn();
const getDebugTask = vi.fn();
const getDebugDiff = vi.fn();

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));
vi.mock('../../../services/api/debugModeApi', () => ({
  listDebugTasks: (...a: unknown[]) => listDebugTasks(...a),
  getDebugTask: (...a: unknown[]) => getDebugTask(...a),
  getDebugDiff: (...a: unknown[]) => getDebugDiff(...a),
}));

describe('TaskHistory', () => {
  beforeEach(() => {
    listDebugTasks.mockReset();
    getDebugTask.mockReset();
    getDebugDiff.mockReset();
  });

  it('shows loading, then an empty state', async () => {
    listDebugTasks.mockResolvedValue([]);
    render(<TaskHistory />);
    expect(screen.getByText('debug.panels.loading')).toBeInTheDocument();
    expect(await screen.findByText('debug.panels.history.empty')).toBeInTheDocument();
    expect(listDebugTasks).toHaveBeenCalledWith(50);
  });

  it('shows an error', async () => {
    listDebugTasks.mockRejectedValue(new Error('nope'));
    render(<TaskHistory />);
    expect(await screen.findByRole('alert')).toHaveTextContent('debug.panels.history.error');
  });

  it('groups tasks by day and shows status chips', async () => {
    listDebugTasks.mockResolvedValue([
      makeTask({ id: 'a', created_at: '2026-10-04T12:00:00' }),
      makeTask({ id: 'b', created_at: '2026-10-04T09:00:00', status: 'failed' }),
      makeTask({ id: 'c', created_at: '2026-10-02T09:00:00', status: 'rolled_back' }),
    ]);
    render(<TaskHistory />);
    await waitFor(() => expect(screen.getAllByTestId('debug-task-day')).toHaveLength(2));
    expect(screen.getAllByTestId('debug-task-row')).toHaveLength(3);
    const chips = screen.getAllByTestId('debug-task-status').map(c => c.textContent);
    expect(chips[0]).toContain('✓');
    expect(chips[1]).toContain('✗');
    expect(chips[2]).toContain('↩');
  });

  it('truncates a long request', async () => {
    listDebugTasks.mockResolvedValue([makeTask({ request: 'x'.repeat(300) })]);
    render(<TaskHistory />);
    const row = await screen.findByTestId('debug-task-row');
    expect(row.textContent).toContain('…');
    expect(row.textContent).not.toContain('x'.repeat(200));
  });

  it('expands a row, loading details once, and toggles the diff', async () => {
    listDebugTasks.mockResolvedValue([makeTask()]);
    getDebugTask.mockResolvedValue(makeTask());
    getDebugDiff.mockResolvedValue(makeDiff());
    render(<TaskHistory />);
    fireEvent.click(await screen.findByText('Fix the flaky login test'));
    expect(await screen.findByTestId('debug-task-details')).toHaveTextContent(
      'Fixed the race in login.'
    );
    expect(getDebugTask).toHaveBeenCalledWith('t1');
    expect(getDebugTask).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('debug-task-details')).toHaveTextContent('unit');
    expect(screen.getByTestId('debug-task-details')).toHaveTextContent('cp1');

    fireEvent.click(screen.getByTestId('debug-history-view-diff'));
    await waitFor(() => expect(getDebugDiff).toHaveBeenCalledWith('cp1'));
    expect(await screen.findByTestId('debug-diff-summary')).toBeInTheDocument();
  });
});
