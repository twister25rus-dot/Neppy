import { useCallback, useMemo, useState } from 'react';

import { Badge, CenteredLoadingState, EmptyState, ErrorBanner } from '../../../components/ui';
import { useT } from '../../../lib/i18n/I18nContext';
import { type DebugDiff, getDebugDiff } from '../../../services/api/debugModeApi';
import { errorText } from '../debugFormat';
import { type DiffLineKind, type ParsedDiffFile, parseUnifiedDiff } from './parseUnifiedDiff';
import { useAsyncData } from './useAsyncData';

const LINE_CLASS: Record<DiffLineKind, string> = {
  add: 'bg-sage-500/10 text-sage-700 dark:text-sage-300',
  del: 'bg-coral-500/10 text-coral-600 dark:text-coral-300',
  context: 'text-content-secondary',
  hunk: 'bg-primary-500/10 text-primary-600 dark:text-primary-300',
  note: 'italic text-content-faint',
};

const LINE_MARKER: Record<DiffLineKind, string> = {
  add: '+',
  del: '-',
  context: ' ',
  hunk: '',
  note: '',
};

function FileSection({
  file,
  added,
  removed,
}: {
  file: ParsedDiffFile;
  added: number | null;
  removed: number | null;
}) {
  const { t } = useT();
  const [open, setOpen] = useState(true);
  const bodyId = `debug-diff-file-${file.path.replace(/[^a-zA-Z0-9_-]/g, '_')}`;
  return (
    <section
      className="overflow-hidden rounded-lg border border-line bg-surface"
      data-testid="debug-diff-file">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={bodyId}
        data-analytics-id="debug-diff-toggle-file"
        onClick={() => setOpen(o => !o)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs hover:bg-surface-hover focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-500/25">
        <span aria-hidden="true" className="text-content-muted">
          {open ? '▾' : '▸'}
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-content">{file.path}</span>
        {file.status !== 'modified' ? (
          <Badge variant={file.status === 'deleted' ? 'danger' : 'primary'}>
            {file.status === 'created'
              ? t('debug.panels.diff.status.created')
              : file.status === 'deleted'
                ? t('debug.panels.diff.status.deleted')
                : t('debug.panels.diff.status.renamed')}
          </Badge>
        ) : null}
        {file.binary ? (
          <Badge>{t('debug.panels.diff.binary')}</Badge>
        ) : (
          <span className="shrink-0 font-mono">
            <span className="text-sage-700 dark:text-sage-300">+{added ?? file.added}</span>{' '}
            <span className="text-coral-600 dark:text-coral-300">-{removed ?? file.removed}</span>
          </span>
        )}
      </button>
      {open ? (
        <div id={bodyId} className="overflow-x-auto border-t border-line">
          {file.binary ? (
            <p className="px-3 py-2 text-xs italic text-content-faint">
              {t('debug.panels.diff.binaryNote')}
            </p>
          ) : (
            <pre className="min-w-full font-mono text-xs leading-5">
              {file.lines.map((line, i) => (
                <div
                  key={i}
                  data-line-kind={line.kind}
                  className={`whitespace-pre px-3 ${LINE_CLASS[line.kind]}`}>
                  {LINE_MARKER[line.kind]}
                  {line.text || ' '}
                </div>
              ))}
            </pre>
          )}
        </div>
      ) : null}
    </section>
  );
}

function DiffBody({ diff }: { diff: DebugDiff }) {
  const { t } = useT();
  const parsed = useMemo(() => parseUnifiedDiff(diff.text), [diff.text]);
  const counts = useMemo(() => new Map(diff.files.map(f => [f.path, f])), [diff.files]);
  const { modified, created, deleted } = diff.summary;
  const empty = parsed.length === 0 && diff.untracked.length === 0;

  return (
    <div className="space-y-3">
      <p className="text-xs font-medium text-content" data-testid="debug-diff-summary">
        {t('debug.panels.diff.summary')
          .replace('{modified}', String(modified))
          .replace('{created}', String(created))
          .replace('{deleted}', String(deleted))}
      </p>
      {empty ? <EmptyState label={t('debug.panels.diff.empty')} /> : null}
      {parsed.map(file => {
        const c = counts.get(file.path);
        return (
          <FileSection
            key={`${file.oldPath ?? ''}>${file.path}`}
            file={file}
            added={c ? c.added : null}
            removed={c ? c.deleted : null}
          />
        );
      })}
      {diff.truncated ? (
        <p
          role="status"
          className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300"
          data-testid="debug-diff-truncated">
          {t('debug.panels.diff.truncated')}
        </p>
      ) : null}
      {diff.untracked.length > 0 ? (
        <div data-testid="debug-diff-untracked">
          <h4 className="mb-1 text-xs font-medium text-content-secondary">
            {t('debug.panels.diff.untracked').replace('{count}', String(diff.untracked.length))}
          </h4>
          <ul className="space-y-0.5">
            {diff.untracked.map(path => (
              <li key={path} className="truncate font-mono text-xs text-content-muted">
                {path}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}

/** Working tree vs HEAD, or vs the given checkpoint. `refreshKey` forces a re-fetch. */
export function DiffViewer({
  checkpointId,
  refreshKey = 0,
}: {
  checkpointId?: string;
  refreshKey?: number;
}) {
  const { t } = useT();
  const load = useCallback(() => {
    void refreshKey;
    return getDebugDiff(checkpointId);
  }, [checkpointId, refreshKey]);
  const { data, loading, error, reload } = useAsyncData(load);

  if (loading) return <CenteredLoadingState label={t('debug.panels.loading')} />;
  if (error) {
    return (
      <ErrorBanner
        action={
          <button
            type="button"
            className="text-xs underline"
            data-analytics-id="debug-diff-retry"
            onClick={reload}>
            {t('common.retry')}
          </button>
        }>
        {t('debug.panels.diff.error').replace(
          '{error}',
          errorText(error) || t('debug.panels.unknownError')
        )}
      </ErrorBanner>
    );
  }
  if (!data) return null;
  return <DiffBody diff={data} />;
}

export default DiffViewer;
