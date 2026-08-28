import debug from 'debug';
import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  getLocalMode,
  type LocalModeStatus,
  type LocalReplacementKind,
  type LocalServiceEntry,
  setLocalMode,
} from '../../../services/api/localModeApi';
import {
  SettingsBadge,
  type SettingsBadgeVariant,
  SettingsRow,
  SettingsSection,
  SettingsStatusLine,
  SettingsSwitch,
} from '../controls';

const log = debug('local-mode');

type Status = 'loading' | 'idle' | 'saving' | 'saved' | 'error';

/**
 * Badge colour per replacement kind. `requires_setup` is a warning rather than
 * an error: the feature is present and one step away, which is a different
 * message from "this cannot work here".
 */
const KIND_BADGE_VARIANT: Record<LocalReplacementKind, SettingsBadgeVariant> = {
  replaced: 'success',
  requires_setup: 'warning',
  unavailable: 'neutral',
  not_applicable: 'neutral',
};

const KIND_LABEL_KEY: Record<LocalReplacementKind, string> = {
  replaced: 'localMode.kind.replaced',
  requires_setup: 'localMode.kind.requiresSetup',
  unavailable: 'localMode.kind.unavailable',
  not_applicable: 'localMode.kind.notApplicable',
};

function ServiceRow({ entry }: { entry: LocalServiceEntry }) {
  const { t } = useT();
  return (
    <li className="p-4" data-testid={`local-mode-service-${entry.id}`}>
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <p className="text-sm font-medium text-content">{entry.hosted}</p>
          <p className="text-xs text-content-muted mt-1 leading-relaxed">
            {entry.local_alternative}
          </p>
          {entry.setup ? (
            <p className="text-xs text-content-faint mt-1 leading-relaxed">
              {t('localMode.setupPrefix')} {entry.setup}
            </p>
          ) : null}
        </div>
        <SettingsBadge variant={KIND_BADGE_VARIANT[entry.kind]}>
          {t(KIND_LABEL_KEY[entry.kind])}
        </SettingsBadge>
      </div>
    </li>
  );
}

/**
 * Local Mode settings: the switch, and the inventory of what each hosted
 * service becomes once it is on.
 *
 * The inventory is the point of this panel. Local Mode is not a toggle whose
 * effect a user can guess — some features are replaced outright, some need a
 * local service stood up first, and a few are simply gone. Showing the whole
 * list next to the switch is what makes the decision an informed one, and the
 * list comes from the core rather than this file so it cannot drift from what
 * the backend actually does.
 */
const LocalModeSection = () => {
  const { t } = useT();
  const [state, setState] = useState<LocalModeStatus | null>(null);
  const [status, setStatus] = useState<Status>('loading');
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    getLocalMode()
      .then(next => {
        if (cancelled) return;
        log('[local-mode] loaded', { enabled: next.enabled, active: next.active });
        setState(next);
        setStatus('idle');
      })
      .catch(err => {
        if (cancelled) return;
        console.warn('[local-mode] failed to load:', err);
        setError(err instanceof Error ? err.message : String(err));
        setStatus('error');
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const patch = useCallback(async (next: Parameters<typeof setLocalMode>[0]) => {
    setStatus('saving');
    setError(null);
    try {
      const updated = await setLocalMode(next);
      setState(updated);
      setStatus('saved');
      setTimeout(() => setStatus('idle'), 2000);
    } catch (err) {
      console.warn('[local-mode] failed to save:', err);
      setError(err instanceof Error ? err.message : String(err));
      setStatus('error');
    }
  }, []);

  const busy = status === 'saving' || status === 'loading';

  return (
    <SettingsSection title={t('localMode.title')}>
      <div className="p-4">
        <p className="text-xs text-content-muted leading-relaxed">{t('localMode.description')}</p>
      </div>

      <SettingsRow
        htmlFor="switch-local-mode"
        label={t('localMode.enable')}
        description={t('localMode.enableDesc')}
        control={
          <SettingsSwitch
            id="switch-local-mode"
            checked={state?.enabled ?? false}
            disabled={busy || state === null}
            onCheckedChange={checked => void patch({ enabled: checked })}
            data-testid="local-mode-toggle"
          />
        }
      />

      {state?.enabled ? (
        <>
          <SettingsRow
            htmlFor="switch-local-defaults"
            label={t('localMode.applyDefaults')}
            description={t('localMode.applyDefaultsDesc')}
            control={
              <SettingsSwitch
                id="switch-local-defaults"
                checked={state.applyLocalDefaults}
                disabled={busy}
                onCheckedChange={checked => void patch({ apply_local_defaults: checked })}
                data-testid="local-mode-defaults-toggle"
              />
            }
          />
          <SettingsRow
            htmlFor="switch-local-inference-proxy"
            label={t('localMode.proxyInference')}
            description={t('localMode.proxyInferenceDesc')}
            control={
              <SettingsSwitch
                id="switch-local-inference-proxy"
                checked={state.proxyInference}
                disabled={busy}
                onCheckedChange={checked => void patch({ proxy_inference: checked })}
                data-testid="local-mode-proxy-toggle"
              />
            }
          />
        </>
      ) : null}

      {/*
        `enabled` is the persisted setting; `active` is what the core is doing.
        They diverge across an env override and while a restart is pending, and
        a panel that showed only the switch would tell the user a change had
        taken effect when it had not.
      */}
      {state && state.enabled !== state.active ? (
        <div className="mx-4 mb-4 p-3 rounded-lg bg-surface-muted border border-line">
          <p
            className="text-xs text-content-muted leading-relaxed"
            data-testid="local-mode-pending">
            {state.restartRequired ? t('localMode.restartRequired') : t('localMode.overridden')}
          </p>
        </div>
      ) : null}

      {state?.active ? (
        <div className="mx-4 mb-4 p-3 rounded-lg bg-surface-muted border border-line">
          <p className="text-xs text-content-muted" data-testid="local-mode-backend-url">
            {t('localMode.servingAt')} <code>{state.backendUrl}</code>
          </p>
        </div>
      ) : null}

      {state ? (
        <ul data-testid="local-mode-service-list" className="border-t border-line">
          {state.services.entries.map(entry => (
            <ServiceRow key={entry.id} entry={entry} />
          ))}
        </ul>
      ) : null}

      <div className="p-4">
        <SettingsStatusLine
          saving={status === 'saving'}
          savedNote={status === 'saved' ? t('localMode.saved') : null}
          error={status === 'error' ? (error ?? t('localMode.saveError')) : null}
          savingLabel={t('autonomy.statusSaving')}
        />
      </div>
    </SettingsSection>
  );
};

export default LocalModeSection;
