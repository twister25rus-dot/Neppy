import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import ComposerEffortPill, { EMPTY_EFFORT } from '../ComposerEffortPill';

describe('ComposerEffortPill', () => {
  it('shows the whole scale without needing to be opened', () => {
    renderWithProviders(<ComposerEffortPill value={EMPTY_EFFORT} onChange={vi.fn()} />);

    // The control IS the scale — the previous popover hid the current setting
    // behind a click, which is what made it invisible.
    expect(screen.getByRole('radio', { name: 'Quick' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Balanced' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Thorough' })).toBeInTheDocument();
  });

  it('marks nothing checked while the effort is unset', () => {
    renderWithProviders(<ComposerEffortPill value={EMPTY_EFFORT} onChange={vi.fn()} />);

    // Unset means "leave it to the provider" and sends nothing, so no stop may
    // claim to be the chosen one — even though one renders as current.
    for (const name of ['Quick', 'Balanced', 'Thorough']) {
      expect(screen.getByRole('radio', { name })).toHaveAttribute('aria-checked', 'false');
    }
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
  });
});
