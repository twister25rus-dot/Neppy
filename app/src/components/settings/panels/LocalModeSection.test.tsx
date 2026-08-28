import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { LocalModeStatus } from '../../../services/api/localModeApi';
import { renderWithProviders } from '../../../test/test-utils';
import LocalModeSection from './LocalModeSection';

const callCoreRpc = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: (arg: { method: string; params: unknown }) => callCoreRpc(arg),
}));

function status(overrides: Partial<LocalModeStatus> = {}): LocalModeStatus {
  return {
    enabled: true,
    active: true,
    backendPort: 43117,
    backendUrl: 'http://127.0.0.1:43117',
    applyLocalDefaults: true,
    proxyInference: true,
    restartRequired: false,
    services: {
      replaced: 1,
      requires_setup: 1,
      unavailable: 1,
      not_applicable: 0,
      entries: [
        {
          id: 'auth.session',
          hosted: 'Hosted sign-in',
          routes: ['/auth'],
          kind: 'replaced',
          local_alternative: 'A device-local session.',
          setup: '',
        },
        {
          id: 'search.web',
          hosted: 'Managed web search',
          routes: ['/search'],
          kind: 'requires_setup',
          local_alternative: 'Your own SearXNG instance.',
          setup: 'Run SearXNG and set [searxng] base_url.',
        },
        {
          id: 'integrations.composio',
          hosted: 'Composio integrations',
          routes: ['/agent-integrations/composio'],
          kind: 'unavailable',
          local_alternative: 'MCP servers.',
          setup: '',
        },
      ],
    },
    ...overrides,
  };
}

function mockRpc(initial: LocalModeStatus, onSet?: (params: unknown) => LocalModeStatus) {
  callCoreRpc.mockImplementation((arg: { method: string; params: unknown }) => {
    if (arg.method === 'openhuman.config_get_local_mode') {
      return Promise.resolve({ result: initial });
    }
    if (arg.method === 'openhuman.config_set_local_mode') {
      return Promise.resolve({ result: onSet ? onSet(arg.params) : initial });
    }
    return Promise.reject(new Error(`unexpected method ${arg.method}`));
  });
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('LocalModeSection', () => {
  it('renders every service the core reports, not a hardcoded list', async () => {
    mockRpc(status());
    renderWithProviders(<LocalModeSection />);

    await waitFor(() =>
      expect(screen.getByTestId('local-mode-service-auth.session')).toBeInTheDocument()
    );
    expect(screen.getByTestId('local-mode-service-search.web')).toBeInTheDocument();
    expect(screen.getByTestId('local-mode-service-integrations.composio')).toBeInTheDocument();
  });

  it('shows the setup instruction only for services that need one', async () => {
    mockRpc(status());
    renderWithProviders(<LocalModeSection />);

    await waitFor(() =>
      expect(screen.getByTestId('local-mode-service-search.web')).toBeInTheDocument()
    );
    expect(screen.getByTestId('local-mode-service-search.web')).toHaveTextContent(
      'Run SearXNG and set [searxng] base_url.'
    );
    // `replaced` needs nothing — a next action here would be noise.
    expect(screen.getByTestId('local-mode-service-auth.session')).not.toHaveTextContent(
      'To set up'
    );
  });

  it('sends the toggle through the set RPC', async () => {
    mockRpc(status({ enabled: false, active: false }), () => status());
    renderWithProviders(<LocalModeSection />);

    await waitFor(() => expect(screen.getByTestId('local-mode-toggle')).toBeInTheDocument());
    fireEvent.click(screen.getByTestId('local-mode-toggle'));

    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.config_set_local_mode',
        params: { enabled: true },
      })
    );
  });

  it('warns that a restart is pending when enabled and active disagree', async () => {
    // The failure this prevents: a panel that showed only the switch would
    // report the change as applied while the core is still hosted.
    mockRpc(status({ enabled: true, active: false, restartRequired: true }));
    renderWithProviders(<LocalModeSection />);

    await waitFor(() => expect(screen.getByTestId('local-mode-pending')).toBeInTheDocument());
    expect(screen.getByTestId('local-mode-pending')).toHaveTextContent('Restart OpenHuman');
  });

  it('reports an environment override distinctly from a pending restart', async () => {
    mockRpc(status({ enabled: false, active: true, restartRequired: false }));
    renderWithProviders(<LocalModeSection />);

    await waitFor(() => expect(screen.getByTestId('local-mode-pending')).toBeInTheDocument());
    expect(screen.getByTestId('local-mode-pending')).toHaveTextContent('OPENHUMAN_LOCAL_MODE');
  });

  it('hides the local backend address while local mode is not serving', async () => {
    mockRpc(status({ enabled: false, active: false }));
    renderWithProviders(<LocalModeSection />);
    await waitFor(() => expect(screen.getByTestId('local-mode-toggle')).toBeInTheDocument());
    expect(screen.queryByTestId('local-mode-backend-url')).not.toBeInTheDocument();
  });

  it('shows the local backend address once it is serving', async () => {
    mockRpc(status());
    renderWithProviders(<LocalModeSection />);
    await waitFor(() =>
      expect(screen.getByTestId('local-mode-backend-url')).toHaveTextContent(
        'http://127.0.0.1:43117'
      )
    );
  });

  it('hides the sub-toggles while local mode is off', async () => {
    mockRpc(status({ enabled: false, active: false }));
    renderWithProviders(<LocalModeSection />);

    await waitFor(() => expect(screen.getByTestId('local-mode-toggle')).toBeInTheDocument());
    expect(screen.queryByTestId('local-mode-defaults-toggle')).not.toBeInTheDocument();
    expect(screen.queryByTestId('local-mode-proxy-toggle')).not.toBeInTheDocument();
  });

  it('surfaces a load failure instead of rendering an empty list', async () => {
    callCoreRpc.mockRejectedValue(new Error('core unreachable'));
    renderWithProviders(<LocalModeSection />);

    await waitFor(() => expect(screen.getByText(/core unreachable/)).toBeInTheDocument());
    expect(screen.queryByTestId('local-mode-service-list')).not.toBeInTheDocument();
  });
});
