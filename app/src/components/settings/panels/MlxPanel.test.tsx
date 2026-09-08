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
  configured_model?: string | null;
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
    configured_model: null,
    ...overrides,
  };
}

function cacheListing(overrides: Record<string, unknown> = {}) {
  return {
    models: [
      { id: 'mlx-community/Qwen3.8-27B-nvfp4', size_gib: 11.8, looks_like_mlx: true },
      { id: 'amazon/chronos-bolt-small', size_gib: 0.18, looks_like_mlx: false },
    ],
    total_gib: 11.98,
    cache_dir: '/Users/test/.cache/huggingface/hub',
    ...overrides,
  };
}

/** Route a mocked RPC call to the right fixture. */
function router(handlers: Record<string, unknown>) {
  return (arg: { method: string }) => {
    if (arg.method in handlers) return Promise.resolve(handlers[arg.method]);
    if (arg.method === 'openhuman.mlx_models_list') return Promise.resolve(cacheListing());
    return Promise.resolve(status());
  };
}

function status(overrides: Record<string, unknown> = {}) {
  return {
    enabled: true,
    embeddings_backend: 'ollama',
    memory_used_gib: 0,
    memory_budget_gib: 25.2,
    problems: [],
    chat_provider: 'openhuman',
    chat_uses_mlx: false,
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
    callCoreRpc.mockImplementation(
      router({ 'openhuman.mlx_status': status({ memory_used_gib: 14.4 }) })
    );
    render(<MlxPanel />);

    await waitFor(() => expect(screen.getByText('mlx.memoryTitle')).toBeInTheDocument());
    expect(screen.getByText('14.4', { exact: false })).toBeInTheDocument();
    expect(screen.getByText('25.2', { exact: false })).toBeInTheDocument();
  });

  it('offers Start for a stopped server and Stop for a running one', async () => {
    callCoreRpc.mockImplementation(router({}));
    const { unmount } = render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.start')).toBeInTheDocument());
    expect(screen.queryByText('mlx.stop')).not.toBeInTheDocument();
    unmount();

    callCoreRpc.mockImplementation(
      router({
        'openhuman.mlx_status': status({ servers: [server({ state: 'ready', port: 8794 })] }),
      })
    );
    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.stop')).toBeInTheDocument());
    expect(screen.getByText('mlx.unload')).toBeInTheDocument();
    expect(screen.queryByText('mlx.start')).not.toBeInTheDocument();
  });

  it('starts the addressed server by id', async () => {
    callCoreRpc.mockImplementation(
      router({ 'openhuman.mlx_status': status({ servers: [server({ id: 'vision' })] }) })
    );
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
      return router({})(arg);
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
    callCoreRpc.mockImplementation(router({ 'openhuman.mlx_status': status({ enabled: false }) }));
    render(<MlxPanel />);

    await waitFor(() => expect(screen.getByText('mlx.disabledNotice')).toBeInTheDocument());
  });

  it('loads the log tail only when details are opened', async () => {
    callCoreRpc.mockImplementation(
      router({
        'openhuman.mlx_logs': { id: 'primary', lines: ['[err] out of memory'] },
        'openhuman.mlx_status': status({ servers: [server({ state: 'ready', port: 8794 })] }),
      })
    );

    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.showDetails')).toBeInTheDocument());
    expect(callCoreRpc).not.toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.mlx_logs' })
    );

    await userEvent.click(screen.getByText('mlx.showDetails'));

    await waitFor(() => expect(screen.getByText('[err] out of memory')).toBeInTheDocument());
  });

  it('lists cached models with their disk cost', async () => {
    callCoreRpc.mockImplementation(router({}));
    render(<MlxPanel />);

    await waitFor(() => expect(screen.getByText('mlx.cacheTitle')).toBeInTheDocument());
    expect(screen.getByText('mlx-community/Qwen3.8-27B-nvfp4')).toBeInTheDocument();
    expect(screen.getByText('11.8 GiB')).toBeInTheDocument();
  });

  it('requires a second click before deleting a model', async () => {
    callCoreRpc.mockImplementation(router({}));
    render(<MlxPanel />);
    await waitFor(() => expect(screen.getAllByText('mlx.delete').length).toBeGreaterThan(0));

    await userEvent.click(screen.getAllByText('mlx.delete')[0]);

    // First click only arms the confirmation; nothing is deleted yet.
    expect(callCoreRpc).not.toHaveBeenCalledWith(
      expect.objectContaining({ method: 'openhuman.mlx_models_delete' })
    );

    await userEvent.click(screen.getByText('mlx.confirmDelete'));

    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mlx_models_delete',
        params: { model_id: 'mlx-community/Qwen3.8-27B-nvfp4' },
      })
    );
  });

  it('survives a malformed cache payload rather than blanking the panel', async () => {
    // A shape change upstream must not take down the server cards above it.
    callCoreRpc.mockImplementation(router({ 'openhuman.mlx_models_list': { unexpected: true } }));
    render(<MlxPanel />);

    await waitFor(() => expect(screen.getByText('mlx.start')).toBeInTheDocument());
    expect(screen.queryByText('mlx.cacheTitle')).not.toBeInTheDocument();
  });

  it('renders the redacted command rather than hiding what ran', async () => {
    callCoreRpc.mockImplementation(
      router({
        'openhuman.mlx_logs': { id: 'primary', lines: [] },
        'openhuman.mlx_status': status({
          servers: [
            server({
              state: 'ready',
              port: 8794,
              command: ['--host', '127.0.0.1', '--port', '8794', '--api-key', '***'],
            }),
          ],
        }),
      })
    );

    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.showDetails')).toBeInTheDocument());
    await userEvent.click(screen.getByText('mlx.showDetails'));

    await waitFor(() => expect(screen.getByText(/--api-key \*\*\*/)).toBeInTheDocument());
  });
});

// ── choosing a model, and routing chat to it ─────────────────────────────
//
// Both were missing from the first release: the panel could start a server
// but not say which checkpoint it should load, and starting one did nothing
// for chat because `local_ai.base_url` still pointed at another runtime.

describe('MlxPanel model selection and chat routing', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
  });

  it('offers every cached model as a choice', async () => {
    callCoreRpc.mockImplementation(router({}));
    render(<MlxPanel />);

    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());
    const options = screen.getAllByRole('option').map(option => option.textContent);

    expect(options.some(text => text?.includes('mlx-community/Qwen3.8-27B-nvfp4'))).toBe(true);
    // The size is on the option, because picking a model is a memory decision.
    expect(options.some(text => text?.includes('11.8 GiB'))).toBe(true);
    expect(options).toContain('mlx.modelNone');
  });

  it('sets the chosen model on the addressed server', async () => {
    callCoreRpc.mockImplementation(router({}));
    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByRole('combobox')).toBeInTheDocument());

    await userEvent.selectOptions(screen.getByRole('combobox'), 'mlx-community/Qwen3.8-27B-nvfp4');

    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mlx_set_model',
        params: { id: 'primary', model_id: 'mlx-community/Qwen3.8-27B-nvfp4' },
      })
    );
  });

  it('offers to route chat only to a running server', async () => {
    callCoreRpc.mockImplementation(router({}));
    const { unmount } = render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.start')).toBeInTheDocument());
    // Stopped: routing chat at it would name an address nothing is serving.
    expect(screen.queryByText('mlx.useForChat')).not.toBeInTheDocument();
    unmount();

    callCoreRpc.mockImplementation(
      router({
        'openhuman.mlx_status': status({ servers: [server({ state: 'ready', port: 8794 })] }),
      })
    );
    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.useForChat')).toBeInTheDocument());
  });

  it('routes chat to the addressed server', async () => {
    callCoreRpc.mockImplementation(
      router({
        'openhuman.mlx_status': status({
          servers: [server({ id: 'vision', state: 'ready', port: 8794 })],
        }),
      })
    );
    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.useForChat')).toBeInTheDocument());

    await userEvent.click(screen.getByText('mlx.useForChat'));

    await waitFor(() =>
      expect(callCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.mlx_use_for_chat',
        params: { id: 'vision' },
      })
    );
  });

  it('marks the server chat is actually using', async () => {
    // A started server that nothing routes to must look different from one
    // that is serving, which is precisely what the first release got wrong.
    callCoreRpc.mockImplementation(
      router({
        'openhuman.mlx_status': status({
          chat_provider: 'mlx:mlx-community/Qwen3.8-27B-nvfp4',
          chat_uses_mlx: true,
          servers: [
            server({
              state: 'ready',
              port: 8794,
              configured_model: 'mlx-community/Qwen3.8-27B-nvfp4',
            }),
          ],
        }),
      })
    );
    render(<MlxPanel />);

    await waitFor(() => expect(screen.getByText('mlx.servingChat')).toBeInTheDocument());
    // Already serving, so the button would be a no-op.
    expect(screen.queryByText('mlx.useForChat')).not.toBeInTheDocument();
  });

  it('surfaces a refusal to route chat with no model chosen', async () => {
    const refusal =
      '`primary` has no model selected. Choose one first, or the chat provider would name nothing.';
    callCoreRpc.mockImplementation((arg: { method: string }) => {
      if (arg.method === 'openhuman.mlx_use_for_chat') return Promise.reject(new Error(refusal));
      return router({
        'openhuman.mlx_status': status({ servers: [server({ state: 'ready', port: 8794 })] }),
      })(arg);
    });

    render(<MlxPanel />);
    await waitFor(() => expect(screen.getByText('mlx.useForChat')).toBeInTheDocument());
    await userEvent.click(screen.getByText('mlx.useForChat'));

    await waitFor(() => expect(screen.getByText(refusal)).toBeInTheDocument());
  });
});
