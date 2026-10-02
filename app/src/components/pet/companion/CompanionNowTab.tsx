import debug from 'debug';
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import type {
  CompanionSuggestion,
  PermissionKind,
  PermissionState,
  SuggestionAction,
} from '../../../services/api/petCompanionApi';
import { useAppDispatch } from '../../../store/hooks';
import { createNewThread } from '../../../store/threadSlice';
import Badge, { type BadgeVariant } from '../../ui/Badge';
import Button from '../../ui/Button';
import { formatDateTime, formatRelative } from '../petFormat';
import CompanionDataDialog from './CompanionDataDialog';
import CompanionSuggestionCard from './CompanionSuggestionCard';
import type { UseCompanion } from './useCompanion';

const log = debug('pet:companion:now');

const STATE_VARIANT: Record<string, BadgeVariant> = {
  off: 'neutral',
  observing: 'success',
  observingScreen: 'success',
  paused: 'warning',
  suspended: 'danger',
};

const permissionLabelKey = (state: PermissionState): string =>
  state === 'granted'
    ? 'pet.companion.permission.granted'
    : state === 'unsupported'
      ? 'pet.companion.permission.unsupported'
      : 'pet.companion.permission.missing';

interface CompanionNowTabProps {
  companion: UseCompanion;
  onOpenSettings: () => void;
  /** Called after data was deleted so the page can refresh. */
  onChanged?: () => void;
}

/** What the desktop companion is doing right now, what it noticed, and what it suggests. */
export default function CompanionNowTab({
  companion,
  onOpenSettings,
  onChanged,
}: CompanionNowTabProps) {
  const { t, locale } = useT();
  const dispatch = useAppDispatch();
  const navigate = useNavigate();
  const { settings, status, suggestions, displayState } = companion;
  const [controlBusy, setControlBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showData, setShowData] = useState(false);

  const enabled = settings?.enabled === true;

  const runControl = async (fn: () => Promise<void>) => {
    setControlBusy(true);
    setError(null);
    try {
      await fn();
    } catch (err) {
      log('control failed: %o', err);
      setError(t('pet.companion.errors.controlFailed'));
    } finally {
      setControlBusy(false);
    }
  };

  const openChat = async (suggestion: CompanionSuggestion) => {
    // Create the chat first: if that fails nothing has been recorded yet.
    const thread = await dispatch(createNewThread(undefined)).unwrap();
    const result = await companion.act(suggestion.id, 'open_chat');
    log('open chat id=%s seeded=%s', suggestion.id, Boolean(result.chat_prompt));
    navigate(`/chat/${thread.id}`, {
      state: {
        openThreadId: thread.id,
        ...(result.chat_prompt ? { composerSeed: result.chat_prompt } : {}),
      },
    });
  };

  if (!enabled) {
    return (
      <div className="space-y-3" data-testid="companion-now-tab">
        <div
          data-testid="companion-off"
          className="rounded-2xl border border-dashed border-line-strong px-6 py-10 text-center">
          <h3 className="text-sm font-semibold text-content">{t('pet.companion.now.offTitle')}</h3>
          <p className="mx-auto mt-1 max-w-md text-sm text-content-muted">
            {t('pet.companion.now.offBody')}
          </p>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            className="mt-4"
            analyticsId="pet-companion-open-settings"
            data-testid="companion-open-settings"
            onClick={onOpenSettings}>
            {t('pet.companion.now.openSettings')}
          </Button>
        </div>
      </div>
    );
  }

  const paused = displayState === 'paused';
  const pausedUntil = formatRelative(status?.paused_until, locale);
  const visible = suggestions.filter(s => s.state !== 'dismissed' && s.state !== 'expired');
  const drops = Object.entries(status?.metrics.drops_by_reason ?? {}).filter(
    ([, n]) => (n ?? 0) > 0
  );
  const permissions: Array<{ kind: PermissionKind; labelKey: string; state: PermissionState }> = [
    {
      kind: 'accessibility',
      labelKey: 'pet.companion.permission.accessibility',
      state: status?.permissions.accessibility ?? 'unknown',
    },
  ];
  if (settings?.sources.screen_capture) {
    permissions.push({
      kind: 'screen_recording',
      labelKey: 'pet.companion.permission.screenRecording',
      state: status?.permissions.screen_recording ?? 'unknown',
    });
  }

  return (
    <div className="space-y-5" data-testid="companion-now-tab">
      <section
        data-testid="companion-state-card"
        className="space-y-3 rounded-2xl border border-line bg-surface p-4 shadow-subtle">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Badge variant={STATE_VARIANT[displayState]} data-testid="companion-state">
              {t(`pet.companion.state.${displayState}`)}
            </Badge>
            {paused && pausedUntil && (
              <span className="text-xs text-content-muted">
                {t('pet.companion.pausedUntil').replace('{when}', pausedUntil)}
              </span>
            )}
          </div>
          <div className="flex flex-wrap items-center gap-2">
            {paused ? (
              <Button
                type="button"
                variant="primary"
                size="sm"
                analyticsId="pet-companion-resume"
                data-testid="companion-resume"
                disabled={controlBusy}
                onClick={() => void runControl(() => companion.resume())}>
                {t('pet.companion.resume')}
              </Button>
            ) : (
              <>
                <Button
                  type="button"
                  variant="primary"
                  size="sm"
                  analyticsId="pet-companion-pause"
                  data-testid="companion-pause"
                  disabled={controlBusy}
                  onClick={() => void runControl(() => companion.pause())}>
                  {t('pet.companion.pause')}
                </Button>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  analyticsId="pet-companion-pause-1h"
                  data-testid="companion-pause-1h"
                  disabled={controlBusy}
                  onClick={() => void runControl(() => companion.pause(60))}>
                  {t('pet.companion.pause1h')}
                </Button>
              </>
            )}
          </div>
        </div>
        {displayState === 'suspended' && status?.suspended_reason && (
          <p data-testid="companion-suspended" className="text-xs text-content-secondary">
            {t(`pet.companion.suspended.${status.suspended_reason}`)}
          </p>
        )}
        {(displayState === 'observing' || displayState === 'observingScreen') && (
          <p className="text-xs text-content-muted">{t('pet.companion.now.observingHint')}</p>
        )}
        {error && (
          <p role="alert" className="text-xs text-coral-600 dark:text-coral-400">
            {error}
          </p>
        )}
        <ul className="flex flex-wrap gap-2" data-testid="companion-permissions">
          {permissions.map(p => (
            <li
              key={p.kind}
              className="flex items-center gap-2 rounded-full border border-line px-3 py-1 text-xs text-content-secondary">
              <span>
                {t(p.labelKey)}: {t(permissionLabelKey(p.state))}
              </span>
              {(p.state === 'denied' || p.state === 'unknown') && status?.platform_supported && (
                <Button
                  type="button"
                  variant="tertiary"
                  size="xs"
                  analyticsId={`pet-companion-grant-${p.kind}`}
                  data-testid={`companion-grant-${p.kind}`}
                  onClick={() => void runControl(() => companion.requestPermission(p.kind))}>
                  {t('pet.companion.permission.grant')}
                </Button>
              )}
            </li>
          ))}
        </ul>
      </section>

      <section className="space-y-2" data-testid="companion-sees">
        <h3 className="text-sm font-semibold text-content">{t('pet.companion.now.seesHeading')}</h3>
        {(status?.recent.length ?? 0) === 0 ? (
          <p className="text-xs italic text-content-faint">{t('pet.companion.now.seesEmpty')}</p>
        ) : (
          <ul className="space-y-1">
            {status?.recent.slice(0, 20).map((o, i) => (
              <li
                key={`${o.at}-${i}`}
                data-testid="companion-observation"
                className="flex flex-wrap items-baseline gap-x-2 text-xs text-content-secondary">
                <span className="font-medium text-content">{o.app_name}</span>
                <span>{t(`pet.companion.obs.${o.kind}`)}</span>
                {o.title_excerpt && (
                  <span className="truncate text-content-muted">{o.title_excerpt}</span>
                )}
                {o.dropped && (
                  <Badge variant="warning">
                    {t('pet.companion.now.skipped').replace(
                      '{reason}',
                      t(`pet.companion.drop.${o.dropped}`)
                    )}
                  </Badge>
                )}
                <span className="text-content-faint">{formatDateTime(o.at, locale)}</span>
              </li>
            ))}
          </ul>
        )}
        {drops.length > 0 && (
          <ul className="flex flex-wrap gap-2" data-testid="companion-drops">
            {drops.map(([reason, n]) => (
              <li key={reason}>
                <Badge>{`${t(`pet.companion.drop.${reason}`)}: ${n}`}</Badge>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section className="space-y-2">
        <div className="flex items-center justify-between gap-2">
          <h3 className="text-sm font-semibold text-content">
            {t('pet.companion.now.suggestionsHeading')}
          </h3>
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            analyticsId="pet-companion-view-data"
            data-testid="companion-view-data"
            onClick={() => setShowData(true)}>
            {t('pet.companion.data.view')}
          </Button>
        </div>
        {visible.length === 0 ? (
          <p className="text-xs italic text-content-faint" data-testid="companion-no-suggestions">
            {t('pet.companion.now.suggestionsEmpty')}
          </p>
        ) : (
          <ul className="space-y-2">
            {visible.map(s => (
              <CompanionSuggestionCard
                key={s.id}
                suggestion={s}
                onAct={(action: SuggestionAction, text?: string) =>
                  companion.act(s.id, action, text)
                }
                onOpenChat={openChat}
              />
            ))}
          </ul>
        )}
      </section>

      {showData && (
        <CompanionDataDialog
          onClose={() => setShowData(false)}
          onChanged={() => {
            void companion.refresh();
            onChanged?.();
          }}
        />
      )}
    </div>
  );
}
