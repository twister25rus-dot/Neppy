import { Link } from 'react-router-dom';

import { Alert } from '../../components/ui';
import { useT } from '../../lib/i18n/I18nContext';
import type { DebugStatus } from '../../services/api/debugModeApi';
import { dirtyCount, repoBasename, shortSha } from './debugFormat';

/**
 * Persistent top banner of the Debug page: it must be impossible to forget the
 * agent in this chat can edit the app's own source. Amber (warning) semantics
 * from the shared tokens; `role="status"` because it is always present, not an
 * interruption.
 */
export function DebugBanner({ status }: { status: DebugStatus }) {
  const { t } = useT();
  const branch = status.branch ?? t('debug.banner.detached');
  const head = status.head ? shortSha(status.head) : t('debug.banner.noCommits');
  const dirty = dirtyCount(status);

  return (
    <Alert
      variant="warning"
      role="status"
      data-testid="debug-banner"
      className="flex-wrap items-center gap-x-4 gap-y-1 rounded-none border-x-0 border-t-0 py-2">
      <span className="font-semibold tracking-wide">
        {t('debug.banner.title')} • {t('debug.banner.access')}
      </span>
      <dl className="flex flex-wrap items-center gap-x-4 gap-y-0.5 text-xs">
        <BannerFact label={t('debug.banner.repo')} value={repoBasename(status.project_root)} />
        <BannerFact label={t('debug.banner.branch')} value={branch} />
        <BannerFact label={t('debug.banner.head')} value={head} />
        <BannerFact label={t('debug.banner.dirty')} value={String(dirty)} testId="debug-dirty" />
      </dl>
      <Link
        to="/settings/debug-mode"
        data-analytics-id="debug-banner-settings-link"
        className="ml-auto text-xs font-medium underline underline-offset-2">
        {t('settings.debugMode.bannerLink')}
      </Link>
    </Alert>
  );
}

function BannerFact({ label, value, testId }: { label: string; value: string; testId?: string }) {
  return (
    <div className="flex items-center gap-1">
      <dt className="opacity-80">{label}:</dt>
      <dd className="font-mono font-medium" data-testid={testId}>
        {value}
      </dd>
    </div>
  );
}
