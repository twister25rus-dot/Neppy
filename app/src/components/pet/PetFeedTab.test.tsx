import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import PetFeedTab from './PetFeedTab';
import { makeDigest, makeFeed, makeNote, makePet } from './petFixtures';

const mockDigestNow = vi.fn();

vi.mock('../../services/api/petApi', () => ({
  buildPetDigestNow: (...args: unknown[]) => mockDigestNow(...args),
}));

const baseProps = {
  pet: makePet(),
  onOpenNote: vi.fn(),
  onViewAllNotes: vi.fn(),
  onChanged: vi.fn(),
};

describe('PetFeedTab', () => {
  beforeEach(() => vi.clearAllMocks());

  it('explains how to enable the pet when it is off and nothing exists yet', () => {
    renderWithProviders(
      <PetFeedTab {...baseProps} pet={makePet({ enabled: false })} feed={makeFeed()} />
    );
    expect(screen.getByTestId('pet-feed-empty')).toHaveTextContent('Turn on Pet mode in Settings');
  });

  it('shows a different empty state when the pet is already on', () => {
    renderWithProviders(<PetFeedTab {...baseProps} feed={makeFeed()} />);
    expect(screen.getByTestId('pet-feed-empty')).toHaveTextContent('Your pet is on');
  });

  it('renders the digest markdown and the recent notes', () => {
    renderWithProviders(
      <PetFeedTab
        {...baseProps}
        feed={makeFeed({
          digests: [makeDigest({ item_count: 2, withheld_count: 1 })],
          notes: [makeNote()],
          last_run: {
            run_id: 'r1',
            status: 'completed',
            trigger: 'manual',
            finished_at: null,
            notes_seen: 3,
            notified: 1,
            queued: 1,
            dropped: 1,
            digest_id: null,
          },
        })}
      />
    );
    expect(screen.getByText('Pip: your digest').tagName).toBe('STRONG');
    expect(screen.getByText('2 items')).toBeInTheDocument();
    expect(screen.getByText('1 held back')).toBeInTheDocument();
    expect(screen.getByTestId('pet-last-run')).toHaveTextContent('3 seen');
    fireEvent.click(screen.getByTestId('pet-recent-note'));
    expect(baseProps.onOpenNote).toHaveBeenCalledWith('note-1');
  });

  it('renders links in a digest as inert text', () => {
    renderWithProviders(
      <PetFeedTab
        {...baseProps}
        feed={makeFeed({ digests: [makeDigest({ body_md: '[click me](https://evil.example)' })] })}
      />
    );
    expect(screen.getByText('click me').closest('a')).toBeNull();
  });

  it('highlights the digest named by the deep link', () => {
    renderWithProviders(
      <PetFeedTab
        {...baseProps}
        highlightDigestId="digest-1"
        feed={makeFeed({ digests: [makeDigest()] })}
      />
    );
    expect(screen.getByTestId('pet-digest')).toHaveAttribute('data-highlighted', 'true');
  });

  it('builds a digest on demand and reports when nothing was waiting', async () => {
    mockDigestNow.mockResolvedValue(null);
    renderWithProviders(<PetFeedTab {...baseProps} feed={makeFeed({ digests: [makeDigest()] })} />);
    fireEvent.click(screen.getByTestId('pet-digest-now'));
    expect(await screen.findByRole('status')).toHaveTextContent('Nothing is waiting');
    expect(baseProps.onChanged).toHaveBeenCalled();
  });

  it('shows an error when building a digest fails', async () => {
    mockDigestNow.mockRejectedValue(new Error('nope'));
    renderWithProviders(<PetFeedTab {...baseProps} feed={makeFeed({ digests: [makeDigest()] })} />);
    fireEvent.click(screen.getByTestId('pet-digest-now'));
    await waitFor(() => expect(screen.getByRole('alert')).toBeInTheDocument());
  });
});
