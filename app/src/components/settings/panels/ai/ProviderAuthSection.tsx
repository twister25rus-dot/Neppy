import { LuCircleAlert, LuKeyRound, LuPlus } from 'react-icons/lu';

import { useT } from '../../../../lib/i18n/I18nContext';
import type { ProviderAuthError } from '../../../../services/api/aiSettingsApi';
import Alert from '../../../ui/Alert';
import Button from '../../../ui/Button';
import Card from '../../../ui/Card';
import StatusLine from '../../../ui/StatusLine';
import { routingWithProviderRemoved } from '../aiRouting';
import { BUILTIN_CLOUD_PROVIDER_SLUGS } from '../builtinCloudProviders';
import { ProviderSetupErrorNotice } from '../ProviderSetupErrorNotice';
import {
  type AISettings,
  BUILTIN_PROVIDER_META,
  BUILTIN_RESERVED_SLUGS,
  type CloudProvider,
  formatI18n,
  LOCAL_CHIP_LABEL,
  LOCAL_CHIP_TONE,
  type LocalChipSlug,
} from './aiPanelTypes';
import { ClaudeCodeConnect } from './ClaudeCodeStatusCard';
import { ProviderToggleChip } from './ProviderConnectControls';

const LOCAL_RUNTIME_SLUGS = ['lmstudio', 'ollama', 'omlx'] as const;

/**
 * Provider setup is an at-a-glance surface. Every built-in stays visible, so
 * connecting one takes a single toggle instead of a trip through a picker.
 * Once connected, its label opens the key or endpoint editor.
 */
export const ProviderAuthSection = ({
  draft,
  persist,
  loading,
  error,
  busyAction,
  providerAuthErrors,
  providerSaveNotice,
  onDismissProviderSaveNotice,
  onProviderRemoved,
  codexAuthError,
  onConnectCodex,
  onConnectProvider,
  onOpenKeyDialog,
  onAddCustomProvider,
  onEditCustomProvider,
}: {
  draft: AISettings;
  persist: (next: AISettings) => Promise<void>;
  loading: boolean;
  error: string;
  busyAction: string | null;
  providerAuthErrors: ProviderAuthError[];
  providerSaveNotice: { slug: string; message: string } | null;
  onDismissProviderSaveNotice: () => void;
  onProviderRemoved: (slug: string) => void;
  codexAuthError: string | null;
  onConnectCodex: () => void;
  onConnectProvider: (args: {
    slug: string;
    localLabel?: string | null;
    value: string;
    credentialMode: 'api_key' | 'endpoint' | 'endpoint_key' | 'cli_login' | 'oauth';
  }) => Promise<void>;
  onOpenKeyDialog: (slug: string, localLabel: string | null) => void;
  onAddCustomProvider: () => void;
  onEditCustomProvider: (provider: CloudProvider) => void;
}) => {
  const { t } = useT();

  const bySlug = (slug: string) => draft.cloudProviders.find(cp => cp.slug === slug);
  const customProviders = draft.cloudProviders.filter(
    cp => !BUILTIN_RESERVED_SLUGS.includes(cp.slug)
  );
  const claudeCodeConnected = Boolean(bySlug('claude-code'));

  /** Removing a provider also repairs routes that would otherwise point at it. */
  const removeProvider = async (existing: CloudProvider, isLocalRuntime: boolean) => {
    onProviderRemoved(existing.slug);
    const remaining = draft.cloudProviders.filter(cp => cp.id !== existing.id);
    const nextRouting = routingWithProviderRemoved(
      draft.routing,
      { slug: existing.slug, isLocalRuntime },
      remaining
    );
    await persist({ ...draft, cloudProviders: remaining, routing: nextRouting });
  };

  const toggleBuiltin = (slug: string) => {
    const existing = bySlug(slug);
    if (existing) {
      void removeProvider(existing, false);
      return;
    }
    onOpenKeyDialog(slug, null);
  };

  const toggleLocal = (slug: LocalChipSlug) => {
    const existing = bySlug(slug);
    if (existing) {
      void removeProvider(existing, true);
      return;
    }
    onOpenKeyDialog(slug, LOCAL_CHIP_LABEL[slug]);
  };

  return (
    <div className="flex w-full flex-col gap-4 py-4">
      <div className="flex w-full flex-col gap-4 px-4">
        {providerAuthErrors.length > 0 && (
          <div className="flex w-full flex-col gap-2">
            {providerAuthErrors.map(err => (
              <ProviderSetupErrorNotice key={err.provider} error={err.message} />
            ))}
          </div>
        )}

        {providerSaveNotice && (
          <Alert variant="warning" role="status" className="items-start gap-2 px-3 py-2 text-xs">
            <LuCircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span className="flex-1">{providerSaveNotice.message}</span>
            <Button
              type="button"
              variant="tertiary"
              size="xs"
              className="shrink-0 font-medium normal-case underline-offset-2 hover:underline"
              onClick={onDismissProviderSaveNotice}>
              {t('common.dismiss')}
            </Button>
          </Alert>
        )}

        {loading && <div className="text-xs text-content-muted">{t('common.loading')}</div>}
        {error && <StatusLine saving={false} error={error} savedNote={null} savingLabel="" />}
      </div>

      <Card className="w-full" data-testid="provider-grid-card">
        <div className="px-4 py-4">
          <h2 className="text-lg font-semibold tracking-tight text-content">
            {t('settings.ai.llmProviders')}
          </h2>
          <p className="mt-1 text-sm text-content-muted">{t('settings.ai.llmProvidersDesc')}</p>
        </div>

        <div className="flex flex-wrap gap-2 px-4 py-4" data-testid="provider-chip-grid">
          <ProviderToggleChip
            slug="openhuman"
            label={t('settings.ai.routing.managed')}
            enabled
            alwaysOn
            data-testid="provider-chip-openhuman"
          />

          {BUILTIN_CLOUD_PROVIDER_SLUGS.map(slug => {
            const existing = bySlug(slug);
            const label = BUILTIN_PROVIDER_META[slug]?.label ?? slug;
            return (
              <ProviderToggleChip
                key={slug}
                slug={slug}
                label={label}
                enabled={Boolean(existing)}
                busy={busyAction === `toggle-${slug}`}
                onToggle={() => toggleBuiltin(slug)}
                onLabelClick={existing ? () => onOpenKeyDialog(slug, null) : undefined}
                labelAction={`${t('settings.ai.providers.replaceKey')}: ${label}`}
                data-testid={`provider-chip-${slug}`}
              />
            );
          })}

          {LOCAL_RUNTIME_SLUGS.map(slug => {
            const existing = bySlug(slug);
            const label = LOCAL_CHIP_LABEL[slug];
            return (
              <ProviderToggleChip
                key={slug}
                slug={slug}
                label={label}
                tone={LOCAL_CHIP_TONE[slug]}
                enabled={Boolean(existing)}
                busy={busyAction === `toggle-${slug}`}
                onToggle={() => toggleLocal(slug)}
                onLabelClick={existing ? () => onOpenKeyDialog(slug, label) : undefined}
                labelAction={formatI18n(t('settings.ai.editProviderEndpoint'), { label })}
                data-testid={`provider-chip-${slug}`}
              />
            );
          })}

          {customProviders.map(existing => (
            <ProviderToggleChip
              key={existing.id}
              slug={existing.slug}
              label={existing.label}
              tone={BUILTIN_PROVIDER_META.custom?.tone}
              enabled
              busy={busyAction === `toggle-${existing.slug}`}
              onToggle={() => void removeProvider(existing, false)}
              onLabelClick={() => onEditCustomProvider(existing)}
              labelAction={formatI18n(t('settings.ai.editProvider'), { label: existing.label })}
              data-testid={`provider-chip-${existing.slug}`}
            />
          ))}
        </div>

        <div className="px-4 py-3">
          <p className="text-xs text-content-muted">{t('settings.ai.routing.managedHint')}</p>
        </div>

        <div className="flex flex-col gap-3 px-4 py-4">
          {codexAuthError ? <ProviderSetupErrorNotice error={codexAuthError} /> : null}

          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              variant="secondary"
              size="sm"
              leadingIcon={<LuKeyRound className="h-3.5 w-3.5" />}
              disabled={busyAction === 'toggle-openai'}
              onClick={onConnectCodex}>
              {t('settings.ai.codexAuthButton')}
            </Button>
            <span className="text-xs text-content-muted">{t('settings.ai.codexAuthHelper')}</span>
          </div>

          <ClaudeCodeConnect
            connected={claudeCodeConnected}
            busy={busyAction === 'toggle-claude-code'}
            onConnect={() =>
              onConnectProvider({
                slug: 'claude-code',
                value: 'cli_login',
                credentialMode: 'cli_login',
              })
            }
            onDisconnect={async () => {
              const existing = bySlug('claude-code');
              if (existing) await removeProvider(existing, false);
            }}
          />

          <div className="pt-1">
            <Button
              type="button"
              variant="primary"
              size="sm"
              leadingIcon={<LuPlus className="h-3.5 w-3.5" />}
              onClick={onAddCustomProvider}
              data-testid="add-provider-open">
              {t('settings.ai.routing.addCustomProvider')}
            </Button>
          </div>
        </div>
      </Card>
    </div>
  );
};

export default ProviderAuthSection;
