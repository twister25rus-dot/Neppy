import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  makeCompanionSettings,
  makeCompanionStatus,
  makeSuggestion,
} from '../components/pet/companion/companionFixtures';
import {
  makeDigest,
  makeFeed,
  makeInbox,
  makePet,
  makeProposal,
} from '../components/pet/petFixtures';
import { renderWithProviders } from '../test/test-utils';
import PetPage from './PetPage';

const mockGetPet = vi.fn();
const mockFeed = vi.fn();
const mockInbox = vi.fn();
const mockRunNow = vi.fn();
const mockNotes = vi.fn();

vi.mock('../services/api/petApi', () => ({
  getPet: (...args: unknown[]) => mockGetPet(...args),
  fetchPetFeed: (...args: unknown[]) => mockFeed(...args),
  fetchPetInbox: (...args: unknown[]) => mockInbox(...args),
  runPetNow: (...args: unknown[]) => mockRunNow(...args),
  fetchPetNotes: (...args: unknown[]) => mockNotes(...args),
  buildPetDigestNow: vi.fn(),
}));
const mockCompanionSettings = vi.fn();
const mockCompanionStatus = vi.fn();
const mockCompanionSuggestions = vi.fn();
const mockCompanionPause = vi.fn();
const mockCompanionResume = vi.fn();

vi.mock('../services/api/petCompanionApi', async importOriginal => {
  const actual = await importOriginal<typeof import('../services/api/petCompanionApi')>();
  return {
    ...actual,
    getCompanionSettings: (...args: unknown[]) => mockCompanionSettings(...args),
    getCompanionStatus: (...args: unknown[]) => mockCompanionStatus(...args),
    fetchCompanionSuggestions: (...args: unknown[]) => mockCompanionSuggestions(...args),
    pauseCompanion: (...args: unknown[]) => mockCompanionPause(...args),
    resumeCompanion: (...args: unknown[]) => mockCompanionResume(...args),
  };
});
vi.mock('../services/socketService', () => ({ socketService: { on: vi.fn(), off: vi.fn() } }));

describe('PetPage', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockGetPet.mockResolvedValue(makePet());
    mockFeed.mockResolvedValue(makeFeed({ digests: [makeDigest()] }));
    mockInbox.mockResolvedValue(makeInbox());
    mockNotes.mockResolvedValue([]);
    mockRunNow.mockResolvedValue({ status: 'started' });
    mockCompanionSettings.mockResolvedValue(makeCompanionSettings({ enabled: false }));
    mockCompanionStatus.mockResolvedValue(makeCompanionStatus({ state: 'off' }));
    mockCompanionSuggestions.mockResolvedValue([]);
  });

  it('renders the pet header and the feed tab by default', async () => {
    renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
    expect(await screen.findByTestId('pet-name')).toHaveTextContent('Pip');
    expect(screen.getByTestId('pet-enabled-badge')).toHaveTextContent('On');
    expect(screen.getByTestId('pet-feed-tab')).toBeInTheDocument();
    expect(screen.getByText('Pet mode')).toBeInTheDocument();
  });

  it('opens the tab named in the query string', async () => {
    renderWithProviders(<PetPage />, { initialEntries: ['/pet?tab=inbox'] });
    expect(await screen.findByTestId('pet-inbox-tab')).toBeInTheDocument();
    expect(screen.queryByTestId('pet-feed-tab')).toBeNull();
  });

  it('falls back to the feed for an unknown tab', async () => {
    renderWithProviders(<PetPage />, { initialEntries: ['/pet?tab=bogus'] });
    expect(await screen.findByTestId('pet-feed-tab')).toBeInTheDocument();
  });

  it('shows a pending count on the Inbox tab', async () => {
    mockInbox.mockResolvedValue(makeInbox({ proposals: [makeProposal()] }));
    renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
    await waitFor(() => expect(screen.getByTestId('pet-tab-inbox')).toHaveTextContent('1'));
  });

  it('switches tabs when one is clicked', async () => {
    renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
    await screen.findByTestId('pet-feed-tab');
    fireEvent.mouseDown(screen.getByTestId('pet-tab-settings'));
    fireEvent.click(screen.getByTestId('pet-tab-settings'));
    expect(await screen.findByTestId('pet-settings-tab')).toBeInTheDocument();
  });

  it('runs a pass from the header button', async () => {
    renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
    fireEvent.click(await screen.findByTestId('pet-run-now'));
    await waitFor(() => expect(mockRunNow).toHaveBeenCalledWith(false));
    expect(await screen.findByRole('status')).toHaveTextContent('Your pet is on it');
  });

  it('explains when a pass is already running', async () => {
    mockRunNow.mockRejectedValue(new Error('a Pet pass is already running'));
    renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
    fireEvent.click(await screen.findByTestId('pet-run-now'));
    expect(await screen.findByText(/already running\. Give it a moment/)).toBeInTheDocument();
  });

  it('shows a retryable error when the pet cannot be loaded', async () => {
    mockGetPet.mockRejectedValueOnce(new Error('down'));
    renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
    expect(await screen.findByTestId('pet-load-error')).toHaveTextContent(
      'Could not reach your pet'
    );
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    expect(await screen.findByTestId('pet-name')).toBeInTheDocument();
  });

  it('shows the disabled state and hint when the pet is off', async () => {
    mockGetPet.mockResolvedValue(makePet({ enabled: false, next_research_at: null }));
    renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
    expect(await screen.findByTestId('pet-enabled-badge')).toHaveTextContent('Off');
    expect(screen.getByText(/Pet mode is off/)).toBeInTheDocument();
  });

  describe('desktop companion', () => {
    beforeEach(() => {
      mockCompanionSettings.mockResolvedValue(makeCompanionSettings({ enabled: true }));
      mockCompanionStatus.mockResolvedValue(makeCompanionStatus());
    });

    it('opens on the Now tab by default when the companion is on', async () => {
      renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
      expect(await screen.findByTestId('companion-now-tab')).toBeInTheDocument();
      expect(screen.queryByTestId('pet-feed-tab')).toBeNull();
    });

    it('keeps the Feed as the default when the companion is off', async () => {
      mockCompanionSettings.mockResolvedValue(makeCompanionSettings({ enabled: false }));
      renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
      expect(await screen.findByTestId('pet-feed-tab')).toBeInTheDocument();
      expect(screen.queryByTestId('companion-header-pause')).toBeNull();
    });

    it('opens the Now tab from the tray deep link', async () => {
      mockCompanionSettings.mockResolvedValue(makeCompanionSettings({ enabled: false }));
      renderWithProviders(<PetPage />, { initialEntries: ['/pet?tab=now'] });
      expect(await screen.findByTestId('companion-off')).toBeInTheDocument();
    });

    it('shows the observing indicator and a Pause button in the header', async () => {
      renderWithProviders(<PetPage />, { initialEntries: ['/pet?tab=feed'] });
      expect(await screen.findByTestId('companion-header-badge')).toHaveTextContent('Observing');
      mockCompanionPause.mockResolvedValue(makeCompanionStatus({ state: 'paused' }));
      fireEvent.click(screen.getByTestId('companion-header-pause'));
      await waitFor(() => expect(mockCompanionPause).toHaveBeenCalledWith(undefined, 'ui'));
      await waitFor(() =>
        expect(screen.getByTestId('companion-header-badge')).toHaveTextContent('Paused')
      );
      mockCompanionResume.mockResolvedValue(makeCompanionStatus());
      fireEvent.click(screen.getByTestId('companion-header-resume'));
      await waitFor(() => expect(mockCompanionResume).toHaveBeenCalledWith('ui'));
    });

    it('labels observing the screen distinctly in the header', async () => {
      mockCompanionStatus.mockResolvedValue(makeCompanionStatus({ screen_capture_active: true }));
      renderWithProviders(<PetPage />, { initialEntries: ['/pet?tab=feed'] });
      expect(await screen.findByTestId('companion-header-badge')).toHaveTextContent(
        'Observing screen'
      );
    });

    it('counts new suggestions on the Now tab', async () => {
      mockCompanionSuggestions.mockResolvedValue([makeSuggestion({ state: 'new' })]);
      renderWithProviders(<PetPage />, { initialEntries: ['/pet?tab=feed'] });
      await waitFor(() => expect(screen.getByTestId('pet-tab-now')).toHaveTextContent('1'));
    });

    it('still loads when the companion is unavailable (older core)', async () => {
      mockCompanionSettings.mockRejectedValue(new Error('unknown method'));
      renderWithProviders(<PetPage />, { initialEntries: ['/pet'] });
      expect(await screen.findByTestId('pet-feed-tab')).toBeInTheDocument();
      expect(screen.queryByTestId('companion-header-badge')).toBeNull();
    });

    it('mounts the companion settings panel in the Settings tab', async () => {
      renderWithProviders(<PetPage />, { initialEntries: ['/pet?tab=settings'] });
      expect(await screen.findByTestId('companion-settings')).toBeInTheDocument();
    });
  });
});
