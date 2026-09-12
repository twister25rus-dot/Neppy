import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders as render } from '../../../test/test-utils';
import ModelQualityPill from '../ModelQualityPill';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (k: string) => k }) }));

describe('ModelQualityPill', () => {
  it('renders with model name', () => {
    render(<ModelQualityPill />);
    expect(screen.getByText('Neppy')).toBeInTheDocument();
  });

  it('draws no chevron', () => {
    const { container } = render(<ModelQualityPill />);
    // The pill carried a trailing chevron, which #3292 had to add padding for
    // so it would not clip. It is gone: the model name alone reads as the
    // control, and with nothing trailing there is nothing left to clip.
    expect(container.querySelector('svg')).toBeNull();
  });

  it('keeps the pill padding and shape the label needs', () => {
    render(<ModelQualityPill />);
    const button = screen.getByRole('button', { name: 'composer.modelSelector' });
    expect(button).toHaveClass('px-2');
    expect(button).toHaveClass('rounded-md');
  });

  it('has model selector aria-label', () => {
    render(<ModelQualityPill />);
    expect(screen.getByRole('button', { name: 'composer.modelSelector' })).toBeInTheDocument();
  });

  it('applies optional className', () => {
    const { container } = render(<ModelQualityPill className="my-custom-class" />);
    expect(container.firstChild).toHaveClass('my-custom-class');
  });
});
