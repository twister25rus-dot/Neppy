import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import MlxQuickButton, { aggregateState, shortModelId } from './MlxQuickButton';

const callCoreRpc = vi.fn();
vi.mock('../../../services/coreRpcClient', () => ({
  callCoreRpc: (arg: { method: string; params: unknown }) => callCoreRpc(arg),
}));

const navigate = vi.fn();
vi.mock('react-router-dom', () => ({ useNavigate: () => navigate }));

// Keys render verbatim, so assertions stay independent of English copy.
const INTERPOLATED: Record<string, string> = {
  'mlx.memoryUsage': '{used} GiB of {budget} GiB used',
};
vi.mock('../../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string) => INTERPOLATED[key] ?? key }),
}));

vi.mock('../../../services/api/localPresetApi', () => ({
  getLocalModelPreset: () => Promise.resolve('auto'),
  setLocalModelPreset: () => Promise.resolve(),
}));

interface ServerShape {
  id?: string;
  state?: string;
  loaded_model?: string | null;
  settings?: Record<string, unknown>;
}

function server(over: ServerShape = {}) {
  return {
    id: 'primary',
    state: 'stopped',
    loaded_model: null,
    settings: { model: '', embedding_model: '' },
    ...over,
  };
}

function status(over: Record<string, unknown> = {}) {
  return {
    enabled: true,
    chat_uses_mlx: true,
    memory_used_gib: 0,
    memory_budget_gib: 25.2,
    servers: [server()],
    ...over,
  };
}

/** Answer whichever of the two load calls is asked for. */
function respond(statusPayload: unknown, models: { id: string; size_gib: number }[] = []) {
  callCoreRpc.mockImplementation(({ method }: { method: string }) => {
    if (method === 'openhuman.mlx_status') return Promise.resolve(statusPayload);
    if (method === 'openhuman.mlx_models_list') return Promise.resolve({ models });
    return Promise.resolve({});
  });
}

describe('aggregateState', () => {
  it('lets the worst state speak for the runtime', () => {
    expect(aggregateState([{ state: 'ready' }, { state: 'crashed' }] as never)).toBe('crashed');
    expect(aggregateState([{ state: 'ready' }, { state: 'degraded' }] as never)).toBe('degraded');
    expect(aggregateState([{ state: 'ready' }, { state: 'starting' }] as never)).toBe('starting');
    expect(aggregateState([{ state: 'ready' }] as never)).toBe('ready');
  });

  it('reads no servers as stopped rather than as unknown', () => {
    expect(aggregateState([])).toBe('stopped');
  });
});

describe('shortModelId', () => {
  it('keeps the identifying tail of a repo id', () => {
    expect(shortModelId('ornith-ai/Ornith-1.5-9B-MLX-8bit')).toBe('Ornith-1.5-9B-MLX-8bit');
  });

  it('truncates a tail too long to fit the menu', () => {
    const long = `org/${'x'.repeat(60)}`;
    expect(shortModelId(long).endsWith('…')).toBe(true);
    expect(shortModelId(long).length).toBe(34);
  });
});

describe('MlxQuickButton', () => {
  beforeEach(() => {
    callCoreRpc.mockReset();
    navigate.mockReset();
    respond(status());
  });

  it('shows the runtime state on the trigger without being opened', async () => {
    respond(status({ servers: [server({ state: 'ready' })] }));
    render(<MlxQuickButton />);

    // The dot answers "is it running?" at a glance, which is the whole reason
    // this sits in the shell rather than behind a settings page.
    await waitFor(() => {
      expect(screen.getByTestId('mlx-quick-dot').className).toContain('bg-success');
    });
  });

  it('starts a stopped server from the menu', async () => {
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));

    await userEvent.click(await screen.findByTestId('mlx-quick-toggle-primary'));

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mlx_start',
      params: { id: 'primary' },
    });
  });

  it('stops a running server from the same control', async () => {
    respond(status({ servers: [server({ state: 'ready' })] }));
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));

    await userEvent.click(await screen.findByTestId('mlx-quick-toggle-primary'));

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mlx_stop',
      params: { id: 'primary' },
    });
  });

  it('offers chat checkpoints and ticks the chosen one into the chat slot', async () => {
    respond(status(), [
      { id: 'ornith-ai/Ornith-1.5-9B-MLX-8bit', size_gib: 8.9 },
      // An embedder must not be offered as a chat model.
      { id: 'BAAI/bge-m3', size_gib: 0.4 },
    ]);
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));

    const select = await screen.findByTestId('mlx-quick-model-primary');
    expect(select).not.toHaveTextContent('bge-m3');

    await userEvent.selectOptions(select, 'ornith-ai/Ornith-1.5-9B-MLX-8bit');

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mlx_update_server',
      params: { id: 'primary', patch: { model: 'ornith-ai/Ornith-1.5-9B-MLX-8bit' } },
    });
  });

  it('clears the slot a checkpoint already held rather than assuming chat', async () => {
    respond(
      status({ servers: [server({ settings: { model: '', embedding_model: 'org/held-model' } })] }),
      [{ id: 'org/held-model', size_gib: 1 }]
    );
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));

    // The id is listed because a server already holds it, even though its name
    // does not read like a chat model.
    await userEvent.selectOptions(
      await screen.findByTestId('mlx-quick-model-primary'),
      'org/held-model'
    );

    expect(callCoreRpc).toHaveBeenCalledWith({
      method: 'openhuman.mlx_update_server',
      params: { id: 'primary', patch: { model: 'org/held-model' } },
    });
  });

  it('says a runtime could not be read instead of rendering it as stopped', async () => {
    callCoreRpc.mockRejectedValue(new Error('core is not listening'));
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));

    expect(await screen.findByTestId('mlx-quick-error')).toHaveTextContent('core is not listening');
  });

  it('surfaces a refused start verbatim', async () => {
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));
    callCoreRpc.mockRejectedValueOnce(new Error('not enough memory for a 15.0 GiB checkpoint'));

    await userEvent.click(await screen.findByTestId('mlx-quick-toggle-primary'));

    expect(await screen.findByTestId('mlx-quick-error')).toHaveTextContent('not enough memory');
  });

  it('keeps a route to the settings the gear it replaced used to offer', async () => {
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));

    await userEvent.click(await screen.findByText('mlx.quick.allSettings'));
    expect(navigate).toHaveBeenCalledWith('/settings');
  });

  it('deep links to the MLX tab rather than the providers tab', async () => {
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));

    await userEvent.click(await screen.findByText('mlx.quick.openSettings'));
    expect(navigate).toHaveBeenCalledWith('/connections?tab=llm#mlx');
  });

  it('says so when the runtime is configured off', async () => {
    respond(status({ enabled: false }));
    render(<MlxQuickButton />);
    await userEvent.click(await screen.findByTestId('mlx-quick-trigger'));

    expect(await screen.findByText('mlx.disabledNotice')).toBeInTheDocument();
  });
});
