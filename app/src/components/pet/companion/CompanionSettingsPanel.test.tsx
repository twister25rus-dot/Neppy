import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { HIGH_RISK_CATEGORIES } from '../../../services/api/petCompanionApi';
import { renderWithProviders } from '../../../test/test-utils';
import { makeCompanionSettings, makeCompanionStatus, makeObservation } from './companionFixtures';
import CompanionSettingsPanel from './CompanionSettingsPanel';
import { makeFakeCompanion } from './companionTestUtils';

vi.mock('../../../services/api/petCompanionApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../services/api/petCompanionApi')>();
  return { ...actual, getCompanionData: vi.fn(), deleteCompanionData: vi.fn() };
});

describe('CompanionSettingsPanel', () => {
  beforeEach(() => vi.clearAllMocks());

  it('does not enable the companion until the consent dialog is accepted', async () => {
    const companion = makeFakeCompanion({
      settings: makeCompanionSettings({ enabled: false }),
      status: makeCompanionStatus({
        permissions: { accessibility: 'granted', screen_recording: 'granted' },
      }),
      displayState: 'off',
    });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);

    fireEvent.click(screen.getByTestId('companion-enabled-switch'));
    expect(await screen.findByTestId('companion-consent')).toBeInTheDocument();
    expect(companion.saveSettings).not.toHaveBeenCalled();

    fireEvent.click(screen.getByTestId('companion-consent-accept'));
    await waitFor(() => expect(companion.saveSettings).toHaveBeenCalledTimes(1));
    expect(companion.saveSettings).toHaveBeenCalledWith({
      enabled: true,
      sources: { app_window: true, selection: true, clipboard: true, screen_capture: true },
    });
  });

  it('pre-ticks every source in the consent dialog and explains privacy', async () => {
    const companion = makeFakeCompanion({ settings: makeCompanionSettings({ enabled: false }) });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByTestId('companion-enabled-switch'));
    await screen.findByTestId('companion-consent');

    for (const key of ['app_window', 'selection', 'clipboard', 'screen_capture']) {
      expect(screen.getByTestId(`companion-consent-${key}`)).toBeChecked();
    }
    const privacy = screen.getByTestId('companion-consent-private');
    expect(privacy).toHaveTextContent(/passwords, one-time codes, API keys/);
    expect(privacy).toHaveTextContent(/never kept/);
    const dialog = screen.getByTestId('companion-consent');
    expect(
      within(dialog).getByText(/deleted straight away and is never stored or sent/)
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText(/macOS will ask for Accessibility and Screen Recording/)
    ).toBeInTheDocument();
  });

  it('sends only the sources the user left ticked', async () => {
    const companion = makeFakeCompanion({ settings: makeCompanionSettings({ enabled: false }) });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByTestId('companion-enabled-switch'));
    await screen.findByTestId('companion-consent');
    fireEvent.click(screen.getByTestId('companion-consent-clipboard'));
    fireEvent.click(screen.getByTestId('companion-consent-accept'));
    await waitFor(() =>
      expect(companion.saveSettings).toHaveBeenCalledWith({
        enabled: true,
        sources: { app_window: true, selection: true, clipboard: false, screen_capture: true },
      })
    );
  });

  it('requests missing OS permissions only after consent is accepted', async () => {
    const companion = makeFakeCompanion({
      settings: makeCompanionSettings({ enabled: false }),
      status: makeCompanionStatus({
        permissions: { accessibility: 'denied', screen_recording: 'unknown' },
      }),
    });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByTestId('companion-enabled-switch'));
    await screen.findByTestId('companion-consent');
    expect(companion.requestPermission).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('companion-consent-accept'));
    await waitFor(() =>
      expect(companion.requestPermission).toHaveBeenCalledWith('screen_recording')
    );
    expect(companion.requestPermission).toHaveBeenCalledWith('accessibility');
  });

  it('cancelling the consent dialog leaves the companion off', async () => {
    const companion = makeFakeCompanion({ settings: makeCompanionSettings({ enabled: false }) });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByTestId('companion-enabled-switch'));
    fireEvent.click(await screen.findByTestId('companion-consent-cancel'));
    await waitFor(() => expect(screen.queryByTestId('companion-consent')).toBeNull());
    expect(companion.saveSettings).not.toHaveBeenCalled();
  });

  it('turning the companion off needs no consent', async () => {
    const companion = makeFakeCompanion();
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByTestId('companion-enabled-switch'));
    await waitFor(() => expect(companion.saveSettings).toHaveBeenCalledWith({ enabled: false }));
    expect(screen.queryByTestId('companion-consent')).toBeNull();
  });

  it('asks for the OS permission when a source needing it is switched on', async () => {
    const companion = makeFakeCompanion({
      settings: makeCompanionSettings({
        enabled: true,
        sources: { app_window: true, selection: true, clipboard: true, screen_capture: false },
      }),
      status: makeCompanionStatus({
        permissions: { accessibility: 'granted', screen_recording: 'denied' },
      }),
    });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByTestId('companion-source-screen_capture-toggle'));
    await waitFor(() =>
      expect(companion.saveSettings).toHaveBeenCalledWith({
        sources: { app_window: true, selection: true, clipboard: true, screen_capture: true },
      })
    );
    await waitFor(() =>
      expect(companion.requestPermission).toHaveBeenCalledWith('screen_recording')
    );
  });

  it('does not ask for a permission that is already granted', async () => {
    const companion = makeFakeCompanion({
      settings: makeCompanionSettings({
        enabled: true,
        sources: { app_window: false, selection: true, clipboard: true, screen_capture: true },
      }),
    });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByTestId('companion-source-app_window-toggle'));
    await waitFor(() => expect(companion.saveSettings).toHaveBeenCalled());
    expect(companion.requestPermission).not.toHaveBeenCalled();
  });

  it('defaults the cloud model on and lets the user switch to local only', async () => {
    const companion = makeFakeCompanion();
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    const sw = screen.getByTestId('companion-cloud-switch');
    expect(sw).toHaveAttribute('data-state', 'checked');
    fireEvent.click(sw);
    await waitFor(() =>
      expect(companion.saveSettings).toHaveBeenCalledWith({ allow_cloud_model: false })
    );
  });

  it('locks every high-risk category as "Always asks"', () => {
    renderWithProviders(<CompanionSettingsPanel companion={makeFakeCompanion()} />);
    for (const cat of HIGH_RISK_CATEGORIES) {
      const row = screen.getByTestId(`companion-category-${cat}`);
      const select = within(row).getByTestId(`companion-category-${cat}-level`);
      expect(select).toBeDisabled();
      expect(select).toHaveDisplayValue('Always asks');
    }
  });

  it('saves a changed editable category level and the overall level', async () => {
    const companion = makeFakeCompanion();
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.change(screen.getByTestId('companion-category-explain-level'), {
      target: { value: '3' },
    });
    await waitFor(() =>
      expect(companion.saveSettings).toHaveBeenCalledWith({ category_levels: { explain: 3 } })
    );
    fireEvent.click(screen.getByTestId('companion-level-2'));
    await waitFor(() => expect(companion.saveSettings).toHaveBeenCalledWith({ level: 2 }));
  });

  it('adds and removes excluded apps, replacing the whole list', async () => {
    const companion = makeFakeCompanion();
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.change(screen.getByTestId('companion-exclusion-apps-input'), {
      target: { value: 'com.example.secret' },
    });
    fireEvent.click(screen.getByTestId('companion-exclusion-apps-add'));
    await waitFor(() =>
      expect(companion.saveSettings).toHaveBeenCalledWith({
        excluded_apps: ['com.1password.1password', 'com.example.secret'],
      })
    );
    fireEvent.click(screen.getByRole('button', { name: 'Remove: com.1password.1password' }));
    await waitFor(() => expect(companion.saveSettings).toHaveBeenCalledWith({ excluded_apps: [] }));
  });

  it('offers recently seen apps as a quick add', async () => {
    const companion = makeFakeCompanion({
      status: makeCompanionStatus({
        recent: [makeObservation({ app_name: 'Notes', bundle_id: 'com.apple.Notes' })],
      }),
    });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByRole('button', { name: 'com.apple.Notes' }));
    await waitFor(() =>
      expect(companion.saveSettings).toHaveBeenCalledWith({
        excluded_apps: ['com.1password.1password', 'com.apple.Notes'],
      })
    );
  });

  it('rejects an invalid window title regex without saving', async () => {
    const companion = makeFakeCompanion();
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.change(screen.getByTestId('companion-exclusion-titles-input'), {
      target: { value: 're:(' },
    });
    fireEvent.click(screen.getByTestId('companion-exclusion-titles-add'));
    expect(await screen.findByRole('alert')).toHaveTextContent('not a valid regular expression');
    expect(companion.saveSettings).not.toHaveBeenCalled();
  });

  it('adds a valid window title rule', async () => {
    const companion = makeFakeCompanion();
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.change(screen.getByTestId('companion-exclusion-titles-input'), {
      target: { value: 're:^Bank' },
    });
    fireEvent.click(screen.getByTestId('companion-exclusion-titles-add'));
    await waitFor(() =>
      expect(companion.saveSettings).toHaveBeenCalledWith({
        excluded_title_patterns: ['incognito', 'private browsing', 're:^Bank'],
      })
    );
  });

  it('shows the configured shortcuts and unavailable sources as disabled', () => {
    renderWithProviders(<CompanionSettingsPanel companion={makeFakeCompanion()} />);
    expect(screen.getByTestId('companion-hotkey-pause')).toHaveTextContent('Ctrl+Alt+Shift+P');
    const unavailable = screen.getByTestId('companion-unavailable-browser_content');
    expect(unavailable).toHaveTextContent('Browser page content');
    expect(unavailable).toHaveTextContent('Not available yet');
    expect(within(unavailable).getByRole('checkbox')).toBeDisabled();
  });

  it('shows a Grant button for a switched-on source whose permission is missing', async () => {
    const companion = makeFakeCompanion({
      status: makeCompanionStatus({
        permissions: { accessibility: 'denied', screen_recording: 'granted' },
      }),
    });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getAllByTestId('companion-settings-grant-accessibility')[0]);
    expect(companion.requestPermission).toHaveBeenCalledWith('accessibility');
  });

  it('clamps the screen interval before saving', async () => {
    const companion = makeFakeCompanion();
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    const field = within(screen.getByTestId('companion-screen-interval')).getByRole('spinbutton');
    fireEvent.change(field, { target: { value: '2' } });
    fireEvent.blur(field);
    await waitFor(() =>
      expect(companion.saveSettings).toHaveBeenCalledWith({ screen_min_interval_secs: 10 })
    );
  });

  it('reports a save failure in the panel', async () => {
    const companion = makeFakeCompanion({
      saveSettings: vi.fn().mockRejectedValue(new Error('invalid')),
    });
    renderWithProviders(<CompanionSettingsPanel companion={companion} />);
    fireEvent.click(screen.getByTestId('companion-cloud-switch'));
    expect(await screen.findByTestId('companion-settings-error')).toHaveTextContent(
      'Could not save companion settings'
    );
  });

  it('says so when the companion cannot be loaded', () => {
    renderWithProviders(
      <CompanionSettingsPanel companion={makeFakeCompanion({ settings: null })} />
    );
    expect(screen.getByTestId('companion-settings-unavailable')).toBeInTheDocument();
  });
});
