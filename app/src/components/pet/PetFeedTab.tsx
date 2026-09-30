import debug from 'debug';
import { type ReactNode, useEffect, useRef, useState } from 'react';
import Markdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

import { useT } from '../../lib/i18n/I18nContext';
import {
  buildPetDigestNow,
  type PetDigest,
  type PetFeed,
  type PetNote,
  type PetProfile,
} from '../../services/api/petApi';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import { ErrorBanner } from '../ui/LoadingState';
import { formatDateTime, noteStateVariant } from './petFormat';

const log = debug('pet:feed');

const GFM = [remarkGfm];

/**
 * Digest text is built from the user's own data, so links and images are
 * rendered inert: a note can never turn a digest into a click-through.
 */
const MARKDOWN_COMPONENTS = {
  a: ({ children }: { children?: ReactNode }) => <span>{children}</span>,
  img: () => null,
};

interface PetFeedTabProps {
  pet: PetProfile;
  feed: PetFeed;
  highlightDigestId?: string | null;
  highlightNoteId?: string | null;
  onOpenNote: (noteId: string) => void;
  onViewAllNotes: () => void;
  onChanged: () => void;
}

function DigestCard({ digest, highlighted }: { digest: PetDigest; highlighted: boolean }) {
  const { t, locale } = useT();
  const ref = useRef<HTMLElement>(null);
  useEffect(() => {
    if (highlighted) ref.current?.scrollIntoView?.({ block: 'nearest' });
  }, [highlighted]);
  return (
    <article
      ref={ref}
      data-testid="pet-digest"
      data-highlighted={highlighted || undefined}
      className={`rounded-2xl border bg-surface p-4 shadow-subtle ${
        highlighted ? 'border-primary-400 ring-2 ring-primary-500/20' : 'border-line'
      }`}>
      <div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-content-muted">
        <span>{formatDateTime(digest.created_at, locale) ?? digest.local_date}</span>
        <Badge variant="primary">
          {t('pet.feed.itemsCount').replace('{count}', String(digest.item_count))}
        </Badge>
        {digest.withheld_count > 0 && (
          <Badge>{t('pet.feed.withheld').replace('{count}', String(digest.withheld_count))}</Badge>
        )}
      </div>
      <div className="prose prose-sm max-w-none text-sm dark:prose-invert prose-p:my-1 prose-headings:text-sm prose-ul:my-1">
        <Markdown remarkPlugins={GFM} components={MARKDOWN_COMPONENTS}>
          {digest.body_md}
        </Markdown>
      </div>
    </article>
  );
}

function RecentNoteRow({
  note,
  highlighted,
  onOpen,
}: {
  note: PetNote;
  highlighted: boolean;
  onOpen: () => void;
}) {
  const { t, locale } = useT();
  const due = formatDateTime(note.due_at, locale);
  return (
    <li>
      <button
        type="button"
        data-testid="pet-recent-note"
        data-highlighted={highlighted || undefined}
        onClick={onOpen}
        className={`flex w-full items-start justify-between gap-3 rounded-xl border px-3 py-2 text-left transition-colors hover:bg-surface-hover ${
          highlighted ? 'border-primary-400 ring-2 ring-primary-500/20' : 'border-line'
        }`}>
        <span className="min-w-0">
          <span className="block truncate text-sm font-medium text-content">{note.title}</span>
          <span className="mt-0.5 flex flex-wrap items-center gap-1.5 text-xs text-content-muted">
            <span>{t(`pet.notes.source.${note.source}`)}</span>
            {due && (
              <span>
                {t('pet.notes.due')}: {due}
              </span>
            )}
          </span>
        </span>
        <span className="flex shrink-0 items-center gap-1.5">
          <Badge>{t(`pet.notes.kind.${note.kind}`)}</Badge>
          <Badge variant={noteStateVariant(note.state)}>{t(`pet.notes.state.${note.state}`)}</Badge>
          {note.score != null && (
            <Badge variant="neutral">
              {t('pet.notes.score')} {Math.round(note.score)}
            </Badge>
          )}
        </span>
      </button>
    </li>
  );
}

/** The latest digests, a short list of recent notes, and the last pass summary. */
export default function PetFeedTab({
  pet,
  feed,
  highlightDigestId,
  highlightNoteId,
  onOpenNote,
  onViewAllNotes,
  onChanged,
}: PetFeedTabProps) {
  const { t } = useT();
  const [building, setBuilding] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleDigestNow = async () => {
    setBuilding(true);
    setMessage(null);
    setError(null);
    log('digest now: start');
    try {
      const digest = await buildPetDigestNow();
      log('digest now: built=%s', digest !== null);
      if (digest === null) setMessage(t('pet.feed.digestNothing'));
      onChanged();
    } catch (err) {
      log('digest now failed: %o', err);
      setError(t('pet.errors.runFailed'));
    } finally {
      setBuilding(false);
    }
  };

  const run = feed.last_run;
  const isEmpty = feed.digests.length === 0 && feed.notes.length === 0;

  return (
    <div className="space-y-4" data-testid="pet-feed-tab">
      {error && <ErrorBanner message={error} />}
      {run && run.status !== 'started' && (
        <p className="text-xs text-content-muted" data-testid="pet-last-run">
          {t('pet.feed.lastRun')
            .replace('{seen}', String(run.notes_seen))
            .replace('{notified}', String(run.notified))
            .replace('{queued}', String(run.queued))
            .replace('{dropped}', String(run.dropped))}
        </p>
      )}

      {isEmpty ? (
        <div
          data-testid="pet-feed-empty"
          className="rounded-2xl border border-dashed border-line-strong px-6 py-10 text-center">
          <h3 className="text-sm font-semibold text-content">{t('pet.feed.emptyTitle')}</h3>
          <p className="mx-auto mt-1 max-w-md text-sm text-content-muted">
            {pet.enabled ? t('pet.feed.emptyEnabledBody') : t('pet.feed.emptyBody')}
          </p>
        </div>
      ) : (
        <>
          <section className="space-y-3">
            <div className="flex items-center justify-between gap-3">
              <h3 className="text-sm font-semibold text-content">{t('pet.feed.digestHeading')}</h3>
              <Button
                type="button"
                variant="secondary"
                size="xs"
                analyticsId="pet-digest-now"
                data-testid="pet-digest-now"
                disabled={building}
                onClick={() => void handleDigestNow()}>
                {t('pet.feed.digestNow')}
              </Button>
            </div>
            {message && (
              <p role="status" className="text-xs text-content-muted">
                {message}
              </p>
            )}
            {feed.digests.map(digest => (
              <DigestCard
                key={digest.id}
                digest={digest}
                highlighted={digest.id === highlightDigestId}
              />
            ))}
          </section>

          {feed.notes.length > 0 && (
            <section className="space-y-2">
              <div className="flex items-center justify-between gap-3">
                <h3 className="text-sm font-semibold text-content">{t('pet.feed.recentNotes')}</h3>
                <Button
                  type="button"
                  variant="tertiary"
                  size="xs"
                  analyticsId="pet-view-all-notes"
                  onClick={onViewAllNotes}>
                  {t('pet.feed.viewAllNotes')}
                </Button>
              </div>
              <ul className="space-y-2">
                {feed.notes.map(note => (
                  <RecentNoteRow
                    key={note.id}
                    note={note}
                    highlighted={note.id === highlightNoteId}
                    onOpen={() => onOpenNote(note.id)}
                  />
                ))}
              </ul>
            </section>
          )}
        </>
      )}
    </div>
  );
}
