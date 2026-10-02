import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import type { CompanionSourceKey, CompanionSources } from '../../../services/api/petCompanionApi';
import Button from '../../ui/Button';
import Checkbox from '../../ui/Checkbox';
import { ModalShell } from '../../ui/ModalShell';

export const COMPANION_SOURCE_KEYS: readonly CompanionSourceKey[] = [
  'app_window',
  'selection',
  'clipboard',
  'screen_capture',
];

interface CompanionConsentDialogProps {
  busy?: boolean;
  error?: string | null;
  /** Called with the sources the user left ticked. Nothing is enabled before this. */
  onAccept: (sources: CompanionSources) => void;
  onCancel: () => void;
}

/**
 * First-run consent. Every source is pre-ticked (turning the companion on is the
 * consent), each one is explained in plain words, and each can be unticked.
 * The companion is only enabled after the user accepts here.
 */
export default function CompanionConsentDialog({
  busy = false,
  error,
  onAccept,
  onCancel,
}: CompanionConsentDialogProps) {
  const { t } = useT();
  const [ticked, setTicked] = useState<CompanionSources>({
    app_window: true,
    selection: true,
    clipboard: true,
    screen_capture: true,
  });

  return (
    <ModalShell
      title={t('pet.companion.consent.title')}
      titleId="pet-companion-consent-title"
      maxWidthClassName="max-w-lg"
      closePolicy={busy ? { escape: false, backdrop: false, button: false } : undefined}
      onClose={onCancel}
      footer={
        <div className="flex justify-end gap-2">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            analyticsId="pet-companion-consent-cancel"
            data-testid="companion-consent-cancel"
            disabled={busy}
            onClick={onCancel}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            analyticsId="pet-companion-consent-accept"
            data-testid="companion-consent-accept"
            disabled={busy}
            onClick={() => onAccept(ticked)}>
            {busy ? t('common.working') : t('pet.companion.consent.accept')}
          </Button>
        </div>
      }>
      <div className="space-y-4" data-testid="companion-consent">
        <p className="text-sm text-content-secondary">{t('pet.companion.consent.intro')}</p>
        <ul className="space-y-3">
          {COMPANION_SOURCE_KEYS.map(key => (
            <li key={key} className="flex items-start gap-3">
              <Checkbox
                id={`companion-consent-${key}`}
                data-testid={`companion-consent-${key}`}
                checked={ticked[key]}
                disabled={busy}
                onCheckedChange={on => setTicked(prev => ({ ...prev, [key]: on }))}
                className="mt-0.5"
              />
              <label htmlFor={`companion-consent-${key}`} className="min-w-0 cursor-pointer">
                <span className="block text-sm font-medium text-content">
                  {t(`pet.companion.source.${key}`)}
                </span>
                <span className="block text-xs leading-relaxed text-content-muted">
                  {t(`pet.companion.source.${key}.desc`)}
                </span>
              </label>
            </li>
          ))}
        </ul>
        <div className="space-y-2 rounded-xl bg-surface-subtle p-3 text-xs leading-relaxed text-content-secondary">
          <p data-testid="companion-consent-private">{t('pet.companion.consent.private')}</p>
          <p>{t('pet.companion.consent.model')}</p>
          <p>{t('pet.companion.consent.permissions')}</p>
          <p>{t('pet.companion.consent.control')}</p>
        </div>
        {error && (
          <p role="alert" className="text-xs text-coral-600 dark:text-coral-400">
            {error}
          </p>
        )}
      </div>
    </ModalShell>
  );
}
