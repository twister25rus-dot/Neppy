import { useCallback, useEffect, useState } from 'react';

import { getLocalModelPreset, setLocalModelPreset } from '../../../../services/api/localPresetApi';
import { type PresetId, PRESETS } from '../../../chat/ChatPresetPill';

/**
 * How a local model is run, as one choice rather than seven dials.
 *
 * The individual parameters stay below in `MlxServerParams` for anyone who
 * wants them, and they are the same values a preset resolves to — this section
 * is the intent, not a second set of settings. The preset is stored in core
 * config, which is also what the chat bar reads, so the two surfaces cannot
 * disagree about what is selected.
 */
export default function LocalPresetSection() {
  const [preset, setPreset] = useState<PresetId>('auto');
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void getLocalModelPreset()
      .then(value => {
        if (!cancelled) setPreset(value);
      })
      .catch(() => {
        // Unreadable means "show the default", not "block the panel".
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const choose = useCallback(
    async (next: PresetId) => {
      const previous = preset;
      setPreset(next);
      setSaving(true);
      try {
        await setLocalModelPreset(next);
      } catch {
        // Put the selection back rather than showing a choice that did not save.
        setPreset(previous);
      } finally {
        setSaving(false);
      }
    },
    [preset]
  );

  return (
    <section
      className="rounded-xl border border-line bg-surface-subtle p-3"
      data-testid="local-preset-section">
      <h3 className="text-sm font-semibold text-content">Run preset</h3>
      <p className="mt-0.5 text-xs text-content-muted">
        Auto decides per request. The parameters below stay available, and are what a preset
        resolves to.
      </p>
      <div className="mt-3 grid gap-1.5 sm:grid-cols-2" role="radiogroup" aria-label="Run preset">
        {PRESETS.map(option => {
          const selected = option.id === preset;
          return (
            <button
              key={option.id}
              type="button"
              role="radio"
              aria-checked={selected}
              disabled={saving}
              data-testid={`local-preset-${option.id}`}
              data-analytics-id="settings-local-preset"
              onClick={() => void choose(option.id)}
              className={`rounded-lg border px-3 py-2 text-left transition-colors disabled:opacity-60 ${
                selected
                  ? 'border-primary-500 bg-surface'
                  : 'border-line bg-surface hover:border-line-strong'
              }`}>
              <span className="block text-sm font-medium text-content">{option.label}</span>
              <span className="block text-xs text-content-muted">{option.hint}</span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
