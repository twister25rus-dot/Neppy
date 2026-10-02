import debug from 'debug';
import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  type CompanionLevel,
  type CompanionSettingsPatch,
  type CompanionSourceKey,
  type CompanionSources,
  EDITABLE_CATEGORIES,
  type EditableCategory,
  HIGH_RISK_CATEGORIES,
  type PermissionKind,
} from '../../../services/api/petCompanionApi';
import Button from '../../ui/Button';
import Checkbox from '../../ui/Checkbox';
import Field from '../../ui/Field';
import NativeSelect from '../../ui/NativeSelect';
import NumberField from '../../ui/NumberField';
import Switch from '../../ui/Switch';
import { formatHotkey, titleRuleProblem } from '../petFormat';
import CompanionConsentDialog, { COMPANION_SOURCE_KEYS } from './CompanionConsentDialog';
import CompanionDataDialog from './CompanionDataDialog';
import CompanionExclusionEditor from './CompanionExclusionEditor';
import type { UseCompanion } from './useCompanion';

const log = debug('pet:companion:settings');

const LEVELS: CompanionLevel[] = [0, 1, 2, 3];
const CHATTINESS = ['quiet', 'normal', 'eager'] as const;
const MIN_SCREEN_SECS = 10;
const MAX_SCREEN_SECS = 600;
const MIN_RETENTION = 1;
const MAX_RETENTION = 90;

const isMac = (): boolean => typeof navigator !== 'undefined' && /mac/i.test(navigator.platform);

interface CompanionSettingsPanelProps {
  companion: UseCompanion;
  /** Called after data was deleted so the page can refresh. */
  onChanged?: () => void;
}

const clamp = (n: number, lo: number, hi: number) => Math.min(Math.max(n, lo), hi);

/** The permission a source needs, if any. */
const permissionFor = (source: CompanionSourceKey): PermissionKind | null =>
  source === 'app_window' || source === 'selection'
    ? 'accessibility'
    : source === 'screen_capture'
      ? 'screen_recording'
      : null;

/** Settings for the desktop companion: consent, sources, autonomy, exclusions, data. */
export default function CompanionSettingsPanel({
  companion,
  onChanged,
}: CompanionSettingsPanelProps) {
  const { t } = useT();
  const { settings, status } = companion;
  const [consentOpen, setConsentOpen] = useState(false);
  const [consentBusy, setConsentBusy] = useState(false);
  const [consentError, setConsentError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showData, setShowData] = useState(false);
  const [screenSecs, setScreenSecs] = useState(String(settings?.screen_min_interval_secs ?? 30));
  const [retention, setRetention] = useState(String(settings?.retention_days ?? 7));

  // Re-seed the draft number fields when the saved settings change (adjusting
  // state during render, not in an effect).
  const [seededFrom, setSeededFrom] = useState(settings);
  if (settings && settings !== seededFrom) {
    setSeededFrom(settings);
    setScreenSecs(String(settings.screen_min_interval_secs));
    setRetention(String(settings.retention_days));
  }

  if (!settings) {
    return (
      <p className="text-xs text-content-muted" data-testid="companion-settings-unavailable">
        {t('pet.companion.loadFailed')}
      </p>
    );
  }

  const save = async (patch: CompanionSettingsPatch): Promise<boolean> => {
    setError(null);
    try {
      await companion.saveSettings(patch);
      return true;
    } catch (err) {
      log('save failed: %o', err);
      setError(t('pet.companion.errors.saveFailed'));
      return false;
    }
  };

  /** Asks the OS for whatever permission a source needs, only when it is missing. */
  const ensurePermission = async (source: CompanionSourceKey) => {
    const kind = permissionFor(source);
    if (!kind || !status?.platform_supported) return;
    const state = status.permissions[kind];
    if (state === 'granted' || state === 'unsupported') return;
    try {
      await companion.requestPermission(kind);
    } catch (err) {
      log('permission request failed kind=%s err=%o', kind, err);
    }
  };

  const toggleSource = async (source: CompanionSourceKey, on: boolean) => {
    const ok = await save({ sources: { ...settings.sources, [source]: on } });
    if (ok && on) await ensurePermission(source);
  };

  const acceptConsent = async (sources: CompanionSources) => {
    setConsentBusy(true);
    setConsentError(null);
    log('consent accepted sources=%s', COMPANION_SOURCE_KEYS.filter(k => sources[k]).join(','));
    try {
      // Only now, after the user accepted, is the companion turned on.
      await companion.saveSettings({ enabled: true, sources });
      setConsentOpen(false);
      for (const key of COMPANION_SOURCE_KEYS) {
        if (sources[key]) await ensurePermission(key);
      }
    } catch (err) {
      log('enable failed: %o', err);
      setConsentError(t('pet.companion.errors.saveFailed'));
    } finally {
      setConsentBusy(false);
    }
  };

  const mac = isMac();
  const recentApps = Array.from(
    new Set((status?.recent ?? []).map(o => o.bundle_id || o.app_name).filter(Boolean))
  );
  const hotkeys: Array<{ id: 'pause' | 'ask' | 'capture'; labelKey: string }> = [
    { id: 'pause', labelKey: 'pet.companion.hotkey.pause' },
    { id: 'ask', labelKey: 'pet.companion.hotkey.ask' },
    { id: 'capture', labelKey: 'pet.companion.hotkey.capture' },
  ];

  const permissionState = (source: CompanionSourceKey) => {
    const kind = permissionFor(source);
    return kind ? (status?.permissions[kind] ?? 'unknown') : 'granted';
  };

  return (
    <section className="space-y-4" data-testid="companion-settings">
      <div>
        <h3 className="text-sm font-semibold text-content">
          {t('pet.companion.settings.heading')}
        </h3>
        <p className="mt-0.5 text-xs text-content-muted">{t('pet.companion.settings.intro')}</p>
      </div>

      <div className="divide-y divide-line overflow-hidden rounded-2xl border border-line bg-surface shadow-subtle">
        <Field
          htmlFor="companion-enabled"
          label={t('pet.companion.settings.enabled')}
          description={
            status?.platform_supported === false
              ? t('pet.companion.settings.macOnly')
              : t('pet.companion.settings.enabledHint')
          }
          control={
            <Switch
              id="companion-enabled"
              data-testid="companion-enabled-switch"
              checked={settings.enabled}
              onCheckedChange={on => {
                if (on) setConsentOpen(true);
                else void save({ enabled: false });
              }}
            />
          }
        />
        <Field
          htmlFor="companion-cloud"
          label={t('pet.companion.settings.cloud')}
          description={t('pet.companion.settings.cloudHint')}
          control={
            <Switch
              id="companion-cloud"
              data-testid="companion-cloud-switch"
              checked={settings.allow_cloud_model}
              onCheckedChange={on => void save({ allow_cloud_model: on })}
            />
          }
        />
        <Field
          stacked
          label={t('pet.companion.settings.level')}
          description={t('pet.companion.settings.levelHint')}
          control={
            <div
              role="radiogroup"
              className="space-y-2"
              aria-label={t('pet.companion.settings.level')}>
              {LEVELS.map(level => (
                <label key={level} className="flex items-start gap-2">
                  <input
                    type="radio"
                    name="companion-level"
                    data-testid={`companion-level-${level}`}
                    className="mt-1 accent-primary-500"
                    checked={settings.level === level}
                    onChange={() => void save({ level })}
                  />
                  <span>
                    <span className="block text-sm text-content">
                      {t(`pet.companion.level.${level}.title`)}
                    </span>
                    <span className="block text-xs text-content-muted">
                      {t(`pet.companion.level.${level}.desc`)}
                    </span>
                  </span>
                </label>
              ))}
            </div>
          }
        />
        <Field
          stacked
          label={t('pet.companion.settings.sources')}
          control={
            <div className="space-y-3">
              {COMPANION_SOURCE_KEYS.map(key => {
                const perm = permissionFor(key);
                const state = permissionState(key);
                const needsGrant =
                  settings.sources[key] &&
                  perm !== null &&
                  status?.platform_supported &&
                  (state === 'denied' || state === 'unknown');
                return (
                  <div
                    key={key}
                    className="flex items-start gap-3"
                    data-testid={`companion-source-${key}`}>
                    <Checkbox
                      id={`companion-source-${key}`}
                      data-testid={`companion-source-${key}-toggle`}
                      checked={settings.sources[key]}
                      onCheckedChange={on => void toggleSource(key, on)}
                      className="mt-0.5"
                    />
                    <div className="min-w-0 flex-1">
                      <label
                        htmlFor={`companion-source-${key}`}
                        className="block cursor-pointer text-sm text-content">
                        {t(`pet.companion.source.${key}`)}
                      </label>
                      <p className="text-xs text-content-muted">
                        {t(`pet.companion.source.${key}.desc`)}
                      </p>
                      {needsGrant && perm && (
                        <div className="mt-1 flex items-center gap-2 text-xs text-amber-700 dark:text-amber-300">
                          <span>{t('pet.companion.permission.needed')}</span>
                          <Button
                            type="button"
                            variant="secondary"
                            size="xs"
                            analyticsId={`pet-companion-settings-grant-${perm}`}
                            data-testid={`companion-settings-grant-${perm}`}
                            onClick={() => void companion.requestPermission(perm)}>
                            {t('pet.companion.permission.grant')}
                          </Button>
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
              {(settings.unavailable_sources ?? []).map(id => (
                <div
                  key={id}
                  data-testid={`companion-unavailable-${id}`}
                  className="flex items-start gap-3 opacity-60">
                  <Checkbox
                    checked={false}
                    disabled
                    onCheckedChange={() => undefined}
                    aria-label={t(`pet.companion.unavailable.${id}`)}
                    className="mt-0.5"
                  />
                  <div>
                    <p className="text-sm text-content">{t(`pet.companion.unavailable.${id}`)}</p>
                    <p className="text-xs text-content-muted">
                      {t('pet.companion.unavailable.note')}
                    </p>
                  </div>
                </div>
              ))}
            </div>
          }
        />
        <Field
          stacked
          htmlFor="companion-screen-interval"
          label={t('pet.companion.settings.screenInterval')}
          description={t('pet.companion.settings.screenIntervalHint')}
          control={
            <NumberField
              id="companion-screen-interval"
              data-testid="companion-screen-interval"
              aria-label={t('pet.companion.settings.screenInterval')}
              value={screenSecs}
              unit={t('pet.companion.settings.seconds')}
              min={MIN_SCREEN_SECS}
              max={MAX_SCREEN_SECS}
              disabled={!settings.sources.screen_capture}
              onChange={setScreenSecs}
              onCommit={() => {
                const n = Number.parseInt(screenSecs, 10);
                const next = clamp(Number.isFinite(n) ? n : 30, MIN_SCREEN_SECS, MAX_SCREEN_SECS);
                setScreenSecs(String(next));
                if (next !== settings.screen_min_interval_secs) {
                  void save({ screen_min_interval_secs: next });
                }
              }}
            />
          }
        />
        <Field
          stacked
          label={t('pet.companion.settings.categories')}
          description={t('pet.companion.settings.categoriesHint')}
          control={
            <ul className="space-y-2" data-testid="companion-categories">
              {EDITABLE_CATEGORIES.map((cat: EditableCategory) => (
                <li
                  key={cat}
                  data-testid={`companion-category-${cat}`}
                  className="flex items-center justify-between gap-3">
                  <label
                    htmlFor={`companion-category-${cat}-level`}
                    className="text-sm text-content">
                    {t(`pet.companion.category.${cat}`)}
                  </label>
                  <NativeSelect
                    id={`companion-category-${cat}-level`}
                    data-testid={`companion-category-${cat}-level`}
                    inputSize="sm"
                    className="w-36"
                    value={String(settings.category_levels[cat] ?? 1)}
                    onChange={e =>
                      void save({ category_levels: { [cat]: Number(e.target.value) } })
                    }>
                    {LEVELS.map(l => (
                      <option key={l} value={String(l)}>
                        {t(`pet.companion.level.${l}.title`)}
                      </option>
                    ))}
                  </NativeSelect>
                </li>
              ))}
              {HIGH_RISK_CATEGORIES.map(cat => (
                <li
                  key={cat}
                  data-testid={`companion-category-${cat}`}
                  className="flex items-center justify-between gap-3">
                  <label
                    htmlFor={`companion-category-${cat}-level`}
                    className="text-sm text-content">
                    {t(`pet.companion.category.${cat}`)}
                  </label>
                  <NativeSelect
                    id={`companion-category-${cat}-level`}
                    data-testid={`companion-category-${cat}-level`}
                    inputSize="sm"
                    className="w-36"
                    disabled
                    value="ask"
                    onChange={() => undefined}>
                    <option value="ask">{t('pet.companion.category.alwaysAsks')}</option>
                  </NativeSelect>
                </li>
              ))}
            </ul>
          }
        />
        <Field
          stacked
          label={t('pet.companion.exclusions.heading')}
          control={
            <div className="space-y-4">
              <CompanionExclusionEditor
                name="apps"
                label={t('pet.companion.exclusions.apps')}
                hint={t('pet.companion.exclusions.appsHint')}
                placeholder={t('pet.companion.exclusions.appsPlaceholder')}
                items={settings.excluded_apps}
                quickAdd={recentApps}
                quickAddLabel={t('pet.companion.exclusions.recent')}
                onChange={async next => {
                  if (!(await save({ excluded_apps: next }))) throw new Error('save failed');
                }}
              />
              <CompanionExclusionEditor
                name="titles"
                label={t('pet.companion.exclusions.titles')}
                hint={t('pet.companion.exclusions.titlesHint')}
                placeholder={t('pet.companion.exclusions.titlesPlaceholder')}
                items={settings.excluded_title_patterns}
                validate={(entry, existing) => {
                  const p = titleRuleProblem(entry, existing);
                  return p ? `pet.companion.exclusions.problem.${p}` : null;
                }}
                onChange={async next => {
                  if (!(await save({ excluded_title_patterns: next })))
                    throw new Error('save failed');
                }}
              />
            </div>
          }
        />
        <Field
          stacked
          htmlFor="companion-chattiness"
          label={t('pet.companion.settings.chattiness')}
          description={t('pet.companion.settings.chattinessHint')}
          control={
            <NativeSelect
              id="companion-chattiness"
              data-testid="companion-chattiness"
              value={settings.chattiness}
              onChange={e =>
                void save({ chattiness: e.target.value as (typeof CHATTINESS)[number] })
              }>
              {CHATTINESS.map(c => (
                <option key={c} value={c}>
                  {t(`pet.companion.chattiness.${c}`)}
                </option>
              ))}
            </NativeSelect>
          }
        />
        <Field
          stacked
          htmlFor="companion-retention"
          label={t('pet.companion.settings.retention')}
          description={t('pet.companion.settings.retentionHint')}
          control={
            <NumberField
              id="companion-retention"
              data-testid="companion-retention"
              aria-label={t('pet.companion.settings.retention')}
              value={retention}
              unit={t('pet.companion.settings.days')}
              min={MIN_RETENTION}
              max={MAX_RETENTION}
              onChange={setRetention}
              onCommit={() => {
                const n = Number.parseInt(retention, 10);
                const next = clamp(Number.isFinite(n) ? n : 7, MIN_RETENTION, MAX_RETENTION);
                setRetention(String(next));
                if (next !== settings.retention_days) void save({ retention_days: next });
              }}
            />
          }
        />
        <Field
          stacked
          label={t('pet.companion.hotkey.heading')}
          description={t('pet.companion.hotkey.hint')}
          control={
            <ul className="space-y-1" data-testid="companion-hotkeys">
              {hotkeys.map(h => (
                <li
                  key={h.id}
                  className="flex items-center justify-between gap-3 text-sm text-content">
                  <span>{t(h.labelKey)}</span>
                  <kbd
                    data-testid={`companion-hotkey-${h.id}`}
                    className="rounded-md border border-line bg-surface-subtle px-2 py-0.5 font-mono text-xs text-content-secondary">
                    {formatHotkey(settings.hotkeys[h.id], mac) ??
                      t('pet.companion.hotkey.disabled')}
                  </kbd>
                </li>
              ))}
            </ul>
          }
        />
        <Field
          stacked
          label={t('pet.companion.data.heading')}
          description={t('pet.companion.data.hint')}
          control={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              analyticsId="pet-companion-settings-view-data"
              data-testid="companion-settings-view-data"
              onClick={() => setShowData(true)}>
              {t('pet.companion.data.view')}
            </Button>
          }
        />
      </div>

      {error && (
        <p
          role="alert"
          className="text-xs text-coral-600 dark:text-coral-400"
          data-testid="companion-settings-error">
          {error}
        </p>
      )}

      {consentOpen && (
        <CompanionConsentDialog
          busy={consentBusy}
          error={consentError}
          onAccept={sources => void acceptConsent(sources)}
          onCancel={() => {
            setConsentOpen(false);
            setConsentError(null);
          }}
        />
      )}
      {showData && (
        <CompanionDataDialog
          onClose={() => setShowData(false)}
          onChanged={() => {
            void companion.refresh();
            onChanged?.();
          }}
        />
      )}
    </section>
  );
}
