import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import ComposerEffortPill, { EMPTY_EFFORT } from '../ComposerEffortPill';

describe('ComposerEffortPill', () => {
  it('shows the whole scale without needing to be opened', () => {
    renderWithProviders(<ComposerEffortPill value={EMPTY_EFFORT} onChange={vi.fn()} />);

    // The control IS the scale — the previous popover hid the current setting
    // behind a click, which is what made it invisible.
    expect(screen.getByRole('radio', { name: 'Auto' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Quick' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Balanced' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Thorough' })).toBeInTheDocument();
  });

  it('checks Auto while the effort is unset', () => {
    renderWithProviders(<ComposerEffortPill value={EMPTY_EFFORT} onChange={vi.fn()} />);

    // Unset means "send nothing and let the run preset decide". That is a real
    // choice and has its own stop, so the control is never lit-nowhere — which
    // is what made it read as decorative.
    expect(screen.getByRole('radio', { name: 'Auto' })).toHaveAttribute('aria-checked', 'true');
    for (const name of ['Quick', 'Balanced', 'Thorough']) {
      expect(screen.getByRole('radio', { name })).toHaveAttribute('aria-checked', 'false');
    }
  });

  it('returns to sending nothing when Auto is picked', () => {
    const onChange = vi.fn();
    renderWithProviders(<ComposerEffortPill value={{ effort: 'high' }} onChange={onChange} />);

    fireEvent.click(screen.getByRole('radio', { name: 'Auto' }));

    // `null` is what keeps `reasoning_effort` off the wire, which is the only
    // way the run preset's own reasoning level survives the merge in the core.
    expect(onChange).toHaveBeenCalledWith({ effort: null });
  });

  it('reports the wire value, not the label', () => {
    const onChange = vi.fn();
    renderWithProviders(<ComposerEffortPill value={EMPTY_EFFORT} onChange={onChange} />);

    fireEvent.click(screen.getByRole('radio', { name: 'Thorough' }));

    // `high` is the `reasoning_effort` the provider reads; the label is display.
    expect(onChange).toHaveBeenCalledWith({ effort: 'high' });
  });

  it('checks the stop matching a set effort', () => {
    renderWithProviders(<ComposerEffortPill value={{ effort: 'medium' }} onChange={vi.fn()} />);

    expect(screen.getByRole('radio', { name: 'Balanced' })).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByRole('radio', { name: 'Quick' })).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('radio', { name: 'Auto' })).toHaveAttribute('aria-checked', 'false');
  });
});
