import { useCallback, useEffect, useRef, useState } from 'react';
import { LuCheck } from 'react-icons/lu';

import { useT } from '../../../../lib/i18n/I18nContext';
import { getLocalModelPreset, setLocalModelPreset } from '../../../../services/api/localPresetApi';
import { type PresetId, PRESETS } from '../../../chat/ChatPresetPill';

/**
 * How a local model is run, as one choice rather than seven dials.
 *
 * The individual parameters stay below in `MlxServerParams` for anyone who
 * wants them. The description is careful about which half of a preset a click
 * actually delivers: reasoning effort and answer length ride the next request,
 * while the context window, KV precision and memory policy are properties of
 * how the server process was launched and only change when it restarts.
 * Saying "the parameters below are what a preset resolves to" implied all of
 * it applied immediately, which is the sort of copy that sends someone hunting
 * for a bug in the wrong place.
 *
 * The preset is stored in core config, which is also what the chat bar and the
 * quick MLX menu read, so the three surfaces cannot disagree about what is
 * selected.
 *
 * Preset names and hints are deliberately NOT translated — see
 * `ChatPresetPill`, which owns that list and the reasoning behind it.
 */
export default function LocalPresetSection() {
  const { t } = useT();
  const [preset, setPreset] = useState<PresetId>('auto');
  // Which option is mid-save, so the pending one can say so without the whole
  // grid going grey. Dimming every button on click was read as the control
  // locking up rather than as it working.
  const [pending, setPending] = useState<PresetId | null>(null);
  const buttonsRef = useRef<(HTMLButtonElement | null)[]>([]);

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
      // Optimistic: the selection moves on click, so the control reacts at the
      // speed of the click rather than at the speed of the round trip.
      setPreset(next);
      setPending(next);
      try {
        await setLocalModelPreset(next);
      } catch {
        // Put the selection back rather than showing a choice that did not save.
        setPreset(previous);
      } finally {
        setPending(current => (current === next ? null : current));
      }
    },
    [preset]
  );

  /**
   * Arrow keys move between options and select as they go.
   *
   * A `radiogroup` is expected to be one tab stop with arrows moving inside it;
   * these were six independent tab stops with no arrow handling at all, which
   * is neither the semantics the role promises nor usable without a mouse.
   */
  const onKeyDown = useCallback(
    (event: React.KeyboardEvent, index: number) => {
      const delta =
        event.key === 'ArrowRight' || event.key === 'ArrowDown'
          ? 1
          : event.key === 'ArrowLeft' || event.key === 'ArrowUp'
            ? -1
            : 0;
      if (delta === 0) return;
      event.preventDefault();
      const next = (index + delta + PRESETS.length) % PRESETS.length;
      buttonsRef.current[next]?.focus();
      void choose(PRESETS[next].id);
    },
    [choose]
  );

  return (
    <section
      className="rounded-xl border border-line bg-surface-subtle p-3"
      data-testid="local-preset-section">
      <h3 className="text-sm font-semibold text-content">{t('mlx.preset.title')}</h3>
      <p className="mt-0.5 text-xs text-content-muted">{t('mlx.preset.description')}</p>
      <div
        className="mt-3 grid gap-1.5 sm:grid-cols-2"
        role="radiogroup"
        aria-label={t('mlx.preset.title')}>
        {PRESETS.map((option, index) => {
          const selected = option.id === preset;
          return (
            <button
              key={option.id}
              ref={element => {
                buttonsRef.current[index] = element;
              }}
              type="button"
              role="radio"
              aria-checked={selected}
              aria-busy={pending === option.id}
              // One tab stop for the group, which is what the role promises.
              tabIndex={selected ? 0 : -1}
              data-testid={`local-preset-${option.id}`}
              data-analytics-id="settings-local-preset"
              onClick={() => void choose(option.id)}
              onKeyDown={event => onKeyDown(event, index)}
              // The selected state is carried by the SURFACE, not by a 1px
              // border: at this size a border colour change was easy to miss,
              // which is what made a click read as nothing happening. `active:`
              // supplies the press itself, and the tick removes any ambiguity
              // about which of six is current.
              className={`flex items-start gap-2 rounded-lg border px-3 py-2 text-left transition-all active:scale-[0.99] ${
                selected
                  ? 'border-primary-500 bg-primary-500/10 ring-1 ring-primary-500/40'
                  : 'border-line bg-surface hover:border-line-strong hover:bg-surface-hover active:bg-surface-hover'
              }`}>
              <LuCheck
                aria-hidden
                className={`mt-0.5 h-3.5 w-3.5 shrink-0 ${
                  selected ? 'text-primary-500' : 'opacity-0'
                }`}
              />
              <span className="min-w-0">
                <span
                  className={`block text-sm font-medium ${
                    selected ? 'text-content' : 'text-content-secondary'
                  }`}>
                  {option.label}
                </span>
                <span className="block text-xs text-content-muted">{option.hint}</span>
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
