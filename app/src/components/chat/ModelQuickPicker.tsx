import { LuCheck, LuChevronRight } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import { PopoverContent, PopoverRoot, PopoverTrigger } from '../ui/Popover';

interface ModelQuickPickerProps {
  /** Current `provider:model` value, or null for managed routing. */
  value: string | null | undefined;
  onSelect: (value: string | null) => void;
  /** Opens the full provider/model dialog — the escape hatch from this list. */
  onBrowseAll: () => void;
  children: React.ReactNode;
}

/** `anthropic:claude-opus-5` reads as "claude-opus-5" once the provider is implied. */
const modelOf = (key: string): string => {
  const separator = key.indexOf(':');
  return separator >= 0 ? key.slice(separator + 1) : key;
};

/**
 * The composer's model menu: a popover, not a modal.
 *
 * Switching model mid-thought used to mean a full-screen dialog with a provider
 * column, a search box and a Cancel/Use-this-model footer — a lot of ceremony
 * for picking between two models you use every day. This is the short list you
 * actually pin, one click each, with the dialog still one row away for
 * everything else.
 *
 * The list is whatever Connections → LLM has marked visible. When nothing is
 * pinned it falls back to the current selection alone, so the menu is never
 * empty and never pretends the current model is unavailable.
 */
export default function ModelQuickPicker({
  value,
  onSelect,
  onBrowseAll,
  children,
}: ModelQuickPickerProps) {
  const { t } = useT();
  const visibleModels = useAppSelector(state => state.chatRuntime.visibleModels);

  // A pinned list that has lost the current selection would offer no way back
  // to it without opening the dialog, so it is always included.
  const entries =
    visibleModels.length > 0
      ? visibleModels.includes(value ?? '')
        ? visibleModels
        : [...visibleModels, ...(value ? [value] : [])]
      : value
        ? [value]
        : [];

  return (
    <PopoverRoot>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent align="start" className="w-60 p-1">
        <button
          type="button"
          data-analytics-id="model-quick-managed"
          onClick={() => onSelect(null)}
          className="flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-sm text-content-secondary hover:bg-surface-hover hover:text-content">
          <span>{t('settings.ai.managedSourceLabel')}</span>
          {value == null && <LuCheck className="h-4 w-4 text-primary-500" />}
        </button>

        {entries.map(key => (
          <button
            key={key}
            type="button"
            data-analytics-id="model-quick-select"
            onClick={() => onSelect(key)}
            className="flex w-full items-center justify-between gap-2 rounded-md px-2 py-1.5 text-left text-sm text-content-secondary hover:bg-surface-hover hover:text-content">
            <span className="min-w-0 truncate">{modelOf(key)}</span>
            {key === value && <LuCheck className="h-4 w-4 shrink-0 text-primary-500" />}
          </button>
        ))}

        <div className="my-1 border-t border-line" />
        <button
          type="button"
          data-analytics-id="model-quick-browse-all"
          onClick={onBrowseAll}
          className="flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-sm text-content hover:bg-surface-hover">
          <span>{t('composer.modelSelector')}</span>
          <LuChevronRight className="h-4 w-4 text-content-faint" />
        </button>
      </PopoverContent>
    </PopoverRoot>
  );
}
