import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import Button from '../../ui/Button';
import Input from '../../ui/Input';

interface CompanionExclusionEditorProps {
  /** Used for ids and test ids: `apps` or `titles`. */
  name: string;
  label: string;
  hint: string;
  placeholder: string;
  items: string[];
  /** Suggestions the user can add with one click (not already in `items`). */
  quickAdd?: string[];
  quickAddLabel?: string;
  disabled?: boolean;
  /** Returns an i18n key when the new entry is invalid, or `null`. */
  validate?: (entry: string, existing: string[]) => string | null;
  /** Replaces the whole list. Rejects on failure. */
  onChange: (next: string[]) => Promise<void>;
}

/** A list of exclusion entries with add and remove. Saves the whole list on every change. */
export default function CompanionExclusionEditor({
  name,
  label,
  hint,
  placeholder,
  items,
  quickAdd = [],
  quickAddLabel,
  disabled = false,
  validate,
  onChange,
}: CompanionExclusionEditorProps) {
  const { t } = useT();
  const [draft, setDraft] = useState('');
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const commit = async (next: string[]) => {
    setBusy(true);
    setProblem(null);
    try {
      await onChange(next);
    } catch {
      setProblem(t('pet.companion.errors.saveFailed'));
    } finally {
      setBusy(false);
    }
  };

  const add = async (raw: string) => {
    const entry = raw.trim();
    if (!entry) return;
    if (items.some(i => i.toLowerCase() === entry.toLowerCase())) {
      setDraft('');
      return;
    }
    const key = validate?.(entry, items);
    if (key) {
      setProblem(t(key));
      return;
    }
    await commit([...items, entry]);
    setDraft('');
  };

  const suggestions = quickAdd.filter(q => !items.some(i => i.toLowerCase() === q.toLowerCase()));

  return (
    <div className="space-y-2" data-testid={`companion-exclusions-${name}`}>
      <div>
        <p className="text-sm font-medium text-content">{label}</p>
        <p className="mt-0.5 text-xs text-content-muted">{hint}</p>
      </div>
      {items.length === 0 ? (
        <p className="text-xs italic text-content-faint">{t('pet.companion.exclusions.none')}</p>
      ) : (
        <ul className="space-y-1">
          {items.map(item => (
            <li
              key={item}
              data-testid={`companion-exclusion-${name}`}
              className="flex items-center justify-between gap-3 rounded-lg bg-surface-subtle px-3 py-1 text-sm text-content">
              <span className="min-w-0 truncate font-mono text-xs">{item}</span>
              <Button
                type="button"
                variant="tertiary"
                tone="danger"
                size="xs"
                analyticsId={`pet-companion-exclusion-${name}-remove`}
                aria-label={`${t('pet.companion.exclusions.remove')}: ${item}`}
                disabled={disabled || busy}
                onClick={() => void commit(items.filter(i => i !== item))}>
                {t('pet.companion.exclusions.remove')}
              </Button>
            </li>
          ))}
        </ul>
      )}
      {suggestions.length > 0 && quickAddLabel && (
        <div className="flex flex-wrap items-center gap-1.5 text-xs text-content-muted">
          <span>{quickAddLabel}</span>
          {suggestions.slice(0, 6).map(q => (
            <Button
              key={q}
              type="button"
              variant="secondary"
              size="xs"
              analyticsId={`pet-companion-exclusion-${name}-quick-add`}
              disabled={disabled || busy}
              onClick={() => void add(q)}>
              {q}
            </Button>
          ))}
        </div>
      )}
      <div className="flex gap-2">
        <Input
          data-testid={`companion-exclusion-${name}-input`}
          aria-label={label}
          placeholder={placeholder}
          value={draft}
          maxLength={200}
          disabled={disabled}
          onChange={e => {
            setDraft(e.target.value);
            setProblem(null);
          }}
          onKeyDown={e => {
            if (e.key === 'Enter') {
              e.preventDefault();
              void add(draft);
            }
          }}
        />
        <Button
          type="button"
          variant="secondary"
          size="md"
          analyticsId={`pet-companion-exclusion-${name}-add`}
          data-testid={`companion-exclusion-${name}-add`}
          disabled={disabled || busy || draft.trim().length === 0}
          onClick={() => void add(draft)}>
          {t('pet.companion.exclusions.add')}
        </Button>
      </div>
      {problem && (
        <p role="alert" className="text-xs text-coral-600 dark:text-coral-400">
          {problem}
        </p>
      )}
    </div>
  );
}
