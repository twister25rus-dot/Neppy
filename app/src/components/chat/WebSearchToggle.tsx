import debug from 'debug';
import { useContext, useState } from 'react';
import { LuGlobe } from 'react-icons/lu';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import { CoreStateContext } from '../../providers/coreStateContext';
import {
  getDefaultEnabledTools,
  getEnabledRustToolNames,
  normalizeEnabledToolList,
} from '../../utils/toolDefinitions';

const log = debug('neppy:chat:web-search-toggle');

const WEB_SEARCH_ID = 'web_search';

/**
 * Globe button in the composer: turns the agent's `web_search` tool on or off.
 *
 * It is the same switch as Settings → Tools → Web Search — both read and write
 * the persisted `onboardingTasks.enabledTools` list in core state (UI ids
 * expanded to the Rust tool names the session builder filters on), so the two
 * never disagree. Flips optimistically and reverts if the save fails. Renders
 * nothing outside a `CoreStateProvider` (isolated test hosts).
 */
export default function WebSearchToggle({ className }: { className?: string }) {
  const { t } = useT();
  const core = useContext(CoreStateContext);
  const [pending, setPending] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);

  if (!core) return null;

  const tasks = core.snapshot.localState.onboardingTasks;
  const persisted = tasks?.enabledTools;
  const enabledIds =
    persisted && persisted.length > 0
      ? normalizeEnabledToolList(persisted)
      : getDefaultEnabledTools();
  const persistedOn = enabledIds.includes(WEB_SEARCH_ID);
  const on = pending ?? persistedOn;

  const toggle = async () => {
    if (saving) return;
    const next = !on;
    const nextIds = next
      ? [...enabledIds.filter(id => id !== WEB_SEARCH_ID), WEB_SEARCH_ID]
      : enabledIds.filter(id => id !== WEB_SEARCH_ID);
    log('toggle web_search %s->%s', on, next);
    setPending(next);
    setSaving(true);
    try {
      await core.setOnboardingTasks({
        accessibilityPermissionGranted: tasks?.accessibilityPermissionGranted ?? false,
        localModelConsentGiven: tasks?.localModelConsentGiven ?? false,
        localModelDownloadStarted: tasks?.localModelDownloadStarted ?? false,
        enabledTools: getEnabledRustToolNames(nextIds),
        connectedSources: tasks?.connectedSources ?? [],
        updatedAtMs: Date.now(),
      });
      log('saved web_search=%s', next);
    } catch (error) {
      log('save failed, reverting error=%o', error);
    } finally {
      setPending(null);
      setSaving(false);
    }
  };

  const label = t('composer.webSearch');

  return (
    <button
      type="button"
      aria-pressed={on}
      aria-label={label}
      title={label}
      disabled={saving}
      data-testid="composer-web-search"
      data-analytics-id="chat-composer-web-search"
      onClick={() => void toggle()}
      className={cn(
        'flex size-10 shrink-0 items-center justify-center rounded-full border transition-colors',
        'focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/30',
        'disabled:cursor-wait',
        on
          ? 'border-primary-500/40 bg-primary-500/15 text-primary-500 hover:bg-primary-500/20'
          : 'border-line text-content-muted hover:bg-surface-hover hover:text-content',
        className
      )}>
      <LuGlobe aria-hidden className="h-5 w-5" />
    </button>
  );
}
