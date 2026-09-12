import { LuSettings } from 'react-icons/lu';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { entryRoute, findEntryById } from '../../settings/settingsRouteRegistry';
import { Button, PopoverContent, PopoverRoot, PopoverTrigger, Tooltip } from '../../ui';

/**
 * The settings people reach for repeatedly, in the order they reach for them.
 *
 * Ids rather than hand-written labels and routes: the registry already owns
 * both, so a renamed panel follows automatically instead of leaving a dead row
 * here. An id that stops resolving drops out of the menu rather than rendering
 * a link to nowhere — see the filter below.
 */
const QUICK_IDS = ['llm', 'agent-access', 'appearance', 'tools', 'account'] as const;

/**
 * Small settings affordance in the top-right of the content area.
 *
 * The full settings page is four clicks deep from a conversation, and the
 * handful of panels worth changing mid-task are always the same ones. This is
 * those, one click away, without leaving the thread.
 *
 * Top-RIGHT specifically: on macOS the top-left belongs to the traffic lights,
 * and the shell already reserves that band (`WindowDragBar`). The right side of
 * that strip is empty at every window width.
 */
export default function QuickSettingsButton() {
  const { t } = useT();
  const navigate = useNavigate();

  // Resolved once per render against the registry, so a removed panel simply is
  // not offered.
  const entries = QUICK_IDS.map(id => findEntryById(id)).filter(
    (entry): entry is NonNullable<typeof entry> => entry != null
  );

  return (
    <PopoverRoot>
      <Tooltip label={t('nav.settings')}>
        <PopoverTrigger asChild>
          <Button
            type="button"
            iconOnly
            variant="tertiary"
            size="xs"
            aria-label={t('nav.settings')}
            analyticsId="quick-settings-open"
            data-testid="quick-settings-trigger"
            className="h-7 w-7 rounded-md text-content-muted hover:text-content-secondary">
            <LuSettings className="h-4 w-4" />
          </Button>
        </PopoverTrigger>
      </Tooltip>
      <PopoverContent align="end" className="w-56 p-1">
        {entries.map(entry => (
          <button
            key={entry.id}
            type="button"
            data-analytics-id="quick-settings-entry"
            onClick={() => navigate(`/settings/${entryRoute(entry)}`)}
            className="block w-full rounded-md px-2 py-1.5 text-left text-sm text-content-secondary hover:bg-surface-hover hover:text-content">
            {t(entry.titleKey)}
          </button>
        ))}
        <div className="my-1 border-t border-line" />
        <button
          type="button"
          data-analytics-id="quick-settings-all"
          onClick={() => navigate('/settings')}
          className="block w-full rounded-md px-2 py-1.5 text-left text-sm font-medium text-content hover:bg-surface-hover">
          {t('nav.settings')}
        </button>
      </PopoverContent>
    </PopoverRoot>
  );
}
