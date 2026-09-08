import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import MlxPanel from './MlxPanel';

const callCoreRpc = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: (arg: { method: string; params: unknown }) => callCoreRpc(arg),
}));

// The panel renders translation keys verbatim under this mock, which keeps the
// assertions readable and independent of English copy. The two interpolated
// keys keep their placeholders, because the component substitutes into them
// and a bare key would silently swallow the value being asserted on.
const INTERPOLATED: Record<string, string> = {
  'mlx.memoryUsage': '{used} GiB of {budget} GiB used',
  'mlx.availableModels': 'Available models ({count})',
};

vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string) => INTERPOLATED[key] ?? key }),
}));

interface ServerOverrides {
  id?: string;
  state?: string;
  port?: number | null;
  loaded_model?: string | null;
  resident_gib?: number | null;
  detail?: string | null;
  command?: string[];
  models?: string[];
}

function server(overrides: ServerOverrides = {}) {
  return {
    id: 'primary',
    kind: 'vlm',
    state: 'stopped',
    port: null,
    base_url: null,
    pid: null,
    loaded_model: null,
    resident_gib: null,
    estimated_gib: null,
    models: [],
    detail: null,
    command: [],
    ...overrides,
  };
}

function status(overrides: Record<string, unknown> = {}) {
  return {
    enabled: true,
    embeddings_backend: 'ollama',
    memory_used_gib: 0,
    memory_budget_gib: 25.2,
    problems: [],
    servers: [server()],
    ...overrides,
  };
}

describe('MlxPanel', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
  });

  it('shows memory use against the shared budget', async () => {
    // This is the number that decides whether a second server can start, so
    // it is the one thing the panel must always surface.
    callCoreRpc.mockResolvedValue(status({ memory_used_gib: 14.4 }));
    render(<MlxPanel />);

    await waitFor(() => expect(screen.getByText('mlx.memoryTitle')).toBeInTheDocument());
    expect(screen.getByText('14.4', { exact: false })).toBeInTheDocument();
    expect(screen.getByText('25.2', { exact: false })).toBeInTheDocument();
  });

  it('offers Start for a stopped server and Stop for a running one', async () => {
    callCoreRpc.mockResolvedValue(status());
    const { unmount } = render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.start')).toBeInTheDocument());
    expect(screen.queryByText('mlx.stop')).not.toBeInTheDocument();
    unmount();

    callCoreRpc.mockResolvedValue(status({ servers: [server({ state: 'ready', port: 8794 })] }));
    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.stop')).toBeInTheDocument());
    expect(screen.getByText('mlx.unload')).toBeInTheDocument();
    expect(screen.queryByText('mlx.start')).not.toBeInTheDocument();
  });

  it('starts the addressed server by id', async () => {
    callCoreRpc.mockResolvedValue(status({ servers: [server({ id: 'vision' })] }));
    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.start')).toBeInTheDocument());

    await userEvent.click(screen.getByText('mlx.start'));

    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mlx_start',
        params: { id: 'vision' },
      })
    );
  });

  it('surfaces a refused start verbatim', async () => {
    // The core's refusal names the shortfall and the next step, so paraphrasing
    // it in the UI would throw away the useful part.
    const refusal =
      'mlx-community/Qwen3.8-27B-nvfp4 needs about 14.4 GiB but only 9.1 GiB of the 25.2 GiB MLX budget is free. Stop another MLX server first, or raise mlx.memory_budget_gib.';
    callCoreRpc.mockImplementation((arg: { method: string }) => {
      if (arg.method === 'openhuman.mlx_start') return Promise.reject(new Error(refusal));
      return Promise.resolve(status());
    });

    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.start')).toBeInTheDocument());
    await userEvent.click(screen.getByText('mlx.start'));

    await waitFor(() => expect(screen.getByText(refusal)).toBeInTheDocument());
  });

  it('reports configuration problems from the core', async () => {
    callCoreRpc.mockResolvedValue(status({ problems: ['duplicate [[mlx.server]] id `primary`'] }));
    render(<MlxPanel />);

    await waitFor(() =>
      expect(screen.getByText('duplicate [[mlx.server]] id `primary`')).toBeInTheDocument()
    );
  });

  it('says when the runtime is switched off', async () => {
    callCoreRpc.mockResolvedValue(status({ enabled: false }));
    render(<MlxPanel />);

    await waitFor(() => expect(screen.getByText('mlx.disabledNotice')).toBeInTheDocument());
  });

  it('loads the log tail only when details are opened', async () => {
    callCoreRpc.mockImplementation((arg: { method: string }) => {
      if (arg.method === 'openhuman.mlx_logs') {
        return Promise.resolve({ id: 'primary', lines: ['[err] out of memory'] });
      }
      return Promise.resolve(status({ servers: [server({ state: 'ready', port: 8794 })] }));
    });

    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.showDetails')).toBeInTheDocument());
    expect(callCoreRpc).not.toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.mlx_logs' })
    );

    await userEvent.click(screen.getByText('mlx.showDetails'));

    await waitFor(() => expect(screen.getByText('[err] out of memory')).toBeInTheDocument());
  });

  it('renders the redacted command rather than hiding what ran', async () => {
    callCoreRpc.mockImplementation((arg: { method: string }) => {
      if (arg.method === 'openhuman.mlx_logs') {
        return Promise.resolve({ id: 'primary', lines: [] });
      }
      return Promise.resolve(
        status({
          servers: [
            server({
              state: 'ready',
              port: 8794,
              command: ['--host', '127.0.0.1', '--port', '8794', '--api-key', '***'],
            }),
          ],
        })
      );
    });

    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.showDetails')).toBeInTheDocument());
    await userEvent.click(screen.getByText('mlx.showDetails'));

    await waitFor(() => expect(screen.getByText(/--api-key \*\*\*/)).toBeInTheDocument());
  });
});
