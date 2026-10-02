import { LuPawPrint } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import type { PetProfile } from '../../services/api/petApi';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import { type CompanionDisplayState, formatRelative } from './petFormat';

interface PetHeaderProps {
  pet: PetProfile;
  running: boolean;
  notice: string | null;
  error: string | null;
  onRunNow: () => void;
  /** Desktop companion indicator. Omit to hide it. */
  companion?: {
    displayState: CompanionDisplayState;
    busy?: boolean;
    onPause: () => void;
    onResume: () => void;
  };
}

/** The pet's name, whether it is on, when it last and next works, and Run now. */
export default function PetHeader({
  pet,
  running,
  notice,
  error,
  onRunNow,
  companion,
}: PetHeaderProps) {
  const { t, locale } = useT();
  const never = t('pet.status.never');
  const notScheduled = t('pet.status.notScheduled');
  const lastPass = formatRelative(pet.last_pass_at, locale) ?? never;
  const nextPass = pet.enabled
    ? (formatRelative(pet.next_research_at, locale) ?? notScheduled)
    : notScheduled;
  const nextDigest = pet.enabled
    ? (formatRelative(pet.next_digest_at, locale) ?? notScheduled)
    : notScheduled;

  return (
    <section
      data-testid="pet-header"
      className="rounded-2xl border border-line bg-surface px-4 py-3 shadow-subtle">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <span
            aria-hidden="true"
            className="mt-0.5 flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-primary-50 text-primary-600 dark:bg-primary-500/10 dark:text-primary-300">
            <LuPawPrint className="h-5 w-5" />
          </span>
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h2 className="truncate text-base font-semibold text-content" data-testid="pet-name">
                {pet.name}
              </h2>
              <Badge variant={pet.enabled ? 'success' : 'neutral'} data-testid="pet-enabled-badge">
                {pet.enabled ? t('pet.status.enabled') : t('pet.status.disabled')}
              </Badge>
            </div>
            <ul className="mt-1 space-y-0.5 text-xs text-content-muted">
              <li>{t('pet.status.lastPass').replace('{when}', lastPass)}</li>
              <li>{t('pet.status.nextPass').replace('{when}', nextPass)}</li>
              <li>{t('pet.status.nextDigest').replace('{when}', nextDigest)}</li>
            </ul>
            {companion && companion.displayState !== 'off' && (
              <div
                data-testid="companion-indicator"
                data-state={companion.displayState}
                className="mt-2 flex flex-wrap items-center gap-2">
                <Badge
                  variant={
                    companion.displayState === 'paused'
                      ? 'warning'
                      : companion.displayState === 'suspended'
                        ? 'danger'
                        : 'success'
                  }
                  data-testid="companion-header-badge">
                  {t(`pet.companion.state.${companion.displayState}`)}
                </Badge>
                {companion.displayState === 'paused' ? (
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    analyticsId="pet-companion-header-resume"
                    data-testid="companion-header-resume"
                    disabled={companion.busy}
                    onClick={companion.onResume}>
                    {t('pet.companion.resume')}
                  </Button>
                ) : (
                  <Button
                    type="button"
                    variant="secondary"
                    size="xs"
                    analyticsId="pet-companion-header-pause"
                    data-testid="companion-header-pause"
                    disabled={companion.busy}
                    onClick={companion.onPause}>
                    {t('pet.companion.pause')}
                  </Button>
                )}
              </div>
            )}
            {!pet.enabled && (
              <p className="mt-2 text-xs text-content-secondary">{t('pet.header.disabledHint')}</p>
            )}
          </div>
        </div>
        <Button
          type="button"
          variant="primary"
          size="sm"
          analyticsId="pet-run-now"
          data-testid="pet-run-now"
          disabled={running}
          onClick={onRunNow}>
          {running ? t('pet.actions.running') : t('pet.actions.runNow')}
        </Button>
      </div>
      {notice && (
        <p role="status" className="mt-3 text-xs text-sage-700 dark:text-sage-300">
          {notice}
        </p>
      )}
      {error && (
        <p role="alert" className="mt-3 text-xs text-coral-600 dark:text-coral-400">
          {error}
        </p>
      )}
    </section>
  );
}
