import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import Button from '../../ui/Button';
import { SettingsNumberField, SettingsRow, SettingsTextField } from '../controls';
import { MAX_REPAIR_MAX, MAX_REPAIR_MIN } from './DebugModeSettingsState';

// Both fields keep a local draft and are remounted (via `key`) by the panel
// whenever the saved value changes, which also resets the draft after a revert.

interface RepairsFieldProps {
  value: number;
  onCommit: (next: number) => void;
}

/** Max repair attempts: committed on blur/Enter, clamped to 1..20, no-op if unchanged. */
export const DebugModeRepairsField = ({ value, onCommit }: RepairsFieldProps) => {
  const { t } = useT();
  const [draft, setDraft] = useState(String(value));

  const commit = () => {
    const parsed = Math.round(Number(draft.trim()));
    const next = Number.isFinite(parsed)
      ? Math.min(MAX_REPAIR_MAX, Math.max(MAX_REPAIR_MIN, parsed))
      : value;
    setDraft(String(next));
    if (next !== value) onCommit(next);
  };

  return (
    <SettingsRow
      htmlFor="debug-max-repairs"
      label={t('settings.debugMode.maxRepairs.label')}
      description={t('settings.debugMode.maxRepairs.desc')}
      control={
        <SettingsNumberField
          id="debug-max-repairs"
          value={draft}
          onChange={setDraft}
          onCommit={commit}
          min={MAX_REPAIR_MIN}
          max={MAX_REPAIR_MAX}
          unit={t('settings.debugMode.maxRepairs.unit')}
          aria-label={t('settings.debugMode.maxRepairs.label')}
        />
      }
    />
  );
};

interface RootFieldProps {
  /** `null` = auto-detect. */
  value: string | null;
  onCommit: (next: string | null) => void;
}

/** Project root: committed on blur/Enter; empty or "Use default" sends `null`. */
export const DebugModeRootField = ({ value, onCommit }: RootFieldProps) => {
  const { t } = useT();
  const [draft, setDraft] = useState(value ?? '');

  const commit = () => {
    const next = draft.trim();
    setDraft(next);
    if (next !== (value ?? '')) onCommit(next === '' ? null : next);
  };

  return (
    <SettingsRow
      stacked
      htmlFor="debug-project-root"
      label={t('settings.debugMode.projectRoot.label')}
      description={t('settings.debugMode.projectRoot.desc')}
      control={
        <div className="flex items-center gap-2">
          <SettingsTextField
            id="debug-project-root"
            mono
            className="flex-1"
            inputSize="sm"
            value={draft}
            onChange={e => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={e => {
              if (e.key === 'Enter') {
                e.preventDefault();
                commit();
              }
            }}
            placeholder={t('settings.debugMode.projectRoot.placeholder')}
            aria-label={t('settings.debugMode.projectRoot.label')}
          />
          <Button
            type="button"
            variant="secondary"
            size="xs"
            analyticsId="settings-debug-mode-root-default"
            disabled={value === null}
            onClick={() => onCommit(null)}>
            {t('settings.debugMode.projectRoot.useDefault')}
          </Button>
        </div>
      }
    />
  );
};
