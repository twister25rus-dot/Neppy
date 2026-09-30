import debug from 'debug';
import { type ReactNode, useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  dismissPetNote,
  fetchPetNotes,
  type PetNote,
  type PetNoteState,
} from '../../services/api/petApi';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import { CenteredLoadingState, ErrorBanner } from '../ui/LoadingState';
import NativeSelect from '../ui/NativeSelect';
import { formatDateTime, noteStateVariant, urgencyVariant } from './petFormat';

const log = debug('pet:notes');

const STATES: PetNoteState[] = ['new', 'notified', 'queued', 'digested', 'dropped', 'dismissed'];
const NOTES_LIMIT = 100;

interface PetNotesTabProps {
  selectedId: string | null;
  onSelect: (noteId: string | null) => void;
  /** Changes whenever the parent has fresh data, so this list refetches too. */
  refreshKey: number;
  onChanged: () => void;
}

function MetaRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-xs">
      <dt className="text-content-muted">{label}</dt>
      <dd className="text-content-secondary">{children}</dd>
    </div>
  );
}

function NoteDetail({
  note,
  busy,
  onDismiss,
}: {
  note: PetNote;
  busy: boolean;
  onDismiss: () => void;
}) {
  const { t, locale } = useT();
  const due = formatDateTime(note.due_at, locale);
  return (
    <div
      data-testid="pet-note-detail"
      className="space-y-3 rounded-2xl border border-line bg-surface p-4 shadow-subtle">
      <h3 className="text-sm font-semibold text-content">{note.title}</h3>
      {note.injection_flagged && (
        <div
          role="alert"
          data-testid="pet-note-injection-warning"
          className="rounded-xl border border-amber-200 bg-amber-50 p-3 text-xs text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300">
          {t('pet.notes.injectionWarning')}
        </div>
      )}
      <dl className="space-y-1">
        <MetaRow label={t('pet.notes.sourceLabel')}>{t(`pet.notes.source.${note.source}`)}</MetaRow>
        <MetaRow label={t('pet.notes.kindLabel')}>{t(`pet.notes.kind.${note.kind}`)}</MetaRow>
        <MetaRow label={t('pet.notes.stateLabel')}>
          <Badge variant={noteStateVariant(note.state)}>{t(`pet.notes.state.${note.state}`)}</Badge>
        </MetaRow>
        <MetaRow label={t('pet.notes.urgency')}>
          <Badge variant={urgencyVariant(note.urgency)}>
            {t('pet.notes.urgencyValue').replace('{level}', String(note.urgency))}
          </Badge>
        </MetaRow>
        {note.score != null && (
          <MetaRow label={t('pet.notes.score')}>{Math.round(note.score)}</MetaRow>
        )}
        {note.bucket && (
          <MetaRow label={t('pet.notes.bucketLabel')}>
            {t(`pet.notes.bucket.${note.bucket}`)}
          </MetaRow>
        )}
        {due && <MetaRow label={t('pet.notes.due')}>{due}</MetaRow>}
      </dl>
      {/* Plain text on purpose: the body is untrusted text from the user's data. */}
      {note.body && (
        <p
          data-testid="pet-note-body"
          className="whitespace-pre-wrap wrap-break-word text-sm text-content-secondary">
          {note.body}
        </p>
      )}
      {note.proposed_action && (
        <div>
          <p className="text-xs font-medium text-content-muted">{t('pet.notes.proposedAction')}</p>
          <p className="mt-0.5 whitespace-pre-wrap text-sm text-content">{note.proposed_action}</p>
        </div>
      )}
      {note.state !== 'dismissed' && (
        <Button
          type="button"
          variant="secondary"
          tone="danger"
          size="sm"
          analyticsId="pet-note-dismiss"
          data-testid="pet-note-dismiss"
          disabled={busy}
          onClick={onDismiss}>
          {t('pet.notes.dismiss')}
        </Button>
      )}
    </div>
  );
}

/** Every note the pet has recorded, with a state filter and a detail pane. */
export default function PetNotesTab({
  selectedId,
  onSelect,
  refreshKey,
  onChanged,
}: PetNotesTabProps) {
  const { t, locale } = useT();
  const [filter, setFilter] = useState<PetNoteState | 'all'>('all');
  const [notes, setNotes] = useState<PetNote[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Bumped by the Retry button to refetch without changing the filter.
  const [retryKey, setRetryKey] = useState(0);

  useEffect(() => {
    let cancelled = false;
    log('load filter=%s', filter);
    fetchPetNotes({ limit: NOTES_LIMIT, ...(filter === 'all' ? {} : { state: filter }) })
      .then(next => {
        if (cancelled) return;
        setNotes(next);
        setError(null);
        log('loaded count=%d', next.length);
      })
      .catch(err => {
        if (cancelled) return;
        log('load failed: %o', err);
        setError(t('pet.errors.loadFailed'));
      });
    return () => {
      cancelled = true;
    };
  }, [filter, refreshKey, retryKey, t]);

  const handleDismiss = async (note: PetNote) => {
    setBusy(true);
    setError(null);
    log('dismiss id=%s', note.id);
    try {
      const updated = await dismissPetNote(note.id);
      setNotes(prev => prev?.map(n => (n.id === updated.id ? updated : n)) ?? prev);
      onChanged();
    } catch (err) {
      log('dismiss failed id=%s err=%o', note.id, err);
      setError(t('pet.errors.dismissFailed'));
    } finally {
      setBusy(false);
    }
  };

  const selected = notes?.find(n => n.id === selectedId) ?? null;

  return (
    <div className="space-y-3" data-testid="pet-notes-tab">
      <div className="flex items-center gap-2">
        <label htmlFor="pet-notes-filter" className="text-xs text-content-muted">
          {t('pet.notes.filterLabel')}
        </label>
        <NativeSelect
          id="pet-notes-filter"
          inputSize="sm"
          data-testid="pet-notes-filter"
          value={filter}
          onChange={e => setFilter(e.target.value as PetNoteState | 'all')}>
          <option value="all">{t('pet.notes.filterAll')}</option>
          {STATES.map(s => (
            <option key={s} value={s}>
              {t(`pet.notes.state.${s}`)}
            </option>
          ))}
        </NativeSelect>
      </div>

      {error && (
        <ErrorBanner
          message={error}
          action={
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={() => setRetryKey(n => n + 1)}>
              {t('pet.actions.retry')}
            </Button>
          }
        />
      )}

      {notes === null && !error && <CenteredLoadingState label={t('pet.loading')} />}

      {notes !== null && notes.length === 0 && (
        <div
          data-testid="pet-notes-empty"
          className="rounded-2xl border border-dashed border-line-strong px-6 py-10 text-center">
          <h3 className="text-sm font-semibold text-content">{t('pet.notes.emptyTitle')}</h3>
          <p className="mx-auto mt-1 max-w-md text-sm text-content-muted">
            {t('pet.notes.emptyBody')}
          </p>
        </div>
      )}

      {notes !== null && notes.length > 0 && (
        <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_minmax(0,1.2fr)]">
          <ul className="space-y-2" aria-label={t('pet.tabs.notes')}>
            {notes.map(note => (
              <li key={note.id}>
                <button
                  type="button"
                  data-testid="pet-note-row"
                  aria-pressed={note.id === selectedId}
                  onClick={() => onSelect(note.id)}
                  className={`w-full rounded-xl border px-3 py-2 text-left transition-colors hover:bg-surface-hover ${
                    note.id === selectedId
                      ? 'border-primary-400 bg-primary-50/40 dark:bg-primary-500/10'
                      : 'border-line'
                  }`}>
                  <span className="block truncate text-sm font-medium text-content">
                    {note.title}
                  </span>
                  <span className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-content-muted">
                    <Badge>{t(`pet.notes.kind.${note.kind}`)}</Badge>
                    <Badge variant={noteStateVariant(note.state)}>
                      {t(`pet.notes.state.${note.state}`)}
                    </Badge>
                    {note.due_at && (
                      <span>
                        {t('pet.notes.due')}: {formatDateTime(note.due_at, locale)}
                      </span>
                    )}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {selected ? (
            <NoteDetail
              note={selected}
              busy={busy}
              onDismiss={() => void handleDismiss(selected)}
            />
          ) : (
            <p
              data-testid="pet-note-select-prompt"
              className="rounded-2xl border border-dashed border-line-strong px-4 py-8 text-center text-sm text-content-muted">
              {t('pet.notes.selectPrompt')}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
