import { useEffect, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import { listProviderModels } from '../../../../services/api/aiSettingsApi';
import { toggleVisibleModel } from '../../../../store/chatRuntimeSlice';
import { useAppDispatch, useAppSelector } from '../../../../store/hooks';
import type { CloudProvider } from './aiPanelTypes';
import { ProviderSwatch } from './ProviderListRow';

interface ChatModelVisibilitySectionProps {
  cloudProviders: CloudProvider[];
}

/**
 * Chooses which models the composer's quick picker offers.
 *
 * The picker is a short list by design, and before this there was no way to say
 * what belonged on it — it could only fall back to whatever was already
 * selected. Pick a provider here, tick the models worth reaching for, and they
 * are one click away in chat; everything else stays reachable through the full
 * dialog.
 *
 * Deliberately its own section rather than an expansion of the provider chips
 * above: those chips already own a click (it opens the key dialog), and
 * overloading it would make "press the provider" mean two things depending on
 * where you landed.
 */
export default function ChatModelVisibilitySection({
  cloudProviders,
}: ChatModelVisibilitySectionProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const visibleModels = useAppSelector(state => state.chatRuntime.visibleModels);
  const [selectedSlug, setSelectedSlug] = useState<string | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!selectedSlug) {
      setModels([]);
      return;
    }
    let active = true;
    setLoading(true);
    setError(null);
    void listProviderModels(selectedSlug)
      .then(list => {
        if (!active) return;
        setModels(list.map(entry => entry.id));
        setLoading(false);
      })
      .catch(() => {
        if (!active) return;
        setLoading(false);
        setError(t('settings.ai.modelsLoadFailed', 'Could not load models from this provider.'));
      });
    return () => {
      active = false;
    };
    // Keyed on the slug alone — see ProviderModelPickerDialog for why depending
    // on an object prop here is how a refetch loop starts.
  }, [selectedSlug, t]);

  if (cloudProviders.length === 0) return null;

  return (
    <section className="mt-4 rounded-xl border border-line bg-surface-subtle p-3">
      <h3 className="text-sm font-semibold text-content">
        {t('settings.ai.chatModels.title', 'Models shown in chat')}
      </h3>
      <p className="mt-0.5 text-xs text-content-muted">
        {t(
          'settings.ai.chatModels.description',
          'Pick a provider, then tick the models to offer in the chat model menu.'
        )}
      </p>

      <div className="mt-3 flex flex-wrap gap-1.5">
        {cloudProviders.map(provider => {
          const selected = provider.slug === selectedSlug;
          return (
            <button
              key={provider.id}
              type="button"
              data-analytics-id="chat-models-provider"
              data-testid={`chat-models-provider-${provider.slug}`}
              onClick={() => setSelectedSlug(selected ? null : provider.slug)}
              className={`flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs transition-colors active:scale-[0.99] ${
                selected
                  ? 'border-primary-500 bg-surface text-content ring-1 ring-primary-500/40'
                  : 'border-line bg-surface text-content-muted hover:border-line-strong hover:text-content-secondary'
              }`}>
              <ProviderSwatch slug={provider.slug} label={provider.label} tone="" />
              {provider.label}
            </button>
          );
        })}
      </div>

      {selectedSlug && (
        <div className="mt-3 space-y-1" data-testid="chat-models-list">
          {loading && <p className="text-xs text-content-faint">{t('common.loading')}</p>}
          {error && <p className="text-xs text-coral-600">{error}</p>}
          {!loading && !error && models.length === 0 && (
            <p className="text-xs text-content-faint">
              {t('settings.ai.chatModels.none', 'No models reported by this provider.')}
            </p>
          )}
          {models.map(model => {
            const key = `${selectedSlug}:${model}`;
            return (
              <label
                key={key}
                className="flex cursor-pointer items-center gap-2 rounded-md px-2 py-1 text-sm text-content-secondary hover:bg-surface-hover">
                <input
                  type="checkbox"
                  checked={visibleModels.includes(key)}
                  onChange={() => dispatch(toggleVisibleModel(key))}
                  data-testid={`chat-models-toggle-${model}`}
                />
                <span className="min-w-0 truncate">{model}</span>
              </label>
            );
          })}
        </div>
      )}
    </section>
  );
}
