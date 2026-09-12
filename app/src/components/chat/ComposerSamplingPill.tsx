import { LuSlidersHorizontal } from 'react-icons/lu';

import { useT } from '../../lib/i18n/I18nContext';
import Button from '../ui/Button';
import { PopoverContent, PopoverRoot, PopoverTrigger } from '../ui/Popover';
import Slider from '../ui/Slider';

/** Generation settings for one turn. `null` means "leave it to the provider". */
export interface ComposerSampling {
  temperature: number | null;
  topP: number | null;
  maxTokens: number | null;
  /**
   * How hard to think this turn, as the OpenAI-compatible `reasoning_effort`
   * wire value. It lives here rather than beside the model chip because it is
   * a per-turn generation setting like the three below it, and it reaches the
   * provider through the same send params.
   */
  effort: string | null;
}

export const EMPTY_SAMPLING: ComposerSampling = {
  temperature: null,
  topP: null,
  maxTokens: null,
  effort: null,
};

/**
 * The scale, low to high. The ids are the wire values and must not change; the
 * labels are what the slider reads out, and they name the decision being made
 * rather than the knob: the question is whether you want an answer now or a
 * better one.
 */
const EFFORT_STOPS = [
  { id: 'low', labelKey: 'chat.thinking.quick' },
  { id: 'medium', labelKey: 'chat.thinking.balanced' },
  { id: 'high', labelKey: 'chat.thinking.thorough' },
] as const;

interface ComposerSamplingPillProps {
  value: ComposerSampling;
  onChange: (next: ComposerSampling) => void;
  className?: string;
}

/**
 * Per-turn generation settings, next to the model chip.
 *
 * The field labels are the wire parameter names and are deliberately NOT
 * translated. That is the rule `MlxServerParams` already follows for the same
 * values: they are the names the provider reads, so a translated label would
 * name a field that does not exist. Only the control's own accessible name is
 * translated, and it reuses an existing key rather than adding one to fourteen
 * locale files for two words.
 *
 * Empty means unset: the turn runs on whatever the role already resolves to,
 * which is why a cleared field sends nothing rather than sending a zero.
 */
export default function ComposerSamplingPill({
  value,
  onChange,
  className,
}: ComposerSamplingPillProps) {
  const { t } = useT();
  const active =
    value.temperature != null ||
    value.topP != null ||
    value.maxTokens != null ||
    value.effort != null;

  // An unset effort parks the thumb at the first stop rather than leaving the
  // track handle-less; the state stays null until the slider is actually moved,
  // so an untouched control still sends nothing.
  const effortIndex = Math.max(
    0,
    EFFORT_STOPS.findIndex(stop => stop.id === value.effort)
  );

  const set = (key: keyof ComposerSampling) => (raw: string) => {
    const trimmed = raw.trim();
    if (trimmed.length === 0) {
      onChange({ ...value, [key]: null });
      return;
    }
    const parsed = Number(trimmed);
    // Ignore junk rather than storing NaN, which would serialise as null on the
    // wire and read as "unset" — silently different from what was typed.
    if (!Number.isFinite(parsed)) return;
    onChange({ ...value, [key]: parsed });
  };

  const field = (key: keyof ComposerSampling, label: string, step: string, placeholder: string) => (
    <label className="flex items-center justify-between gap-3 text-xs">
      <span className="font-mono text-content-secondary">{label}</span>
      <input
        type="number"
        step={step}
        inputMode="decimal"
        placeholder={placeholder}
        aria-label={label}
        value={value[key] ?? ''}
        onChange={event => set(key)(event.target.value)}
        className="w-24 rounded-md border border-line bg-surface px-2 py-1 text-right tabular-nums"
      />
    </label>
  );

  return (
    <PopoverRoot>
      <PopoverTrigger asChild>
        <Button
          type="button"
          iconOnly
          variant="tertiary"
          size="xs"
          aria-label={t('mlx.params.generation')}
          title={t('mlx.params.generation')}
          analyticsId="chat-sampling-controls"
          data-testid="composer-sampling-trigger"
          className={`${active ? 'text-primary' : ''} ${className ?? ''}`}>
          <LuSlidersHorizontal className="h-4 w-4" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-56 space-y-2 p-3">
        <div className="space-y-1.5 border-b border-line pb-2.5" data-testid="composer-effort">
          <span className="text-xs text-content-secondary">{t('chat.thinking.label')}</span>
          <Slider
            size="sm"
            min={0}
            max={EFFORT_STOPS.length - 1}
            step={1}
            value={[effortIndex]}
            onValueChange={([next]) => {
              const stop = EFFORT_STOPS[next ?? 0];
              if (stop) onChange({ ...value, effort: stop.id });
            }}
            thumbLabels={[t('chat.thinking.label')]}
            data-testid="composer-effort-slider"
          />
          {/* The scale itself: three labels under the three stops the thumb can
              occupy, with only the active one at full contrast, so the current
              setting is readable without a second row repeating it. */}
          <div aria-hidden="true" className="flex justify-between gap-1 text-[10px]">
            {EFFORT_STOPS.map((stop, index) => (
              <span
                key={stop.id}
                className={
                  index === effortIndex && value.effort != null
                    ? 'font-medium text-content-primary'
                    : 'text-content-faint'
                }>
                {t(stop.labelKey)}
              </span>
            ))}
          </div>
        </div>
        <div className="space-y-2" data-testid="composer-sampling-fields">
          {field('temperature', 'temperature', '0.05', 'default')}
          {field('topP', 'top_p', '0.05', 'default')}
          {field('maxTokens', 'max_tokens', '1', 'default')}
        </div>
        <Button
          type="button"
          variant="tertiary"
          size="xs"
          analyticsId="chat-sampling-reset"
          disabled={!active}
          onClick={() => onChange(EMPTY_SAMPLING)}
          className="w-full">
          {t('common.reset')}
        </Button>
      </PopoverContent>
    </PopoverRoot>
  );
}
