/**
 * Tests for the global app-update prompt.
 *
 * Drives the underlying `useAppUpdate` hook through the shared mocks and
 * asserts the user-visible UX contract:
 *   - a found update NOTIFIES and downloads nothing until asked
 *     ("Download" / "Later" on `available`)
 *   - "Later" is remembered for that version, so the 15-minute re-check does
 *     not bring the same notice back
 *   - prompt with "Restart now" / "Later" once bytes are staged
 *     (`ready_to_install`)
 *   - error surface with retry path
 */
import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import AppUpdatePrompt from '../AppUpdatePrompt';

const hoisted = vi.hoisted(() => ({
  mockCheckAppUpdate: vi.fn(),
  mockApplyAppUpdate: vi.fn(),
  mockDownloadAppUpdate: vi.fn(),
  mockInstallAppUpdate: vi.fn(),
  mockIsTauri: vi.fn(() => true),
  statusListeners: [] as ((event: { payload: string }) => void)[],
}));

const {
  mockCheckAppUpdate,
  mockApplyAppUpdate,
  mockDownloadAppUpdate,
  mockInstallAppUpdate,
  mockIsTauri,
  statusListeners,
} = hoisted;

vi.mock('../../utils/tauriCommands', () => ({
  checkAppUpdate: hoisted.mockCheckAppUpdate,
  applyAppUpdate: hoisted.mockApplyAppUpdate,
  downloadAppUpdate: hoisted.mockDownloadAppUpdate,
  installAppUpdate: hoisted.mockInstallAppUpdate,
  isTauri: hoisted.mockIsTauri,
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((event: string, handler: (event: { payload: string }) => void) => {
    if (event === 'app-update:status') {
      hoisted.statusListeners.push(handler);
    }
    return Promise.resolve(() => {
      const idx = hoisted.statusListeners.indexOf(handler);
      if (idx >= 0) hoisted.statusListeners.splice(idx, 1);
    });
  }),
}));

const emitStatus = (payload: string) => {
  for (const listener of [...statusListeners]) listener({ payload });
};

describe('AppUpdatePrompt', () => {
  beforeEach(() => {
    statusListeners.length = 0;
    mockCheckAppUpdate.mockReset();
    mockApplyAppUpdate.mockReset();
    mockDownloadAppUpdate.mockReset();
    mockInstallAppUpdate.mockReset();
    mockIsTauri.mockReturnValue(true);
  });

  it('notifies that a version is available and downloads nothing on its own', async () => {
    mockCheckAppUpdate.mockResolvedValue({
      current_version: '0.50.0',
      available: true,
      available_version: '0.51.0',
      body: null,
    });

    renderWithProviders(<AppUpdatePrompt initialCheckDelayMs={0} recheckIntervalMs={0} />);

    await waitFor(() => {
      expect(screen.getByText('Update available')).toBeInTheDocument();
    });
    expect(screen.getByText('Version 0.51.0 is available.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Download/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Later/ })).toBeInTheDocument();
    // The whole point: the bytes are the user's bandwidth to spend. Waited out
    // the old auto-download grace window before asserting, so this would fail
    // if the default came back.
    await new Promise(resolve => setTimeout(resolve, 50));
    expect(mockDownloadAppUpdate).not.toHaveBeenCalled();
  });

  it('clicking "Download" starts the download', async () => {
    mockCheckAppUpdate.mockResolvedValue({
      current_version: '0.50.0',
      available: true,
      available_version: '0.51.0',
      body: null,
    });
    mockDownloadAppUpdate.mockResolvedValue({ ready: true, version: '0.51.0', body: null });

    renderWithProviders(<AppUpdatePrompt initialCheckDelayMs={0} recheckIntervalMs={0} />);

    const downloadBtn = await screen.findByTestId('app-update-download');
    fireEvent.click(downloadBtn);

    await waitFor(() => expect(mockDownloadAppUpdate).toHaveBeenCalledTimes(1));
    expect(mockApplyAppUpdate).not.toHaveBeenCalled();
  });

  it('remembers "Later" for that version across the re-check cadence', async () => {
    mockCheckAppUpdate.mockResolvedValue({
      current_version: '0.50.0',
      available: true,
      available_version: '0.51.0',
      body: null,
    });

    // A 50ms re-check, so the cadence runs several times inside the test: the
    // dismissal has to survive re-entering `available`, not just one render.
    renderWithProviders(<AppUpdatePrompt initialCheckDelayMs={0} recheckIntervalMs={50} />);

    const laterBtn = await screen.findByRole('button', { name: /Later/ });
    fireEvent.click(laterBtn);
    await waitFor(() => {
      expect(screen.queryByText('Update available')).not.toBeInTheDocument();
    });

    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 180));
    });
    expect(screen.queryByText('Update available')).not.toBeInTheDocument();
    expect(mockDownloadAppUpdate).not.toHaveBeenCalled();
  });

  it('shows the "Restart now" prompt once the download is staged', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    // Wait for listeners to register.
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    // Simulate the Rust side emitting ready_to_install.
    emitStatus('ready_to_install');

    await waitFor(() => {
      expect(screen.getByText('Update ready to install')).toBeInTheDocument();
    });
    expect(screen.getByRole('button', { name: /Restart now/ })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Later/ })).toBeInTheDocument();
  });

  it('clicking "Restart now" invokes installAppUpdate (the staged path)', async () => {
    mockInstallAppUpdate.mockResolvedValueOnce(undefined);

    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    // The Rust side emits `ready_to_install` once bytes are staged. The
    // hook's status listener flips `stagedRef` to true on that event, so a
    // subsequent install() must take the fast staged path and call
    // `installAppUpdate` directly — never falling back to the legacy
    // combined `applyAppUpdate`.
    emitStatus('ready_to_install');

    const restartBtn = await screen.findByRole('button', { name: /Restart now/ });
    fireEvent.click(restartBtn);

    await waitFor(() => {
      expect(mockInstallAppUpdate).toHaveBeenCalledTimes(1);
    });
    expect(mockApplyAppUpdate).not.toHaveBeenCalled();
  });

  it('clicking "Later" hides the banner without calling install', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('ready_to_install');

    const laterBtn = await screen.findByRole('button', { name: /Later/ });
    fireEvent.click(laterBtn);

    await waitFor(() => {
      expect(screen.queryByText('Update ready to install')).not.toBeInTheDocument();
    });
    expect(mockInstallAppUpdate).not.toHaveBeenCalled();
    expect(mockApplyAppUpdate).not.toHaveBeenCalled();
  });

  it('renders an error banner with retry on failure', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('error');

    await waitFor(() => {
      expect(screen.getByText('Update failed')).toBeInTheDocument();
    });
    expect(screen.getByRole('button', { name: /Try again/ })).toBeInTheDocument();
  });

  it('clicking "Try again" after error invokes downloadAppUpdate', async () => {
    mockDownloadAppUpdate.mockResolvedValueOnce({ ready: true, version: '0.51.0', body: null });

    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('error');

    const retryBtn = await screen.findByRole('button', { name: /Try again/ });
    fireEvent.click(retryBtn);

    await waitFor(() => {
      expect(mockDownloadAppUpdate).toHaveBeenCalledTimes(1);
    });
  });

  it('shows the installing-phase banner with progress copy', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('installing');

    await waitFor(() => {
      expect(screen.getByText('Installing update')).toBeInTheDocument();
    });
    expect(screen.getByText(/Installing the new version/i)).toBeInTheDocument();
  });

  it('shows the restarting-phase banner', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('restarting');

    await waitFor(() => {
      // Header label is "Restarting…" (with the ellipsis char).
      expect(screen.getByText(/Restarting/)).toBeInTheDocument();
    });
  });

  it('clicking "Dismiss" on the error banner hides the prompt', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('error');
    await waitFor(() => expect(screen.getByText('Update failed')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /^Dismiss$/i }));

    await waitFor(() => {
      expect(screen.queryByText('Update failed')).not.toBeInTheDocument();
    });
  });

  it('does not re-open the same dismissed update error on the next background cycle', async () => {
    renderWithProviders(
      <AppUpdatePrompt autoCheck={false} initialCheckDelayMs={0} recheckIntervalMs={0} />
    );
    await waitFor(() => expect(statusListeners.length).toBeGreaterThan(0));

    emitStatus('error');
    await waitFor(() => expect(screen.getByText('Update failed')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /^Dismiss$/i }));
    await waitFor(() => {
      expect(screen.queryByText('Update failed')).not.toBeInTheDocument();
    });

    await act(async () => {
      emitStatus('checking');
      emitStatus('error');
    });
    expect(screen.queryByText('Update failed')).not.toBeInTheDocument();
  });

  it('renders nothing when not in Tauri', async () => {
    mockIsTauri.mockReturnValue(false);

    renderWithProviders(<AppUpdatePrompt initialCheckDelayMs={0} recheckIntervalMs={0} />);

    await new Promise(resolve => setTimeout(resolve, 30));
    expect(screen.queryByTestId('app-update-prompt')).not.toBeInTheDocument();
  });
});
