import { useState } from 'react';

import { Badge, Button, ConfirmDialog } from '../../components/ui';
import { useT } from '../../lib/i18n/I18nContext';
import { useLocalInstall } from './useLocalInstall';

/**
 * "Build & install locally": builds the current source into a Neppy.app and,
 * after an explicit confirmation, swaps it into /Applications and restarts.
 * Nothing is uploaded. The previous app is backed up and restored by the
 * installer helper if the new one does not start; when that happened on the
 * last launch, a one-time notice says so.
 */
export function LocalInstallCard() {
  const { t } = useT();
  const { status, result, error, busy, build, install, acknowledgeResult } = useLocalInstall();
  const [confirming, setConfirming] = useState(false);

  const phase = status?.phase ?? 'idle';
  const canBuild = !busy && phase !== 'building' && phase !== 'installing';
  const showRestored = result?.status === 'restored' && !result.seen;
  const errorKey = error
    ? (
        {
          start: 'debug.localInstall.startFailed',
          install: 'debug.localInstall.installFailed',
          quit: 'debug.localInstall.quitFailed',
        } as const
      )[error.kind]
    : null;

  return (
    <section
      aria-label={t('debug.localInstall.title')}
      data-testid="debug-local-install"
      data-phase={phase}
      className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line bg-surface px-4 py-2 text-sm">
      <span className="text-xs font-semibold uppercase tracking-wide text-content-muted">
        {t('debug.localInstall.title')}
      </span>
      {phase === 'building' ? (
        <span
          className="text-xs text-content-secondary"
          role="status"
          data-testid="debug-local-building">
          {t('debug.localInstall.building')}
        </span>
      ) : null}
      {phase === 'ready' ? (
        <>
          <span className="text-xs text-content-secondary" data-testid="debug-local-ready">
            {t('debug.localInstall.ready')}
          </span>
          {status?.version ? (
            <Badge variant="neutral">
              {t('debug.localInstall.version').replace('{version}', status.version)}
            </Badge>
          ) : null}
        </>
      ) : null}
      {phase === 'installing' ? (
        <span
          className="text-xs text-content-secondary"
          role="status"
          data-testid="debug-local-installing">
          {t('debug.localInstall.installing')}
        </span>
      ) : null}
      {phase === 'failed' ? (
        <span className="text-xs text-coral" data-testid="debug-local-failed">
          {t('debug.localInstall.failed')}
        </span>
      ) : null}
      <div className="ml-auto flex items-center gap-2">
        {phase === 'ready' ? (
          <Button
            size="xs"
            analyticsId="debug-local-install-apply"
            data-testid="debug-local-apply"
            disabled={busy}
            onClick={() => setConfirming(true)}>
            {t('debug.localInstall.install')}
          </Button>
        ) : null}
        <Button
          size="xs"
          variant={phase === 'ready' ? 'secondary' : 'primary'}
          analyticsId="debug-local-install-build"
          data-testid="debug-local-build"
          disabled={!canBuild}
          onClick={() => void build()}>
          {phase === 'failed' || phase === 'ready'
            ? t('debug.localInstall.rebuild')
            : t('debug.localInstall.build')}
        </Button>
      </div>
      {(phase === 'building' || phase === 'failed') && (status?.log_tail || status?.error) ? (
        <pre
          aria-label={t('debug.localInstall.logLabel')}
          data-testid="debug-local-log"
          className="max-h-24 w-full overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 font-mono text-[11px] text-content-secondary">
          {phase === 'failed' ? status?.error || status?.log_tail : status?.log_tail}
        </pre>
      ) : null}
      {error && errorKey ? (
        <p role="alert" className="w-full text-xs text-coral" data-testid="debug-local-error">
          {t(errorKey).replace('{error}', error.message)}
        </p>
      ) : null}
      {showRestored ? (
        <p
          role="status"
          className="flex w-full items-center gap-2 text-xs text-amber-700 dark:text-amber-300"
          data-testid="debug-local-restored">
          <span>{t('debug.localInstall.restored')}</span>
          <Button
            size="xs"
            variant="secondary"
            analyticsId="debug-local-install-dismiss"
            data-testid="debug-local-restored-dismiss"
            onClick={() => void acknowledgeResult()}>
            {t('debug.localInstall.dismiss')}
          </Button>
        </p>
      ) : null}
      {confirming ? (
        <ConfirmDialog
          title={t('debug.localInstall.confirmTitle')}
          titleId="debug-local-install-title"
          confirmLabel={t('debug.localInstall.install')}
          busy={busy}
          onCancel={() => {
            if (!busy) setConfirming(false);
          }}
          onConfirm={() => {
            setConfirming(false);
            void install();
          }}
          body={<p data-testid="debug-local-confirm-body">{t('debug.localInstall.confirmBody')}</p>}
        />
      ) : null}
    </section>
  );
}
