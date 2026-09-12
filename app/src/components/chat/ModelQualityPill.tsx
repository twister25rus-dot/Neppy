import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { loadAISettings } from '../../services/api/aiSettingsApi';
import { type CloudProvider } from '../settings/panels/ai/aiPanelTypes';
import type { OllamaModel } from '../settings/panels/ai/aiPanelTypes';
import {
  ProviderModelPickerDialog,
  type ProviderModelSelection,
} from '../settings/panels/ai/ProviderModelPickerDialog';
import { Button } from '../ui';
import ModelQuickPicker from './ModelQuickPicker';

interface ModelQualityPillProps {
  className?: string;
  value?: string | null;
  /**
   * `null` means "managed" — the host clears its override and routing falls
   * back to whatever the product picks. It is deliberately not the empty
   * string: the host resolves its override with `??`, which passes `''`
   * straight through as a real (and empty) model id.
   */
  onValueChange?: (value: string | null, contextWindow?: number | null) => void;
}

/**
 * Hoisted so its identity is stable across renders.
 *
 * Written inline as an empty array literal, this was a NEW array every render, and
 * the dialog's catalog effect depends on that prop — so it re-ran, cleared the
 * catalog, refetched, re-rendered, and produced another `[]`. The models list
 * never settled: it sat on "Loading models…" and started over every couple of
 * seconds. The composer pill has no Ollama list to offer, and "none" is a
 * constant.
 */
const NO_LOCAL_MODELS: OllamaModel[] = [];

function selectionFromValue(value: string | null | undefined): ProviderModelSelection | null {
  if (!value || value.startsWith('hint:')) return null;
  const separator = value.indexOf(':');
  if (separator <= 0 || separator === value.length - 1) return null;
  return {
    source: { kind: 'cloud', providerSlug: value.slice(0, separator) },
    model: value.slice(separator + 1),
  };
}

function selectionValue(selection: ProviderModelSelection): string | null {
  const { source, model } = selection;
  switch (source.kind) {
    // Managed has no model id to encode — clearing the override IS the choice.
    case 'managed':
      return null;
    case 'cloud':
      return `${source.providerSlug}:${model}`;
    case 'local':
      return `ollama:${model}`;
    case 'claude-code':
      return `claude-code:${model}`;
  }
}

function displayValue(value: string | null | undefined): string {
  if (!value || value.startsWith('hint:')) return 'Neppy';
  const separator = value.indexOf(':');
  return separator >= 0 ? value.slice(separator + 1) : value;
}

/**
 * assistant-ui's compact model-selector trigger, backed by Neppy's shared
 * provider/model picker so configured providers and their model discovery stay
 * consistent with routing.
 */
export default function ModelQualityPill({
  className,
  value,
  onValueChange,
}: ModelQualityPillProps) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const [providers, setProviders] = useState<CloudProvider[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open || providers.length > 0) return;
    let active = true;
    setLoading(true);
    void loadAISettings()
      .then(settings => {
        if (!active) return;
        setProviders(
          settings.cloudProviders.map(provider => ({
            id: provider.id,
            slug: provider.slug,
            label: provider.label,
            endpoint: provider.endpoint,
            authStyle: provider.auth_style,
            maskedKey: provider.has_api_key ? '••••' : '',
          }))
        );
      })
      .finally(() => {
        if (active) setLoading(false);
      });
    return () => {
      active = false;
    };
  }, [open, providers.length]);

  const initial = useMemo(() => selectionFromValue(value), [value]);

  return (
    <>
      <ModelQuickPicker
        value={value}
        onSelect={next => {
          onValueChange?.(next);
        }}
        onBrowseAll={() => setOpen(true)}>
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          analyticsId="chat-model-selector"
          aria-label={t('composer.modelSelector')}
          title={t('composer.modelSelector')}
          disabled={!onValueChange || loading}
          className={`h-7 min-w-0 rounded-md px-2 text-xs text-content-muted hover:bg-surface-hover hover:text-content ${className ?? ''}`}>
          <span className="min-w-0 truncate font-medium">
            {loading ? 'Loading models…' : displayValue(value)}
          </span>
        </Button>
      </ModelQuickPicker>
      {open && !loading && (
        <ProviderModelPickerDialog
          cloudProviders={providers}
          localModels={NO_LOCAL_MODELS}
          ollamaRunning={false}
          claudeCodeEnabled={false}
          initial={initial}
          onClose={() => setOpen(false)}
          onSelect={selection => {
            onValueChange?.(selectionValue(selection), selection.contextWindow);
            setOpen(false);
          }}
        />
      )}
    </>
  );
}
