import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type CompanionData,
  deleteCompanionData,
  getCompanionData,
} from '../../../services/api/petCompanionApi';
import Button from '../../ui/Button';
import Checkbox from '../../ui/Checkbox';
import { ConfirmDialog } from '../../ui/ConfirmDialog';
import { CenteredLoadingState } from '../../ui/LoadingState';
import { ModalShell } from '../../ui/ModalShell';
import { formatDateTime } from '../petFormat';

const log = debug('pet:companion:data');

interface CompanionDataDialogProps {
  onClose: () => void;
  /** Called after anything is deleted so the page can refresh its own lists. */
  onChanged?: () => void;
}

/** Everything the companion keeps: suggestions and the action log. View, delete one, delete all. */
export default function CompanionDataDialog({ onClose, onChanged }: CompanionDataDialogProps) {
  const { t, locale } = useT();
  const [data, setData] = useState<CompanionData | null>(null);
  const [loadError, setLoadError] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirming, setConfirming] = useState(false);
  const [includeNotes, setIncludeNotes] = useState(false);

  const load = useCallback(async () => {
    try {
      setData(await getCompanionData());
      setLoadError(false);
    } catch (err) {
      log('load failed: %o', err);
      setLoadError(true);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    getCompanionData()
      .then(next => {
        if (cancelled) return;
        setData(next);
        setLoadError(false);
      })
      .catch(err => {
        log('load failed: %o', err);
        if (!cancelled) setLoadError(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const deleteOne = async (id: string) => {
    setBusy(true);
    setError(null);
    log('delete one id=%s', id);
    try {
      await deleteCompanionData({ suggestionId: id });
      await load();
      onChanged?.();
    } catch (err) {
      log('delete one failed: %o', err);
      setError(t('pet.companion.errors.deleteFailed'));
    } finally {
      setBusy(false);
    }
  };

  const deleteAll = async () => {
    setBusy(true);
    setError(null);
    log('delete all includeNotes=%s', includeNotes);
    try {
      await deleteCompanionData({ all: true, includeSavedNotes: includeNotes });
      setConfirming(false);
      await load();
      onChanged?.();
    } catch (err) {
      log('delete all failed: %o', err);
      setConfirming(false);
      setError(t('pet.companion.errors.deleteFailed'));
    } finally {
      setBusy(false);
    }
  };

  const empty = data !== null && data.suggestions.length === 0 && data.actions.length === 0;

  return (
    <>
      <ModalShell
        title={t('pet.companion.data.title')}
        titleId="pet-companion-data-title"
        maxWidthClassName="max-w-lg"
        onClose={onClose}
        footer={
          <div className="flex justify-between gap-2">
            <Button
              type="button"
              variant="secondary"
              tone="danger"
              size="sm"
              analyticsId="pet-companion-delete-all"
              data-testid="companion-delete-all"
              disabled={busy || data === null}
              onClick={() => setConfirming(true)}>
              {t('pet.companion.data.deleteAll')}
            </Button>
            <Button type="button" variant="secondary" size="sm" onClick={onClose}>
              {t('common.close')}
            </Button>
          </div>
        }>
        <div className="max-h-[60vh] space-y-4 overflow-y-auto" data-testid="companion-data">
          <p className="text-xs text-content-muted">{t('pet.companion.data.hint')}</p>
          {error && (
            <p role="alert" className="text-xs text-coral-600 dark:text-coral-400">
              {error}
            </p>
          )}
          {loadError && <p className="text-xs text-coral-600">{t('pet.companion.loadFailed')}</p>}
          {data === null && !loadError && <CenteredLoadingState label={t('common.working')} />}
          {empty && <p className="text-sm text-content-muted">{t('pet.companion.data.empty')}</p>}
          {data && data.suggestions.length > 0 && (
            <section className="space-y-2">
              <h3 className="text-sm font-semibold text-content">
                {t('pet.companion.data.suggestions').replace(
                  '{count}',
                  String(data.counts.suggestions)
                )}
              </h3>
              <ul className="space-y-1.5">
                {data.suggestions.map(s => (
                  <li
                    key={s.id}
                    data-testid="companion-data-suggestion"
                    className="flex items-center justify-between gap-3 rounded-lg bg-surface-subtle px-3 py-1.5 text-sm text-content">
                    <span className="min-w-0">
                      <span className="block truncate">{s.headline}</span>
                      <span className="block text-xs text-content-muted">
                        {s.app_name} · {formatDateTime(s.created_at, locale)}
                      </span>
                    </span>
                    <Button
                      type="button"
                      variant="tertiary"
                      tone="danger"
                      size="xs"
                      analyticsId="pet-companion-delete-one"
                      data-testid="companion-delete-one"
                      disabled={busy}
                      onClick={() => void deleteOne(s.id)}>
                      {t('common.delete')}
                    </Button>
                  </li>
                ))}
              </ul>
            </section>
          )}
          {data && data.actions.length > 0 && (
            <section className="space-y-2">
              <h3 className="text-sm font-semibold text-content">
                {t('pet.companion.data.actions').replace('{count}', String(data.counts.actions))}
              </h3>
              <ul className="space-y-1">
                {data.actions.map(a => (
                  <li
                    key={a.id}
                    data-testid="companion-data-action"
                    className="text-xs text-content-muted">
                    {formatDateTime(a.at, locale)} · {t(`pet.companion.category.${a.category}`)} ·{' '}
                    {t(`pet.companion.data.decision.${a.decision}`)} ·{' '}
                    {t(`pet.companion.data.outcome.${a.outcome}`)}
                  </li>
                ))}
              </ul>
            </section>
          )}
        </div>
      </ModalShell>
      {confirming && (
        <ConfirmDialog
          title={t('pet.companion.data.confirmTitle')}
          titleId="pet-companion-delete-title"
          body={
            <div className="space-y-3">
              <p>{t('pet.companion.data.confirmBody')}</p>
              <label className="flex items-center gap-2 text-xs text-content">
                <Checkbox
                  data-testid="companion-delete-notes"
                  checked={includeNotes}
                  onCheckedChange={setIncludeNotes}
                />
                {t('pet.companion.data.includeNotes')}
              </label>
            </div>
          }
          confirmLabel={t('pet.companion.data.deleteAll')}
          destructive
          busy={busy}
          onConfirm={() => void deleteAll()}
          onCancel={() => setConfirming(false)}
        />
      )}
    </>
  );
}
