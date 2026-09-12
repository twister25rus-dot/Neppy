import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { listProviderModels } from '../../../../services/api/aiSettingsApi';
import { ProviderModelPickerDialog } from './ProviderModelPickerDialog';

vi.mock('../../../../services/api/aiSettingsApi', () => ({ listProviderModels: vi.fn() }));

describe('ProviderModelPickerDialog', () => {
  it('fetches the catalog once when the caller re-renders with a fresh array', async () => {
    // The catalog effect used to depend on the `localModels` prop by reference.
    // A caller writing `localModels={[]}` inline handed it a new array on every
    // render, so the effect re-armed, cleared the catalog and refetched — which
    // re-rendered the caller, producing another `[]`. The list never settled:
    // it sat on "Loading models…" and restarted every couple of seconds.
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'gpt-4o-mini', owned_by: 'openai', context_window: 128_000 },
    ]);
    const providers = [
      {
        id: 'openai',
        slug: 'openai',
        label: 'OpenAI',
        endpoint: 'https://api.openai.com/v1',
        authStyle: 'bearer' as const,
        maskedKey: '••••',
      },
    ];
    const props = {
      cloudProviders: providers,
      ollamaRunning: false,
      claudeCodeEnabled: false,
      initial: { source: { kind: 'cloud' as const, providerSlug: 'openai' }, model: '' },
      onClose: () => {},
      onSelect: () => {},
    };

    // A fresh array literal each render is exactly what the caller wrote.
    const { rerender } = render(<ProviderModelPickerDialog {...props} localModels={[]} />);
    await waitFor(() => expect(listProviderModels).toHaveBeenCalled());
    rerender(<ProviderModelPickerDialog {...props} localModels={[]} />);
    rerender(<ProviderModelPickerDialog {...props} localModels={[]} />);

    expect(listProviderModels).toHaveBeenCalledTimes(1);
  });

  it('returns the provider-reported context window with a catalog selection', async () => {
    vi.mocked(listProviderModels).mockResolvedValue([
      { id: 'gpt-4o-mini', owned_by: 'openai', context_window: 128_000 },
    ]);
    const onSelect = vi.fn();

    render(
      <ProviderModelPickerDialog
        cloudProviders={[
          {
            id: 'openai',
            slug: 'openai',
            label: 'OpenAI',
            endpoint: 'https://api.openai.com/v1',
            authStyle: 'bearer',
            maskedKey: '••••',
          },
        ]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={null}
        onClose={() => {}}
        onSelect={onSelect}
      />
    );

    // Managed is the first source now, so reaching a provider's catalog means
    // selecting that provider — the same step a user takes.
    fireEvent.click(screen.getByRole('button', { name: /OpenAI/ }));

    fireEvent.change(await screen.findByRole('combobox', { name: 'Model' }), {
      target: { value: 'gpt-4o-mini' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Use this model' }));

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({
        source: { kind: 'cloud', providerSlug: 'openai' },
        model: 'gpt-4o-mini',
        contextWindow: 128_000,
      })
    );
  });

  /**
   * Managed must be reachable from every model picker. Without it, choosing any
   * specific model was a one-way door: nothing in the UI routed back to the
   * product's own model selection.
   */
  it('offers managed first and selects it without requiring a model id', async () => {
    const onSelect = vi.fn();

    render(
      <ProviderModelPickerDialog
        cloudProviders={[]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={null}
        onClose={() => {}}
        onSelect={onSelect}
      />
    );

    // Preselected, and its pane explains the choice rather than asking for one.
    expect(screen.getByTestId('model-picker-managed-pane')).toBeInTheDocument();
    expect(screen.queryByRole('combobox', { name: 'Model' })).toBeNull();

    const submit = screen.getByRole('button', { name: 'Use this model' });
    expect(submit).not.toBeDisabled();
    fireEvent.click(submit);

    await waitFor(() =>
      expect(onSelect).toHaveBeenCalledWith({
        source: { kind: 'managed' },
        model: '',
        contextWindow: null,
      })
    );
  });

  it('omits managed when the host opts out', () => {
    render(
      <ProviderModelPickerDialog
        allowManaged={false}
        cloudProviders={[]}
        localModels={[]}
        ollamaRunning={false}
        claudeCodeEnabled={false}
        initial={null}
        onClose={() => {}}
        onSelect={() => {}}
      />
    );

    expect(screen.queryByTestId('model-picker-managed-pane')).toBeNull();
  });
});
