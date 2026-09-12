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
 * The scale, low to high. The ids are the wire values and must not change; the
 * labels name the decision rather than the knob — the question is whether you
 * want an answer now or a better one.
 */
const EFFORT_STOPS = [
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
  // An unset effort shows the first stop as current without having been chosen,
  // so the control always reads as *something* rather than rendering blank.
  const activeIndex = Math.max(
    0,
    EFFORT_STOPS.findIndex(stop => stop.id === value.effort)
  );

  return (
    <div
      role="radiogroup"
      aria-label={t('chat.thinking.label')}
      data-testid="composer-effort"
      className={`flex items-center gap-0.5 rounded-full bg-surface-strong p-0.5 ${className ?? ''}`}>
      {EFFORT_STOPS.map((stop, index) => {
        const selected = index === activeIndex && value.effort != null;
        return (
          <button
            key={stop.id}
            type="button"
            role="radio"
            aria-checked={selected}
            data-analytics-id={`composer-effort-${stop.id}`}
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
