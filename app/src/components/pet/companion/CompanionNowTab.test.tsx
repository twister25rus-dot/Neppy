import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import {
  makeCompanionSettings,
  makeCompanionStatus,
  makeObservation,
  makeSuggestion,
} from './companionFixtures';
import CompanionNowTab from './CompanionNowTab';
import { makeFakeCompanion } from './companionTestUtils';
import { useCompanion } from './useCompanion';

const api = vi.hoisted(() => ({
  getCompanionSettings: vi.fn(),
  getCompanionStatus: vi.fn(),
  fetchCompanionSuggestions: vi.fn(),
  updateCompanionSettings: vi.fn(),
  pauseCompanion: vi.fn(),
  resumeCompanion: vi.fn(),
  requestCompanionPermission: vi.fn(),
  actOnCompanionSuggestion: vi.fn(),
  getCompanionData: vi.fn(),
  deleteCompanionData: vi.fn(),
}));
vi.mock('../../../services/api/petCompanionApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../../../services/api/petCompanionApi')>();
  return { ...actual, ...api };
});

const socketHandlers = vi.hoisted(() => new Map<string, (...args: unknown[]) => void>());
vi.mock('../../../services/socketService', () => ({
  socketService: {
    on: (event: string, cb: (...args: unknown[]) => void) => socketHandlers.set(event, cb),
    off: (event: string) => socketHandlers.delete(event),
  },
}));

function Harness({ onOpenSettings = vi.fn() }: { onOpenSettings?: () => void }) {
  const companion = useCompanion();
  if (companion.loading) return <p>loading</p>;
  return <CompanionNowTab companion={companion} onOpenSettings={onOpenSettings} />;
}

const emit = (payload: unknown) => act(() => socketHandlers.get('pet:companion')?.(payload));

describe('CompanionNowTab', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    socketHandlers.clear();
    api.getCompanionSettings.mockResolvedValue(makeCompanionSettings({ enabled: true }));
    api.getCompanionStatus.mockResolvedValue(makeCompanionStatus());
    api.fetchCompanionSuggestions.mockResolvedValue([]);
  });

  it('shows the off card and opens settings when the companion is disabled', () => {
    const onOpenSettings = vi.fn();
    renderWithProviders(
      <CompanionNowTab
        companion={makeFakeCompanion({
          settings: makeCompanionSettings({ enabled: false }),
          displayState: 'off',
        })}
        onOpenSettings={onOpenSettings}
      />
    );
    expect(screen.getByTestId('companion-off')).toHaveTextContent('The desktop companion is off');
    fireEvent.click(screen.getByTestId('companion-open-settings'));
    expect(onOpenSettings).toHaveBeenCalled();
  });

  it('pauses over RPC and reflects the paused state from a socket event', async () => {
    api.pauseCompanion.mockResolvedValue(makeCompanionStatus({ state: 'paused' }));
    renderWithProviders(<Harness />);
    expect(await screen.findByTestId('companion-state')).toHaveTextContent('Observing');

    fireEvent.click(screen.getByTestId('companion-pause'));
    await waitFor(() => expect(api.pauseCompanion).toHaveBeenCalledWith(undefined, 'ui'));
    expect(await screen.findByTestId('companion-resume')).toBeInTheDocument();
    expect(screen.getByTestId('companion-state')).toHaveTextContent('Paused');

    // The tray or hotkey resumes: the UI follows the socket event.
    emit({ type: 'state', state: 'observing', paused: false });
    await waitFor(() =>
      expect(screen.getByTestId('companion-state')).toHaveTextContent('Observing')
    );
    expect(screen.getByTestId('companion-pause')).toBeInTheDocument();
  });

  it('pauses for one hour and resumes', async () => {
    api.pauseCompanion.mockResolvedValue(makeCompanionStatus({ state: 'paused' }));
    api.resumeCompanion.mockResolvedValue(makeCompanionStatus());
    renderWithProviders(<Harness />);
    fireEvent.click(await screen.findByTestId('companion-pause-1h'));
    await waitFor(() => expect(api.pauseCompanion).toHaveBeenCalledWith(60, 'ui'));
    fireEvent.click(await screen.findByTestId('companion-resume'));
    await waitFor(() => expect(api.resumeCompanion).toHaveBeenCalledWith('ui'));
    expect(await screen.findByTestId('companion-pause')).toBeInTheDocument();
  });

  it('distinguishes observing from observing the screen', async () => {
    renderWithProviders(<Harness />);
    expect(await screen.findByTestId('companion-state')).toHaveTextContent('Observing');
    emit({ type: 'state', state: 'observing', screen_capture_active: true });
    await waitFor(() =>
      expect(screen.getByTestId('companion-state')).toHaveTextContent('Observing screen')
    );
  });

  it('shows why the companion is suspended', async () => {
    api.getCompanionStatus.mockResolvedValue(
      makeCompanionStatus({ state: 'suspended', suspended_reason: 'no_indicator' })
    );
    renderWithProviders(<Harness />);
    expect(await screen.findByTestId('companion-suspended')).toHaveTextContent(
      'menu bar indicator is not visible'
    );
  });

  it('renders recent observations and drop counters', async () => {
    api.getCompanionStatus.mockResolvedValue(
      makeCompanionStatus({
        recent: [
          makeObservation({ app_name: 'Terminal', kind: 'clipboard' }),
          makeObservation({ app_name: 'Safari', kind: 'selection', dropped: 'secure_field' }),
        ],
        metrics: { drops_by_reason: { secure_field: 2, excluded_app: 1, paused: 0 } },
      })
    );
    renderWithProviders(<Harness />);
    const rows = await screen.findAllByTestId('companion-observation');
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent('Terminal');
    expect(rows[0]).toHaveTextContent('Clipboard');
    expect(rows[1]).toHaveTextContent('Skipped: Password field');
    const drops = screen.getByTestId('companion-drops');
    expect(drops).toHaveTextContent('Password field: 2');
    expect(drops).toHaveTextContent('Excluded app: 1');
    expect(drops).not.toHaveTextContent('Paused');
  });

  it('offers a Grant button for a missing permission and calls the RPC', async () => {
    api.getCompanionStatus.mockResolvedValue(
      makeCompanionStatus({ permissions: { accessibility: 'denied', screen_recording: 'granted' } })
    );
    api.requestCompanionPermission.mockResolvedValue({ state: 'denied', opened_settings: true });
    renderWithProviders(<Harness />);
    fireEvent.click(await screen.findByTestId('companion-grant-accessibility'));
    await waitFor(() =>
      expect(api.requestCompanionPermission).toHaveBeenCalledWith('accessibility')
    );
    expect(screen.queryByTestId('companion-grant-screen_recording')).toBeNull();
  });

  it('adds a suggestion that arrives over the socket and updates it in place', async () => {
    renderWithProviders(<Harness />);
    expect(await screen.findByTestId('companion-no-suggestions')).toBeInTheDocument();
    emit({ type: 'suggestion', suggestion: makeSuggestion({ id: 's9', headline: 'First' }) });
    expect(await screen.findByText('First')).toBeInTheDocument();
    emit({
      type: 'suggestion_update',
      suggestion: makeSuggestion({ id: 's9', headline: 'Second' }),
    });
    await screen.findByText('Second');
    expect(screen.getAllByTestId('companion-suggestion')).toHaveLength(1);
  });

  it('hides dismissed suggestions', async () => {
    api.fetchCompanionSuggestions.mockResolvedValue([
      makeSuggestion({ id: 'a', headline: 'Keep me' }),
      makeSuggestion({ id: 'b', headline: 'Gone', state: 'dismissed' }),
    ]);
    renderWithProviders(<Harness />);
    expect(await screen.findByText('Keep me')).toBeInTheDocument();
    expect(screen.queryByText('Gone')).toBeNull();
  });

  it('opens the retained data dialog', async () => {
    api.getCompanionData.mockResolvedValue({
      suggestions: [makeSuggestion({ headline: 'Stored one' })],
      actions: [],
      counts: { suggestions: 1, actions: 0 },
    });
    renderWithProviders(<Harness />);
    fireEvent.click(await screen.findByTestId('companion-view-data'));
    const data = await screen.findByTestId('companion-data');
    expect(await within(data).findByText('Stored one')).toBeInTheDocument();
  });
});
