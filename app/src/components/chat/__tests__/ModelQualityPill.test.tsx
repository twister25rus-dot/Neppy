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

  it('draws a trailing chevron that does not shrink', () => {
    const { container } = render(<ModelQualityPill />);
    // The composer redesign brings the dropdown chevron back. It is
    // `shrink-0` so a long model name truncates instead of clipping it (#3292).
    const chevron = container.querySelector('svg');
    expect(chevron).not.toBeNull();
    expect(chevron).toHaveClass('shrink-0');
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
