import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { DebugModeSettings } from '../../../services/api/debugModeApi';
import { renderWithProviders } from '../../../test/test-utils';
import DebugModePanel from './DebugModePanel';

const mockGet = vi.fn();
const mockUpdate = vi.fn();

vi.mock('../../../services/api/debugModeApi', () => ({
  getDebugSettings: (...a: unknown[]) => mockGet(...a),
  updateDebugSettings: (...a: unknown[]) => mockUpdate(...a),
}));

const base: DebugModeSettings = {
  enabled: true,
  project_root: null,
  auto_checkpoint: true,
  auto_repair: true,
  max_repair_iterations: 5,
  run_tests_after_changes: true,
  run_build_after_changes: true,
  allow_dependency_install: false,
  allow_external_filesystem: false,
  external_paths: [],
  allow_system_commands: false,
  allow_git_commit: true,
  allow_git_push: false,
  dangerous_commands_require_confirmation: true,
};

const renderPanel = () =>
  renderWithProviders(<DebugModePanel />, { initialEntries: ['/settings/debug-mode'] });

beforeEach(() => {
  mockGet.mockReset();
  mockUpdate.mockReset();
  mockGet.mockResolvedValue({ ...base });
  mockUpdate.mockImplementation(async (patch: Partial<DebugModeSettings>) => ({
    ...base,
    ...patch,
  }));
});

describe('DebugModePanel', () => {
  it('loads and renders the current values', async () => {
    mockGet.mockResolvedValue({ ...base, auto_repair: false, max_repair_iterations: 7 });
    renderPanel();
    await screen.findByTestId('debug-dangerous-section');
    expect(document.getElementById('switch-debug-enabled')).toHaveAttribute('aria-checked', 'true');
    expect(document.getElementById('switch-debug-auto_repair')).toHaveAttribute(
      'aria-checked',
      'false'
    );
    expect(document.getElementById('switch-debug-allow_git_push')).toHaveAttribute(
      'aria-checked',
      'false'
    );
    expect(document.getElementById('debug-max-repairs')).toHaveValue(7);
    // Always-on capabilities are informational, not switches.
    expect(screen.getAllByText('Always on')).toHaveLength(4);
  });

  it('shows a load error with retry', async () => {
    mockGet.mockRejectedValueOnce(new Error('core offline'));
    renderPanel();
    expect(await screen.findByText('core offline')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await screen.findByTestId('debug-dangerous-section');
    expect(mockGet).toHaveBeenCalledTimes(2);
  });

  it('toggling sends a minimal patch', async () => {
    renderPanel();
    await screen.findByTestId('debug-dangerous-section');
    fireEvent.click(document.getElementById('switch-debug-allow_git_push') as HTMLElement);
    await waitFor(() => expect(mockUpdate).toHaveBeenCalledTimes(1));
    expect(mockUpdate).toHaveBeenCalledWith({ allow_git_push: true });
    await waitFor(() =>
      expect(document.getElementById('switch-debug-allow_git_push')).toHaveAttribute(
        'aria-checked',
        'true'
      )
    );
  });

  it('reverts and shows an inline error when saving fails', async () => {
    mockUpdate.mockRejectedValueOnce(new Error('not allowed here'));
    renderPanel();
    await screen.findByTestId('debug-dangerous-section');
    fireEvent.click(document.getElementById('switch-debug-allow_system_commands') as HTMLElement);
    expect(await screen.findByText('not allowed here')).toBeInTheDocument();
    expect(document.getElementById('switch-debug-allow_system_commands')).toHaveAttribute(
      'aria-checked',
      'false'
    );
  });

  it('adds and removes external paths', async () => {
    renderPanel();
    await screen.findByTestId('debug-dangerous-section');
    const input = screen.getByLabelText('Absolute path, e.g. /Users/me/shared');

    fireEvent.change(input, { target: { value: 'relative/dir' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    expect(screen.getByText('Enter an absolute path.')).toBeInTheDocument();
    expect(mockUpdate).not.toHaveBeenCalled();

    fireEvent.change(input, { target: { value: '/opt/shared' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add' }));
    await waitFor(() =>
      expect(mockUpdate).toHaveBeenCalledWith({ external_paths: ['/opt/shared'] })
    );
    expect(await screen.findByText('/opt/shared')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(mockUpdate).toHaveBeenLastCalledWith({ external_paths: [] }));
  });

  it('clamps max repair attempts to 1..20 before saving', async () => {
    renderPanel();
    await screen.findByTestId('debug-dangerous-section');
    let input = document.getElementById('debug-max-repairs') as HTMLInputElement;
    fireEvent.change(input, { target: { value: '99' } });
    fireEvent.blur(input);
    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ max_repair_iterations: 20 }));
    // The field remounts on the saved value, so re-query it.
    await waitFor(() => expect(document.getElementById('debug-max-repairs')).toHaveValue(20));
    input = document.getElementById('debug-max-repairs') as HTMLInputElement;

    fireEvent.change(input, { target: { value: '0' } });
    fireEvent.blur(input);
    await waitFor(() => expect(mockUpdate).toHaveBeenLastCalledWith({ max_repair_iterations: 1 }));
  });

  it('does not call the core when the repair value is unchanged', async () => {
    renderPanel();
    await screen.findByTestId('debug-dangerous-section');
    const input = document.getElementById('debug-max-repairs') as HTMLInputElement;
    fireEvent.blur(input);
    expect(mockUpdate).not.toHaveBeenCalled();
  });

  it('saves a project root and resets it to default with null', async () => {
    mockGet.mockResolvedValue({ ...base, project_root: '/work/app' });
    renderPanel();
    await screen.findByTestId('debug-dangerous-section');
    expect(screen.getByLabelText('Project root')).toHaveValue('/work/app');
    fireEvent.click(screen.getByRole('button', { name: 'Use default' }));
    await waitFor(() => expect(mockUpdate).toHaveBeenCalledWith({ project_root: null }));
    await waitFor(() => expect(screen.getByLabelText('Project root')).toHaveValue(''));
    expect(screen.getByRole('button', { name: 'Use default' })).toBeDisabled();

    fireEvent.change(screen.getByLabelText('Project root'), { target: { value: '/new/root' } });
    fireEvent.blur(screen.getByLabelText('Project root'));
    await waitFor(() => expect(mockUpdate).toHaveBeenLastCalledWith({ project_root: '/new/root' }));
  });

  it('warns inline when dangerous-command confirmation is turned off', async () => {
    renderPanel();
    await screen.findByTestId('debug-dangerous-section');
    expect(screen.queryByTestId('debug-confirm-off-warning')).not.toBeInTheDocument();
    fireEvent.click(
      document.getElementById('switch-debug-dangerous_commands_require_confirmation') as HTMLElement
    );
    expect(await screen.findByTestId('debug-confirm-off-warning')).toBeInTheDocument();
    expect(mockUpdate).toHaveBeenCalledWith({ dangerous_commands_require_confirmation: false });
  });
});
