import { screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import SettingsTabbedPage from './SettingsTabbedPage';

vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));

const renderPage = (narrow?: boolean) =>
  renderWithProviders(
    <SettingsTabbedPage title="LLM" narrow={narrow}>
      <div data-testid="page-body">body</div>
    </SettingsTabbedPage>
  );

/**
 * Connector pages cap their width; every other settings page still fills the
 * pane, because the layout deliberately does not cap its column and a gutter
 * there is what made settings look inset next to the other routed pages.
 */
describe('SettingsTabbedPage narrow', () => {
  it('caps the page when narrow is set', () => {
    renderPage(true);

    const page = screen.getByTestId('settings-page-narrow');
    expect(page.className).toContain('mx-auto');
    expect(page.className).toContain('max-w-');
    expect(screen.getByTestId('page-body')).toBeInTheDocument();
  });

  it('fills the pane by default, so existing pages are untouched', () => {
    renderPage();

    expect(screen.queryByTestId('settings-page-narrow')).not.toBeInTheDocument();
    expect(screen.getByTestId('page-body')).toBeInTheDocument();
  });
});
