import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import ComposerSamplingPill, { EMPTY_SAMPLING } from '../ComposerSamplingPill';

const open = () => fireEvent.click(screen.getByTestId('composer-sampling-trigger'));

describe('ComposerSamplingPill', () => {
  it('sends nothing for a field left empty', () => {
    const onChange = vi.fn();
    renderWithProviders(<ComposerSamplingPill value={EMPTY_SAMPLING} onChange={onChange} />);
    open();

    // An empty field means "leave it to the provider", not zero.
    expect(screen.getByLabelText('temperature')).toHaveValue(null);
    expect(screen.getByLabelText('top_p')).toHaveValue(null);
    expect(screen.getByLabelText('max_tokens')).toHaveValue(null);
  });

  it('reports a typed value under its wire name', () => {
    const onChange = vi.fn();
    renderWithProviders(<ComposerSamplingPill value={EMPTY_SAMPLING} onChange={onChange} />);
    open();
    fireEvent.change(screen.getByLabelText('top_p'), { target: { value: '0.9' } });

    expect(onChange).toHaveBeenCalledWith({ ...EMPTY_SAMPLING, topP: 0.9 });
  });

  it('clears a field back to unset rather than to zero', () => {
    const onChange = vi.fn();
    renderWithProviders(
      <ComposerSamplingPill value={{ ...EMPTY_SAMPLING, temperature: 0.7 }} onChange={onChange} />
    );
    open();
    fireEvent.change(screen.getByLabelText('temperature'), { target: { value: '' } });

    expect(onChange).toHaveBeenCalledWith({ ...EMPTY_SAMPLING, temperature: null });
  });

  it('resets everything at once, and only offers it when something is set', () => {
    const onChange = vi.fn();
    const { unmount } = renderWithProviders(
      <ComposerSamplingPill value={EMPTY_SAMPLING} onChange={onChange} />
    );
    open();
    expect(screen.getByRole('button', { name: /reset/i })).toBeDisabled();
    unmount();

    renderWithProviders(
      <ComposerSamplingPill
        value={{ temperature: 0.2, topP: 0.9, maxTokens: 512 }}
        onChange={onChange}
      />
    );
    open();
    fireEvent.click(screen.getByRole('button', { name: /reset/i }));

    expect(onChange).toHaveBeenCalledWith(EMPTY_SAMPLING);
  });
});
