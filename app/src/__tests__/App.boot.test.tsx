/**
 * Tests that App.tsx calls startInternetStatusListener and startCoreHealthMonitor
 * at module boot time (lines 50-51, #1527).
 *
 * We must mock every service/component that App.tsx (or its recursive imports)
 * pulls in at module scope to keep the test fast and isolated.
 */
import * as React from 'react';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MemoryRouter, useLocation, useNavigate } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

// ---- Service mocks that must be in place BEFORE App.tsx is imported ----

const startInternetStatusListenerMock = vi.fn();
const stopInternetStatusListenerMock = vi.fn();
const startCoreHealthMonitorMock = vi.fn();
const stopCoreHealthMonitorMock = vi.fn();
const startNativeNotificationsServiceMock = vi.fn();
const stopNativeNotificationsServiceMock = vi.fn();
const useCoreStateMock = vi.fn(() => ({
  snapshot: { sessionToken: null as string | null, onboardingCompleted: true },
  isBootstrapping: false,
}));

vi.mock('../services/internetStatusListener', () => ({
  startInternetStatusListener: startInternetStatusListenerMock,
  stopInternetStatusListener: stopInternetStatusListenerMock,
}));

vi.mock('../services/coreHealthMonitor', () => ({
  startCoreHealthMonitor: startCoreHealthMonitorMock,
  stopCoreHealthMonitor: stopCoreHealthMonitorMock,
}));

// Stub out the heavy services that also run at module boot in App.tsx.
vi.mock('../lib/nativeNotifications', () => ({
  startNativeNotificationsService: startNativeNotificationsServiceMock,
  stopNativeNotificationsService: stopNativeNotificationsServiceMock,
}));

// Stub out all imports that would pull in Tauri or heavy React trees.
vi.mock('../store', () => ({
  store: { dispatch: vi.fn(), getState: vi.fn(() => ({})), subscribe: vi.fn() },
  persistor: { subscribe: vi.fn(), getState: vi.fn(() => ({ bootstrapped: true })) },
}));
vi.mock('../providers/CoreStateProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
  useCoreState: useCoreStateMock,
}));
vi.mock('../providers/SocketProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../providers/ChatRuntimeProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../AppRoutes', () => ({
  default: ({ location }: { location?: { pathname?: string } | string }) => (
    <div data-testid="rendered-background-route">
      {typeof location === 'string' ? location : (location?.pathname ?? 'ambient')}
    </div>
  ),
}));
vi.mock('../pages/Settings', () => ({
  default: ({ presentation, onClose }: { presentation?: string; onClose?: () => void }) => (
    <div data-testid="settings-presentation" data-presentation={presentation}>
      <button type="button" onClick={onClose}>
        Close settings
      </button>
    </div>
  ),
}));
vi.mock('../components/BootCheckGate/BootCheckGate', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../components/MeshGradient', () => ({ default: () => null }));
vi.mock('../components/AppBackground', () => ({ default: () => null }));
vi.mock('../components/layout/shell/AppSidebar', () => ({ default: () => null }));
vi.mock('../components/layout/shell/RootShellLayout', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../components/AppUpdatePrompt', () => ({ default: () => null }));
vi.mock('../components/LocalAIDownloadSnackbar', () => ({ default: () => null }));
vi.mock('../components/daemon/ServiceBlockingGate', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../components/commands/CommandProvider', () => ({
  default: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));
vi.mock('../components/DictationHotkeyManager', () => ({ default: () => null }));
vi.mock('../components/PttHotkeyManager', () => ({ default: () => null }));
vi.mock('../components/NeppyLinkModal', () => ({ default: () => null }));
vi.mock('../components/notices/NoticeCenter', () => ({ default: () => null }));
vi.mock('../components/walkthrough/AppWalkthrough', () => ({ default: () => null }));
vi.mock('../hooks/useNotchBootSync', () => ({ useNotchBootSync: vi.fn() }));
vi.mock('../services/analytics', () => ({ trackPageView: vi.fn() }));
vi.mock('../utils/accountsFullscreen', () => ({ AGENT_ACCOUNT_ID: '__agent__' }));
vi.mock('../store/hooks', () => ({ useAppSelector: vi.fn(() => null) }));
vi.mock('@sentry/react', () => ({
  ErrorBoundary: ({ children }: { children: React.ReactNode }) => <>{children}</>,
}));

describe('App.tsx boot-time service wiring (lines 50-51)', () => {
  it('calls startInternetStatusListener and startCoreHealthMonitor at module load', async () => {
    await import('../App');
    expect(startInternetStatusListenerMock).toHaveBeenCalled();
    expect(startCoreHealthMonitorMock).toHaveBeenCalled();
  });

  it('stops boot-time services from the HMR cleanup helper', async () => {
    const { stopBootServicesForHmr } = await import('../App');

    stopBootServicesForHmr();

    expect(stopNativeNotificationsServiceMock).toHaveBeenCalled();
    expect(stopInternetStatusListenerMock).toHaveBeenCalled();
    expect(stopCoreHealthMonitorMock).toHaveBeenCalled();
  });
});

function DesktopShellHarness() {
  const location = useLocation();
  const navigate = useNavigate();
  return (
    <>
      <output data-testid="ambient-path">{location.pathname}</output>
      <button type="button" onClick={() => navigate('/settings/account')}>
        Open settings
      </button>
      <AppShellDesktopLoader />
    </>
  );
}

function AppShellDesktopLoader() {
  const [Component, setComponent] = React.useState<React.ComponentType | null>(null);

  React.useEffect(() => {
    void import('../App').then(module => setComponent(() => module.AppShellDesktop));
  }, []);

  return Component ? <Component /> : null;
}

describe('AppShellDesktop settings dialog routing', () => {
  it('keeps the previous page behind Settings and closes back to it', async () => {
    useCoreStateMock.mockReturnValue({
      snapshot: { sessionToken: 'session-token', onboardingCompleted: true },
      isBootstrapping: false,
    });

    render(
      <MemoryRouter initialEntries={['/connections']}>
        <DesktopShellHarness />
      </MemoryRouter>
    );

    await screen.findByTestId('rendered-background-route');
    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));

    expect(await screen.findByTestId('settings-presentation')).toHaveAttribute(
      'data-presentation',
      'dialog'
    );
    expect(screen.getByTestId('rendered-background-route')).toHaveTextContent('/connections');

    fireEvent.click(screen.getByRole('button', { name: 'Close settings' }));
    await waitFor(() =>
      expect(screen.getByTestId('ambient-path')).toHaveTextContent('/connections')
    );
  });

  it('uses Chat behind a direct settings deep link', async () => {
    useCoreStateMock.mockReturnValue({
      snapshot: { sessionToken: 'session-token', onboardingCompleted: true },
      isBootstrapping: false,
    });

    render(
      <MemoryRouter initialEntries={['/settings/account']}>
        <DesktopShellHarness />
      </MemoryRouter>
    );

    expect(await screen.findByTestId('settings-presentation')).toBeInTheDocument();
    expect(screen.getByTestId('rendered-background-route')).toHaveTextContent('/chat');
  });

  it('does not reuse a legacy Settings redirect as the backdrop', async () => {
    useCoreStateMock.mockReturnValue({
      snapshot: { sessionToken: 'session-token', onboardingCompleted: true },
      isBootstrapping: false,
    });

    render(
      <MemoryRouter initialEntries={['/activity']}>
        <DesktopShellHarness />
      </MemoryRouter>
    );

    await screen.findByTestId('rendered-background-route');
    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));

    expect(await screen.findByTestId('settings-presentation')).toBeInTheDocument();
    expect(screen.getByTestId('rendered-background-route')).toHaveTextContent('/chat');
  });

  it('leaves unauthenticated Settings URLs to the protected route', async () => {
    useCoreStateMock.mockReturnValue({
      snapshot: { sessionToken: null, onboardingCompleted: true },
      isBootstrapping: false,
    });

    render(
      <MemoryRouter initialEntries={['/settings/account']}>
        <DesktopShellHarness />
      </MemoryRouter>
    );

    await screen.findByTestId('rendered-background-route');
    expect(screen.queryByTestId('settings-presentation')).not.toBeInTheDocument();
    expect(screen.getByTestId('rendered-background-route')).toHaveTextContent('ambient');
  });
});
