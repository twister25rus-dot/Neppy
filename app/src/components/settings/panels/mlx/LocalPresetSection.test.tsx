import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import LocalPresetSection from './LocalPresetSection';

const getLocalModelPreset = vi.fn();
const setLocalModelPreset = vi.fn();
vi.mock('../../../../services/api/localPresetApi', () => ({
  getLocalModelPreset: () => getLocalModelPreset(),
  setLocalModelPreset: (preset: string) => setLocalModelPreset(preset),
}));

vi.mock('../../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

describe('LocalPresetSection', () => {
  beforeEach(() => {
    getLocalModelPreset.mockReset().mockResolvedValue('auto');
    setLocalModelPreset.mockReset().mockResolvedValue(undefined);
  });

  it('checks the preset the core already holds', async () => {
    getLocalModelPreset.mockResolvedValue('deep');
    render(<LocalPresetSection />);

    await waitFor(() => {
      expect(screen.getByTestId('local-preset-deep')).toHaveAttribute('aria-checked', 'true');
    });
    expect(screen.getByTestId('local-preset-auto')).toHaveAttribute('aria-checked', 'false');
  });

  it('moves the selection on click and persists it', async () => {
    render(<LocalPresetSection />);
    await waitFor(() => expect(getLocalModelPreset).toHaveBeenCalled());

    await userEvent.click(screen.getByTestId('local-preset-fast'));

    // Optimistic: the tick moves at the speed of the click, not of the save.
    expect(screen.getByTestId('local-preset-fast')).toHaveAttribute('aria-checked', 'true');
    expect(setLocalModelPreset).toHaveBeenCalledWith('fast');
  });

  it('puts the selection back when the save fails', async () => {
    setLocalModelPreset.mockRejectedValue(new Error('config is read only'));
    render(<LocalPresetSection />);
    await waitFor(() => expect(getLocalModelPreset).toHaveBeenCalled());

    await userEvent.click(screen.getByTestId('local-preset-deep'));

    // Showing a choice that did not save is worse than showing the old one.
    await waitFor(() => {
      expect(screen.getByTestId('local-preset-auto')).toHaveAttribute('aria-checked', 'true');
    });
    expect(screen.getByTestId('local-preset-deep')).toHaveAttribute('aria-checked', 'false');
  });

  it('keeps every option reachable while a save is in flight', async () => {
    let release = () => {};
    setLocalModelPreset.mockReturnValue(
      new Promise<void>(resolve => {
        release = resolve;
      })
    );
    render(<LocalPresetSection />);
    await waitFor(() => expect(getLocalModelPreset).toHaveBeenCalled());

    await userEvent.click(screen.getByTestId('local-preset-balanced'));

    // The whole grid used to be `disabled` during a save, which dimmed even the
    // button just pressed and read as the control locking up.
    expect(screen.getByTestId('local-preset-deep')).not.toBeDisabled();
    expect(screen.getByTestId('local-preset-balanced')).toHaveAttribute('aria-busy', 'true');
    release();
  });

  it('is one tab stop, the way a radiogroup is expected to be', async () => {
    render(<LocalPresetSection />);
    await waitFor(() => expect(getLocalModelPreset).toHaveBeenCalled());

    expect(screen.getByTestId('local-preset-auto')).toHaveAttribute('tabindex', '0');
    expect(screen.getByTestId('local-preset-fast')).toHaveAttribute('tabindex', '-1');
  });

  it('moves between options with the arrow keys', async () => {
    render(<LocalPresetSection />);
    await waitFor(() => expect(getLocalModelPreset).toHaveBeenCalled());

    screen.getByTestId('local-preset-auto').focus();
    await userEvent.keyboard('{ArrowRight}');

    // `fast` follows `auto` in the presets list.
    expect(setLocalModelPreset).toHaveBeenCalledWith('fast');
    expect(screen.getByTestId('local-preset-fast')).toHaveAttribute('aria-checked', 'true');
  });

  it('wraps from the first option to the last', async () => {
    render(<LocalPresetSection />);
    await waitFor(() => expect(getLocalModelPreset).toHaveBeenCalled());

    screen.getByTestId('local-preset-auto').focus();
    await userEvent.keyboard('{ArrowLeft}');

    expect(setLocalModelPreset).toHaveBeenCalledWith('maximum_quality');
  });

  it('renders the panel even when the stored preset cannot be read', async () => {
    getLocalModelPreset.mockRejectedValue(new Error('unreadable'));
    render(<LocalPresetSection />);

    // Unreadable means "show the default", not "block the panel".
    await waitFor(() => {
      expect(screen.getByTestId('local-preset-auto')).toHaveAttribute('aria-checked', 'true');
    });
  });
});
