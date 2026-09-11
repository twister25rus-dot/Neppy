import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import ChatPresetPill, { PRESETS } from '../ChatPresetPill';

describe('ChatPresetPill', () => {
  it('shows the current choice without opening anything', () => {
    renderWithProviders(<ChatPresetPill value="deep" onChange={vi.fn()} />);

    expect(screen.getByTestId('chat-preset-trigger')).toHaveTextContent('Deep');
  });

  it('falls back to Auto for a value it does not recognise', () => {
    // A preset written by a newer build must not blank the control.
    renderWithProviders(<ChatPresetPill value={'from_the_future' as never} onChange={vi.fn()} />);

    expect(screen.getByTestId('chat-preset-trigger')).toHaveTextContent('Auto');
  });

  // The menu opens on pointer events, not a bare click, so these drive it the
  // way a person does.
  it('offers every preset from the presets document', async () => {
    const user = userEvent.setup();
    renderWithProviders(<ChatPresetPill value="auto" onChange={vi.fn()} />);
    await user.click(screen.getByTestId('chat-preset-trigger'));

    for (const preset of PRESETS) {
      expect(screen.getByTestId(`chat-preset-${preset.id}`)).toBeInTheDocument();
    }
  });

  it('reports the chosen preset', async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    renderWithProviders(<ChatPresetPill value="auto" onChange={onChange} />);
    await user.click(screen.getByTestId('chat-preset-trigger'));
    await user.click(screen.getByTestId('chat-preset-long_context'));

    expect(onChange).toHaveBeenCalledWith('long_context');
  });
});
