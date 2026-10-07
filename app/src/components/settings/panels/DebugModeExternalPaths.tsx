import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import Button from '../../ui/Button';
import { SettingsEmptyState, SettingsListItem, SettingsTextField } from '../controls';

/** POSIX absolute, Windows drive-letter, or UNC path. */
const ABSOLUTE_PATH = /^(\/|[A-Za-z]:[\\/]|\\\\)/;

interface DebugModeExternalPathsProps {
  paths: string[];
  onChange: (next: string[]) => void;
}

/** Editable list of absolute directories outside the project the agent may touch. */
const DebugModeExternalPaths = ({ paths, onChange }: DebugModeExternalPathsProps) => {
  const { t } = useT();
  const [draft, setDraft] = useState('');
  const [invalid, setInvalid] = useState(false);

  const add = () => {
    const path = draft.trim();
    if (!path) return;
    if (!ABSOLUTE_PATH.test(path)) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    setDraft('');
    if (paths.includes(path)) return;
    onChange([...paths, path]);
  };

  return (
    <div data-testid="debug-external-paths">
      {paths.length === 0 ? (
        <SettingsEmptyState label={t('settings.debugMode.externalPaths.none')} />
      ) : (
        <ul>
          {paths.map(p => (
            <SettingsListItem
              key={p}
              label={p}
              mono
              onRemove={() => onChange(paths.filter(x => x !== p))}
              removeLabel={t('settings.debugMode.externalPaths.remove')}
            />
          ))}
        </ul>
      )}
      <div className="flex items-center gap-2 border-t border-line-subtle px-4 py-3">
        <SettingsTextField
          mono
          className="flex-1"
          inputSize="sm"
          value={draft}
          invalid={invalid}
          onChange={e => {
            setDraft(e.target.value);
            setInvalid(false);
          }}
          onKeyDown={e => {
            if (e.key === 'Enter') {
              e.preventDefault();
              add();
            }
          }}
          placeholder={t('settings.debugMode.externalPaths.placeholder')}
          aria-label={t('settings.debugMode.externalPaths.placeholder')}
        />
        <Button
          type="button"
          variant="primary"
          size="xs"
          analyticsId="settings-debug-mode-external-path-add"
          onClick={add}
          disabled={!draft.trim()}>
          {t('settings.debugMode.externalPaths.add')}
        </Button>
      </div>
      {invalid && (
        <p className="px-4 pb-3 text-xs text-coral-600 dark:text-coral-300" role="alert">
          {t('settings.debugMode.externalPaths.invalid')}
        </p>
      )}
    </div>
  );
};

export default DebugModeExternalPaths;
