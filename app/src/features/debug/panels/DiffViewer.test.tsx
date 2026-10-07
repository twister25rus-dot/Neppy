import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { DiffViewer } from './DiffViewer';
import { makeDiff } from './testFixtures';

const getDebugDiff = vi.fn();

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (k: string) => k, locale: 'en' }),
}));
vi.mock('../../../services/api/debugModeApi', () => ({
  getDebugDiff: (...args: unknown[]) => getDebugDiff(...args),
}));

describe('DiffViewer', () => {
  beforeEach(() => getDebugDiff.mockReset());

  it('shows a loading state first', async () => {
    let release: (d: ReturnType<typeof makeDiff>) => void = () => {};
    getDebugDiff.mockReturnValue(new Promise(resolve => (release = resolve)));
    render(<DiffViewer />);
    expect(screen.getByText('debug.panels.loading')).toBeInTheDocument();
    release(makeDiff());
    await screen.findByTestId('debug-diff-summary');
  });

  it('renders summary, per-file sections, coloured lines and untracked files', async () => {
    getDebugDiff.mockResolvedValue(makeDiff());
    render(<DiffViewer checkpointId="cp9" />);
    await waitFor(() => expect(screen.getAllByTestId('debug-diff-file')).toHaveLength(2));
    expect(getDebugDiff).toHaveBeenCalledWith('cp9');
    expect(screen.getByTestId('debug-diff-summary')).toHaveTextContent('debug.panels.diff.summary');
    expect(screen.getByText('src/a.ts')).toBeInTheDocument();
    const added = document.querySelector('[data-line-kind="add"]');
    const removed = document.querySelector('[data-line-kind="del"]');
    expect(added?.className).toContain('sage');
    expect(removed?.className).toContain('coral');
    expect(screen.getByTestId('debug-diff-untracked')).toHaveTextContent('scratch.log');
    expect(screen.queryByTestId('debug-diff-truncated')).toBeNull();
  });

  it('collapses a file section', async () => {
    getDebugDiff.mockResolvedValue(makeDiff());
    render(<DiffViewer />);
    await waitFor(() => screen.getAllByTestId('debug-diff-file'));
    expect(screen.getByText(/old line/)).toBeInTheDocument();
    fireEvent.click(screen.getByText('src/a.ts'));
    expect(screen.queryByText(/old line/)).toBeNull();
  });

  it('shows the truncated notice', async () => {
    getDebugDiff.mockResolvedValue(makeDiff({ truncated: true }));
    render(<DiffViewer />);
    expect(await screen.findByTestId('debug-diff-truncated')).toBeInTheDocument();
  });

  it('shows an empty state for a clean tree', async () => {
    getDebugDiff.mockResolvedValue(
      makeDiff({
        text: '',
        files: [],
        untracked: [],
        summary: { modified: 0, created: 0, deleted: 0 },
      })
    );
    render(<DiffViewer />);
    expect(await screen.findByText('debug.panels.diff.empty')).toBeInTheDocument();
  });

  it('shows an inline error and retries', async () => {
    getDebugDiff.mockRejectedValueOnce(new Error('boom'));
    getDebugDiff.mockResolvedValueOnce(makeDiff());
    render(<DiffViewer />);
    expect(await screen.findByRole('alert')).toHaveTextContent('debug.panels.diff.error');
    fireEvent.click(screen.getByText('common.retry'));
    await waitFor(() => expect(screen.getAllByTestId('debug-diff-file')).toHaveLength(2));
    expect(getDebugDiff).toHaveBeenCalledTimes(2);
  });
});
