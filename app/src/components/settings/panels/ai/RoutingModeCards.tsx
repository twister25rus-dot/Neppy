/*
 * Top-level routing-mode picker. These options are a real radio group: one
 * routing mode is always active, and the primitive supplies its keyboard and
 * screen-reader behavior.
 */
import { cn } from '../../../../lib/cn';
import { useT } from '../../../../lib/i18n/I18nContext';
import Label from '../../../ui/Label';
import { RadioGroupItem, RadioGroupRoot } from '../../../ui/RadioGroup';
import type { RoutingMode } from './aiPanelTypes';

/** One cell of the segmented control. Selected-ness comes from the resolved
 *  mode rather than a `:has()` selector, so it works on the macOS 12 floor. */
const ModeOption = ({
  value,
  selected,
  title,
  description,
}: {
  value: RoutingMode;
  selected: boolean;
  title: string;
  description: string;
}) => (
  <Label
    data-slot="routing-mode-option"
    data-selected={selected}
    className={cn(
      'relative flex min-h-36 cursor-pointer flex-col items-start gap-3 rounded-xl border p-5 transition-colors',
      'focus-within:outline-hidden focus-within:ring-2 focus-within:ring-primary-500/25',
      selected
        ? 'border-sage-500/60 bg-sage-50 dark:bg-sage-500/10'
        : 'border-line bg-surface hover:border-line-strong hover:bg-surface-hover'
    )}>
    <RadioGroupItem value={value} size="md" className="sr-only" />
    <span className="flex min-w-0 flex-col gap-3">
      <span className="text-base font-semibold text-content">{title}</span>
      <span className="text-sm leading-6 text-content-muted">{description}</span>
    </span>
  </Label>
);

export const RoutingModeCards = ({
  effectiveRoutingMode,
  onSelectManaged,
  onSelectOwn,
  onSelectCustom,
}: {
  effectiveRoutingMode: RoutingMode;
  onSelectManaged: () => void;
  onSelectOwn: () => void;
  onSelectCustom: () => void;
}) => {
  const { t } = useT();
  return (
    <section className="flex w-full flex-col gap-4">
      <div>
        <h2 className="text-lg font-semibold tracking-tight text-content">
          {t('settings.ai.routing')}
        </h2>
        <p className="mt-1 text-sm text-content-muted">{t('settings.ai.routingDesc')}</p>
      </div>
      <RadioGroupRoot
        aria-label={t('settings.ai.routing')}
        value={effectiveRoutingMode}
        onValueChange={next => {
          if (next === 'managed') onSelectManaged();
          else if (next === 'own') onSelectOwn();
          else if (next === 'custom') onSelectCustom();
        }}
        className="grid w-full gap-3 md:grid-cols-3">
        <ModeOption
          value="managed"
          selected={effectiveRoutingMode === 'managed'}
          title={t('settings.ai.routing.managed')}
          description={t('settings.ai.routing.managedDesc')}
        />
        <ModeOption
          value="own"
          selected={effectiveRoutingMode === 'own'}
          title={t('settings.ai.routing.useYourOwn')}
          description={t('settings.ai.routing.useYourOwnDesc')}
        />
        <ModeOption
          value="custom"
          selected={effectiveRoutingMode === 'custom'}
          title={t('settings.ai.routing.advanced')}
          description={t('settings.ai.routing.advancedDesc')}
        />
      </RadioGroupRoot>
    </section>
  );
};

export default RoutingModeCards;
