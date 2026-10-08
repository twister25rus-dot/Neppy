import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { Badge, Button, Input, ModalShell, Spinner } from '../../components/ui';
import { useT } from '../../lib/i18n/I18nContext';
import type { ReleaseBlocker, ReleasePreflight } from '../../services/api/debugModeApi';
import { openUrl } from '../../utils/openUrl';
import { useRelease, validateReleaseVersion } from './useRelease';

const log = debug('neppy:debug:release-card');

const BLOCKER_KEYS = {
  not_release_branch: 'debug.release.blocker.not_release_branch',
  dirty: 'debug.release.blocker.dirty',
  behind: 'debug.release.blocker.behind',
  no_signing_key: 'debug.release.blocker.no_signing_key',
  gh_not_ready: 'debug.release.blocker.gh_not_ready',
  release_running: 'debug.release.blocker.release_running',
  nothing_to_release: 'debug.release.blocker.nothing_to_release',
} as const satisfies Record<ReleaseBlocker, string>;

/** `m:ss`, or `h:mm:ss` past the first hour. */
export function formatElapsed(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${sec}` : `${m}:${sec}`;
}

function useElapsedSeconds(startedAt: string | null | undefined, active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return undefined;
    const id = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(id);
  }, [active]);
  const started = startedAt ? Date.parse(startedAt) : Number.NaN;
  return Number.isNaN(started) ? 0 : Math.max(0, (now - started) / 1000);
}

function LogTail({ text, label, testId }: { text: string; label: string; testId: string }) {
  const ref = useRef<HTMLPreElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [text]);
  return (
    <pre
      ref={ref}
      aria-label={label}
      data-testid={testId}
      className="max-h-40 w-full overflow-auto whitespace-pre-wrap rounded-md bg-surface-subtle p-2 font-mono text-[11px] text-content-secondary">
      {text}
    </pre>
  );
}

interface ConfirmProps {
  preflight: ReleasePreflight;
  onConfirm: (version: string) => void;
  onCancel: () => void;
}

/** Asks for the exact version, validated, before anything is published. */
function ReleaseConfirmDialog({ preflight, onConfirm, onCancel }: ConfirmProps) {
  const { t } = useT();
  const [version, setVersion] = useState(preflight.suggested_version);
  const trimmed = version.trim();
  const problem = validateReleaseVersion(trimmed, preflight.current_version);
  const problemText =
    problem === 'format'
      ? t('debug.release.versionInvalid')
      : problem === 'notGreater'
        ? t('debug.release.versionNotGreater').replace('{current}', preflight.current_version)
        : null;

  return (
    <ModalShell
      title={t('debug.release.confirmTitle')}
      titleId="debug-release-title"
      onClose={onCancel}
      maxWidthClassName="max-w-md"
      footer={
        <div className="flex justify-end gap-2">
          <Button
            variant="secondary"
            size="sm"
            analyticsId="debug-release-cancel"
            data-testid="debug-release-cancel"
            onClick={onCancel}>
            {t('common.cancel')}
          </Button>
          <Button
            size="sm"
            analyticsId="debug-release-confirm"
            data-testid="debug-release-confirm"
            disabled={problem !== null}
            onClick={() => onConfirm(trimmed)}>
            {t('debug.release.confirm').replace('{version}', trimmed)}
          </Button>
        </div>
      }>
      <div className="space-y-3 text-sm text-content-secondary">
        <p data-testid="debug-release-confirm-body">{t('debug.release.confirmBody')}</p>
        <label className="block space-y-1">
          <span className="text-xs font-medium text-content">
            {t('debug.release.versionLabel')}
          </span>
          <Input
            monospace
            value={version}
            invalid={problem !== null}
            onChange={e => setVersion(e.target.value)}
            data-testid="debug-release-version"
          />
        </label>
        {problemText ? (
          <p role="alert" className="text-xs text-coral" data-testid="debug-release-version-error">
            {problemText}
          </p>
        ) : null}
      </div>
    </ModalShell>
  );
}

/**
 * "Publish release": after a Debug task is committed, runs the repository's own
 * `scripts/release-neppy.sh` on this machine (build, sign, push, tag, GitHub
 * release) and follows it live. Starting always goes through a confirm dialog
 * that shows the exact version. This is UI-only; no agent tool can reach it.
 */
export function ReleaseCard() {
  const { t } = useT();
  const { preflight, record, error, busy, dismissed, start, reset } = useRelease();
  const [confirming, setConfirming] = useState(false);

  const phase = record?.phase ?? 'idle';
  const running = phase === 'running';
  const elapsed = useElapsedSeconds(record?.started_at, running);
  const showResult = (phase === 'succeeded' || phase === 'failed') && !dismissed;
  const view = running ? 'running' : showResult ? phase : 'idle';
  const blockers = preflight?.blockers ?? [];
  const canPublish = Boolean(preflight) && blockers.length === 0 && !busy && !running;
  const version = record?.version ?? '';
  const errorKey = error
    ? error.kind === 'start'
      ? 'debug.release.startFailed'
      : 'debug.release.loadFailed'
    : null;

  return (
    <section
      aria-label={t('debug.release.title')}
      data-testid="debug-release"
      data-phase={view}
      className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line bg-surface px-4 py-2 text-sm">
      <span className="text-xs font-semibold uppercase tracking-wide text-content-muted">
        {t('debug.release.title')}
      </span>

      {view === 'idle' && preflight ? (
        <>
          <Badge variant="neutral" data-testid="debug-release-current">
            {t('debug.release.currentVersion').replace('{version}', preflight.current_version)}
          </Badge>
          {preflight.last_tag ? (
            <span className="text-xs text-content-secondary" data-testid="debug-release-last-tag">
              {t('debug.release.lastTag').replace('{tag}', preflight.last_tag)}
            </span>
          ) : null}
          <div className="ml-auto">
            <Button
              size="xs"
              analyticsId="debug-release-publish"
              data-testid="debug-release-publish"
              disabled={!canPublish}
              onClick={() => {
                log('confirm dialog opened');
                setConfirming(true);
              }}>
              {t('debug.release.publish')}
            </Button>
          </div>
          {blockers.length > 0 ? (
            <ul
              className="w-full list-disc space-y-0.5 pl-5 text-xs text-content-secondary"
              data-testid="debug-release-blockers">
              {blockers.map(code => (
                <li key={code} data-blocker={code}>
                  {(BLOCKER_KEYS as Record<string, string | undefined>)[code]
                    ? t(BLOCKER_KEYS[code]).replace('{branch}', preflight.release_branch)
                    : code}
                </li>
              ))}
            </ul>
          ) : null}
        </>
      ) : null}

      {view === 'running' ? (
        <>
          <span
            className="flex items-center gap-2 text-xs text-content-secondary"
            role="status"
            data-testid="debug-release-running">
            <Spinner className="h-3.5 w-3.5" />
            {t('debug.release.running').replace('{version}', version)}
          </span>
          <span
            className="ml-auto text-xs tabular-nums text-content-muted"
            data-testid="debug-release-elapsed">
            {t('debug.release.elapsed').replace('{time}', formatElapsed(elapsed))}
          </span>
          {record?.log_tail ? (
            <LogTail
              text={record.log_tail}
              label={t('debug.release.logLabel')}
              testId="debug-release-log"
            />
          ) : null}
        </>
      ) : null}

      {view === 'succeeded' ? (
        <>
          <span
            className="text-xs font-medium text-sage-700 dark:text-sage-300"
            data-testid="debug-release-succeeded">
            {t('debug.release.succeeded').replace('{version}', version)}
          </span>
          {record?.tag ? <Badge variant="neutral">{record.tag}</Badge> : null}
          {record?.release_url ? (
            <a
              href={record.release_url}
              data-analytics-id="debug-release-open"
              data-testid="debug-release-link"
              className="text-xs font-medium text-primary-600 underline underline-offset-2"
              onClick={e => {
                e.preventDefault();
                void openUrl(record.release_url as string).catch(() => log('open url failed'));
              }}>
              {t('debug.release.viewRelease')}
            </a>
          ) : null}
          <div className="ml-auto">
            <Button
              size="xs"
              variant="secondary"
              analyticsId="debug-release-dismiss"
              data-testid="debug-release-dismiss"
              onClick={() => void reset()}>
              {t('debug.release.dismiss')}
            </Button>
          </div>
        </>
      ) : null}

      {view === 'failed' ? (
        <>
          <span className="text-xs text-coral" role="alert" data-testid="debug-release-failed">
            {t('debug.release.failed')}
          </span>
          <div className="ml-auto">
            <Button
              size="xs"
              analyticsId="debug-release-retry"
              data-testid="debug-release-retry"
              onClick={() => void reset()}>
              {t('debug.release.retry')}
            </Button>
          </div>
          {record?.error ? (
            <p className="w-full text-xs text-coral" data-testid="debug-release-failed-reason">
              {record.error}
            </p>
          ) : null}
          {record?.log_tail ? (
            <LogTail
              text={record.log_tail}
              label={t('debug.release.logLabel')}
              testId="debug-release-log"
            />
          ) : null}
        </>
      ) : null}

      {error && errorKey ? (
        <p role="alert" className="w-full text-xs text-coral" data-testid="debug-release-error">
          {t(errorKey).replace('{error}', error.message)}
        </p>
      ) : null}

      {confirming && preflight ? (
        <ReleaseConfirmDialog
          preflight={preflight}
          onCancel={() => setConfirming(false)}
          onConfirm={chosen => {
            log('confirmed version=%s', chosen);
            setConfirming(false);
            void start(chosen);
          }}
        />
      ) : null}
    </section>
  );
}

export default ReleaseCard;
