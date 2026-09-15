import { useT } from '../../lib/i18n/I18nContext';

/**
 * How hard to think on the next turn, as the OpenAI-compatible
 * `reasoning_effort` wire value. `null` means "leave it to the provider": the
 * turn runs on whatever the role already resolves to, and nothing is sent.
 */
export interface ComposerEffort {
  effort: string | null;
}

export const EMPTY_EFFORT: ComposerEffort = { effort: null };

/**
 * The scale, lowest first. The ids are the wire values and must not change; the
 * labels name the decision rather than the knob — the question is whether you
 * want an answer now or a better one.
 *
 * `Auto` is a real stop, not a blank state. `null` means "send nothing and let
 * the run preset decide", which is the default and the only way a preset's own
 * reasoning level can take effect — but as an *unrepresented* value it left the
 * control with no segment lit at rest, which reads as a broken or decorative
 * row of buttons rather than as a setting. Giving that value a stop makes the
 * resting state legible and gives the override somewhere to return to.
 */
const EFFORT_STOPS = [
  { id: null, labelKey: 'chat.thinking.auto' },
  { id: 'low', labelKey: 'chat.thinking.quick' },
  { id: 'medium', labelKey: 'chat.thinking.balanced' },
  { id: 'high', labelKey: 'chat.thinking.thorough' },
] as const;

interface ComposerEffortPillProps {
  value: ComposerEffort;
  onChange: (next: ComposerEffort) => void;
  className?: string;
}

/**
 * Inline segmented control in the composer, beside the model chip.
 *
 * This replaced an icon button that opened a popover holding `temperature`,
 * `top_p`, `max_tokens` and a slider. Those three were raw provider knobs with
 * no answer to "what do I set this to", and putting the one setting people
 * actually reach for behind a click made it invisible. The whole control is the
 * scale now: the current setting is readable without opening anything, and
 * changing it is one click rather than three.
 *
 * Radio semantics rather than buttons — it is a single choice from a fixed set,
 * and that is what a screen reader should hear. Arrow keys move between stops
 * by the browser's own radiogroup handling.
 */
export default function ComposerEffortPill({
  value,
  onChange,
  className,
}: ComposerEffortPillProps) {
  const { t } = useT();

  return (
    <div
      role="radiogroup"
      aria-label={t('chat.thinking.label')}
      data-testid="composer-effort"
      className={`flex items-center gap-0.5 rounded-full bg-surface-strong p-0.5 ${className ?? ''}`}>
      {EFFORT_STOPS.map(stop => {
        const selected = stop.id === value.effort;
        return (
          <button
            key={stop.id ?? 'auto'}
            type="button"
            role="radio"
            aria-checked={selected}
            data-analytics-id={`composer-effort-${stop.id ?? 'auto'}`}
            onClick={() => onChange({ effort: stop.id })}
            className={`rounded-full px-2.5 py-1 text-xs transition-colors ${
              selected
                ? 'bg-surface font-medium text-content shadow-xs'
                : 'text-content-muted hover:text-content-secondary'
            }`}>
            {t(stop.labelKey)}
          </button>
        );
      })}
    </div>
  );
}
