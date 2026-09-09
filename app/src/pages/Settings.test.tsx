import { fireEvent, screen } from '@testing-library/react';
import { Route, Routes } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../test/test-utils';
import Settings from './Settings';

vi.mock('../components/settings/settingsRouteElements', async () => {
  const { Route: MockRoute } =
    await vi.importActual<typeof import('react-router-dom')>('react-router-dom');

  return {
    settingsRouteElements: () => (
      <>
        <MockRoute index element={<div data-testid="settings-test-panel">Index</div>} />
        <MockRoute path="account" element={<div data-testid="settings-test-panel">Account</div>} />
      </>
    ),
  };
});

function SettingsAtRoute({
  presentation = 'page',
  onClose,
}: {
  presentation?: 'page' | 'dialog';
  onClose?: () => void;
}) {
  return (
    <Routes>
      <Route
        path="/settings/*"
        element={<Settings presentation={presentation} onClose={onClose} />}
      />
    </Routes>
  );
}

describe('<Settings /> presentation', () => {
  it('renders a self-contained two-pane dialog and closes from the X button', () => {
    const onClose = vi.fn();
    renderWithProviders(<SettingsAtRoute presentation="dialog" onClose={onClose} />, {
      initialEntries: ['/settings/account'],
    });

    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(screen.getByTestId('settings-dialog-window')).toBeInTheDocument();
    expect(screen.getByTestId('settings-dialog-sidebar')).toBeInTheDocument();
    expect(screen.getByTestId('settings-test-panel')).toHaveTextContent('Account');

    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('keeps the full-page presentation free of dialog chrome', () => {
    renderWithProviders(<SettingsAtRoute />, { initialEntries: ['/settings/account'] });

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByTestId('settings-dialog-window')).not.toBeInTheDocument();
    expect(screen.queryByTestId('settings-dialog-sidebar')).not.toBeInTheDocument();
    expect(screen.getByTestId('settings-test-panel')).toHaveTextContent('Account');
  });

  it('dismisses the dialog with Escape', () => {
    const onClose = vi.fn();
    renderWithProviders(<SettingsAtRoute presentation="dialog" onClose={onClose} />, {
      initialEntries: ['/settings/account'],
    });

    fireEvent.keyDown(document, { key: 'Escape', code: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
