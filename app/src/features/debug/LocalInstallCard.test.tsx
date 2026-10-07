import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { LocalInstallResult, LocalInstallStatus } from '../../services/api/debugModeApi';
import { renderWithProviders } from '../../test/test-utils';
import { LocalInstallCard } from './LocalInstallCard';
import { LOCAL_INSTALL_POLL_MS } from './useLocalInstall';

const api = vi.hoisted(() => ({
  startLocalInstallBuild: vi.fn(),
  getLocalInstallStatus: vi.fn(),
  applyLocalInstall: vi.fn(),
  getLocalInstallResult: vi.fn(),
  quitApp: vi.fn(),
}));
vi.mock('../../services/api/debugModeApi', () => api);
vi.mock('../../services/analytics', () => ({ trackEvent: vi.fn() }));

const status = (over: Partial<LocalInstallStatus>): LocalInstallStatus => ({
  phase: 'idle',
  version: null,
  started_at: null,
  finished_at: null,
  bundle_path: null,
  error: '',
  ...over,
});

const restored: LocalInstallResult = {
  status: 'restored',
  version: '0.69.0',
  backup: '/b',
  ts: '2026-10-07T10:00:00Z',
  reason: 'launch marker never cleared',
  seen: false,
};

async function tick(ms = LOCAL_INSTALL_POLL_MS) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

describe('LocalInstallCard', () => {
  beforeEach(() => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    Object.values(api).forEach(f => f.mockReset());
    api.getLocalInstallStatus.mockResolvedValue(status({}));
    api.getLocalInstallResult.mockResolvedValue(null);
    api.quitApp.mockResolvedValue(undefined);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it('builds, polls the log tail every 3 s, then offers install once ready', async () => {
    api.startLocalInstallBuild.mockResolvedValue(status({ phase: 'building', version: '0.69.0' }));
    renderWithProviders(<LocalInstallCard />);

    const build = await screen.findByTestId('debug-local-build');
    expect(build).toHaveTextContent('Build & install locally');
    expect(screen.queryByTestId('debug-local-apply')).toBeNull();
    fireEvent.click(build);

    expect(await screen.findByTestId('debug-local-building')).toBeInTheDocument();
    expect(api.startLocalInstallBuild).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('debug-local-build')).toBeDisabled();

    const callsBefore = api.getLocalInstallStatus.mock.calls.length;
    api.getLocalInstallStatus.mockResolvedValue(
      status({ phase: 'building', log_tail: 'Compiling neppy_core' })
    );
    await tick();
    expect(api.getLocalInstallStatus.mock.calls.length).toBe(callsBefore + 1);
    expect(await screen.findByTestId('debug-local-log')).toHaveTextContent('Compiling neppy_core');

    api.getLocalInstallStatus.mockResolvedValue(
      status({ phase: 'ready', version: '0.69.0', bundle_path: '/x/Neppy.app' })
    );
    await tick();
    expect(await screen.findByTestId('debug-local-ready')).toBeInTheDocument();
    expect(screen.getByText('Version 0.69.0')).toBeInTheDocument();
    expect(screen.getByTestId('debug-local-apply')).toHaveTextContent('Install and restart');

    // Polling stops once the build settled.
    const settled = api.getLocalInstallStatus.mock.calls.length;
    await tick(LOCAL_INSTALL_POLL_MS * 3);
    expect(api.getLocalInstallStatus.mock.calls.length).toBe(settled);
  });

  it('asks before installing, then applies and quits the app', async () => {
    api.getLocalInstallStatus.mockResolvedValue(status({ phase: 'ready', version: '1.0.0' }));
    api.applyLocalInstall.mockResolvedValue(status({ phase: 'installing' }));
    renderWithProviders(<LocalInstallCard />);

    fireEvent.click(await screen.findByTestId('debug-local-apply'));
    expect(await screen.findByTestId('debug-local-confirm-body')).toHaveTextContent(
      /backed up first and restored automatically/
    );
    expect(api.applyLocalInstall).not.toHaveBeenCalled();

    // Cancel changes nothing.
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByTestId('debug-local-confirm-body')).toBeNull());
    expect(api.applyLocalInstall).not.toHaveBeenCalled();
    expect(api.quitApp).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('debug-local-apply'));
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(
      Array.from(dialog.querySelectorAll('button')).find(b =>
        /install and restart/i.test(b.textContent ?? '')
      )!
    );

    await waitFor(() => expect(api.applyLocalInstall).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(api.quitApp).toHaveBeenCalledTimes(1));
    expect(api.applyLocalInstall.mock.invocationCallOrder[0]).toBeLessThan(
      api.quitApp.mock.invocationCallOrder[0]
    );
    expect(await screen.findByTestId('debug-local-installing')).toBeInTheDocument();
  });

  it('shows the core refusal when the build cannot start', async () => {
    api.startLocalInstallBuild.mockRejectedValue(
      new Error('refused: the current debug task changed critical files')
    );
    renderWithProviders(<LocalInstallCard />);
    fireEvent.click(await screen.findByTestId('debug-local-build'));
    expect(await screen.findByTestId('debug-local-error')).toHaveTextContent(
      /Could not start the build: .*critical files/
    );
    expect(screen.getByTestId('debug-local-build')).not.toBeDisabled();
  });

  it('does not quit when the install could not be handed over', async () => {
    api.getLocalInstallStatus.mockResolvedValue(status({ phase: 'ready' }));
    api.applyLocalInstall.mockRejectedValue(new Error('installer helper missing'));
    renderWithProviders(<LocalInstallCard />);
    fireEvent.click(await screen.findByTestId('debug-local-apply'));
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(
      Array.from(dialog.querySelectorAll('button')).find(b =>
        /install and restart/i.test(b.textContent ?? '')
      )!
    );
    expect(await screen.findByTestId('debug-local-error')).toHaveTextContent(
      /Could not start the install: installer helper missing/
    );
    expect(api.quitApp).not.toHaveBeenCalled();
  });

  it('tells the user to quit manually when the app cannot close itself', async () => {
    api.getLocalInstallStatus.mockResolvedValue(status({ phase: 'ready' }));
    api.applyLocalInstall.mockResolvedValue(status({ phase: 'installing' }));
    api.quitApp.mockRejectedValue(new Error('ipc down'));
    renderWithProviders(<LocalInstallCard />);
    fireEvent.click(await screen.findByTestId('debug-local-apply'));
    const dialog = await screen.findByRole('dialog');
    fireEvent.click(
      Array.from(dialog.querySelectorAll('button')).find(b =>
        /install and restart/i.test(b.textContent ?? '')
      )!
    );
    expect(await screen.findByTestId('debug-local-error')).toHaveTextContent(/Quit Neppy yourself/);
  });

  it('shows a failed build with its output and lets the user build again', async () => {
    api.getLocalInstallStatus.mockResolvedValue(
      status({ phase: 'failed', error: 'tauri build exited with Some(1)' })
    );
    renderWithProviders(<LocalInstallCard />);
    expect(await screen.findByTestId('debug-local-failed')).toBeInTheDocument();
    expect(screen.getByTestId('debug-local-log')).toHaveTextContent('tauri build exited');
    expect(screen.getByTestId('debug-local-build')).toHaveTextContent('Build again');
  });

  it('shows the restored notice once and acknowledges it on dismiss', async () => {
    api.getLocalInstallResult.mockResolvedValue(restored);
    renderWithProviders(<LocalInstallCard />);
    expect(await screen.findByTestId('debug-local-restored')).toHaveTextContent(
      /previous version was restored automatically/
    );
    fireEvent.click(screen.getByTestId('debug-local-restored-dismiss'));
    await waitFor(() => expect(screen.queryByTestId('debug-local-restored')).toBeNull());
    expect(api.getLocalInstallResult).toHaveBeenLastCalledWith(true);
  });

  it('stays quiet for a seen or successful result', async () => {
    api.getLocalInstallResult.mockResolvedValue({ ...restored, seen: true });
    const { unmount } = renderWithProviders(<LocalInstallCard />);
    await screen.findByTestId('debug-local-build');
    expect(screen.queryByTestId('debug-local-restored')).toBeNull();
    unmount();
    api.getLocalInstallResult.mockResolvedValue({ ...restored, status: 'installed' });
    renderWithProviders(<LocalInstallCard />);
    await screen.findByTestId('debug-local-build');
    expect(screen.queryByTestId('debug-local-restored')).toBeNull();
  });
});
