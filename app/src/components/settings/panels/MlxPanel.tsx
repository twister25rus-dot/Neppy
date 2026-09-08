import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import Badge, { type BadgeVariant } from '../../ui/Badge';
import Button from '../../ui/Button';

/**
 * The managed MLX runtime surface: one card per `[[mlx.server]]` block.
 *
 * Deliberately compact. A block carries roughly thirty parameters, and a form
 * over all of them would bury the two things this panel exists to answer —
 * is it running, and how much memory is left. Parameters live in config.toml;
 * this shows state and drives the lifecycle.
 *
 * Server flag names (`--kv-bits`, `--max-num-seqs`, …) are rendered verbatim
 * as literal identifiers rather than through i18n. They are command-line
 * arguments of an external binary, not prose: translating them would produce
 * a string that does not exist in any MLX release.
 */

type ServerState = 'stopped' | 'starting' | 'ready' | 'degraded' | 'crashed';

interface MlxServerStatus {
  id: string;
  kind: string;
  state: ServerState;
  port: number | null;
  base_url: string | null;
  pid: number | null;
  loaded_model: string | null;
  resident_gib: number | null;
  estimated_gib: number | null;
  models: string[];
  detail: string | null;
  command: string[];
}

interface MlxStatus {
  enabled: boolean;
  embeddings_backend: string;
  memory_used_gib: number;
  memory_budget_gib: number;
  problems: string[];
  servers: MlxServerStatus[];
}

/** Poll interval while the panel is open. */
const POLL_MS = 5000;

/**
 * Badge tone per state. `starting` is intentionally neutral rather than a
 * warning: loading a 27B checkpoint takes tens of seconds and is not a
 * problem.
 */
const STATE_VARIANT: Record<ServerState, BadgeVariant> = {
  ready: 'success',
  starting: 'neutral',
  stopped: 'neutral',
  degraded: 'warning',
  crashed: 'danger',
};

export default function MlxPanel() {
  const { t } = useT();
  const [status, setStatus] = useState<MlxStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [logs, setLogs] = useState<Record<string, string[]>>({});
  const mounted = useRef(true);

  const load = useCallback(async () => {
    try {
      const resp = await callCoreRpc<MlxStatus>({ method: 'openhuman.mlx_status', params: {} });
      if (!mounted.current) return;
      setStatus(resp);
      setError(null);
    } catch (e) {
      if (!mounted.current) return;
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    void load();
    const timer = setInterval(() => void load(), POLL_MS);
    return () => {
      mounted.current = false;
      clearInterval(timer);
    };
  }, [load]);

  const act = useCallback(
    async (id: string, method: string) => {
      setBusyId(id);
      setError(null);
      try {
        await callCoreRpc({ method, params: { id } });
        await load();
      } catch (e) {
        // A refused start is the common case here, and its message names the
        // shortfall and the next step, so it is worth showing verbatim.
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (mounted.current) setBusyId(null);
      }
    },
    [load]
  );

  const toggleDetails = useCallback(
    async (id: string) => {
      if (expanded === id) {
        setExpanded(null);
        return;
      }
      setExpanded(id);
      try {
        const resp = await callCoreRpc<{ id: string; lines: string[] }>({
          method: 'openhuman.mlx_logs',
          params: { id, limit: 40 },
        });
        if (mounted.current) setLogs(prev => ({ ...prev, [id]: resp.lines }));
      } catch {
        // A missing log tail is not worth an error banner: the card above it
        // already shows why the server is not running.
      }
    },
    [expanded]
  );

  if (!status) {
    return <div className="p-4 text-sm text-content-muted">{t('common.loading')}</div>;
  }

  const memoryPct =
    status.memory_budget_gib > 0
      ? Math.min(100, (status.memory_used_gib / status.memory_budget_gib) * 100)
      : 0;

  return (
    <div className="flex w-full flex-col gap-4">
      <p className="text-sm text-content-muted">{t('mlx.description')}</p>

      {!status.enabled && (
        <div className="rounded-md bg-surface-subtle px-3 py-2 text-sm text-content-muted">
          {t('mlx.disabledNotice')}
        </div>
      )}

      {error && (
        <div className="rounded-md bg-danger-subtle px-3 py-2 text-sm text-danger">{error}</div>
      )}

      {status.problems.length > 0 && (
        <div className="rounded-md bg-warning-subtle px-3 py-2 text-sm">
          <div className="font-medium">{t('mlx.problemsTitle')}</div>
          <ul className="mt-1 list-disc pl-4">
            {status.problems.map(problem => (
              <li key={problem}>{problem}</li>
            ))}
          </ul>
        </div>
      )}

      {/* Memory against the shared budget — the number that decides whether a
          second server can start at all. */}
      <div className="rounded-md border border-border px-3 py-2">
        <div className="flex items-baseline justify-between text-sm">
          <span className="font-medium">{t('mlx.memoryTitle')}</span>
          <span className="text-content-muted">
            {t('mlx.memoryUsage')
              .replace('{used}', status.memory_used_gib.toFixed(1))
              .replace('{budget}', status.memory_budget_gib.toFixed(1))}
          </span>
        </div>
        <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-surface-subtle">
          <div className="h-full bg-primary" style={{ width: `${memoryPct}%` }} />
        </div>
      </div>

      {status.servers.length === 0 && (
        <div className="rounded-md bg-surface-subtle px-3 py-2 text-sm text-content-muted">
          {t('mlx.noServers')}
        </div>
      )}

      {status.servers.map(server => {
        const running = server.state !== 'stopped' && server.state !== 'crashed';
        const busy = busyId === server.id;
        return (
          <div key={server.id} className="rounded-md border border-border p-3">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-medium">{server.id}</span>
              {/* kind is the binary family: literal, not prose. */}
              <span className="text-xs text-content-muted">{server.kind}</span>
              <Badge variant={STATE_VARIANT[server.state]}>{t(`mlx.state.${server.state}`)}</Badge>
              {server.port != null && (
                <span className="text-xs text-content-muted">:{server.port}</span>
              )}
              {server.resident_gib != null && server.resident_gib > 0 && (
                <span className="text-xs text-content-muted">
                  {server.resident_gib.toFixed(1)} GiB
                </span>
              )}
            </div>

            <div className="mt-1 text-sm text-content-muted">
              {server.loaded_model ?? t('mlx.noModelLoaded')}
            </div>

            {server.detail && (
              <div className="mt-1 text-xs text-content-muted">{server.detail}</div>
            )}

            <div className="mt-2 flex flex-wrap gap-2">
              {running ? (
                <>
                  <Button
                    variant="secondary"
                    disabled={busy}
                    analyticsId="mlx-server-stop"
                    onClick={() => void act(server.id, 'openhuman.mlx_stop')}>
                    {t('mlx.stop')}
                  </Button>
                  <Button
                    variant="secondary"
                    disabled={busy}
                    analyticsId="mlx-server-restart"
                    onClick={() => void act(server.id, 'openhuman.mlx_restart')}>
                    {t('mlx.restart')}
                  </Button>
                  <Button
                    variant="secondary"
                    disabled={busy}
                    analyticsId="mlx-server-unload"
                    onClick={() => void act(server.id, 'openhuman.mlx_unload')}>
                    {t('mlx.unload')}
                  </Button>
                </>
              ) : (
                <Button
                  variant="primary"
                  disabled={busy}
                  analyticsId="mlx-server-start"
                  onClick={() => void act(server.id, 'openhuman.mlx_start')}>
                  {t('mlx.start')}
                </Button>
              )}
              <Button
                variant="tertiary"
                analyticsId="mlx-server-details"
                onClick={() => void toggleDetails(server.id)}>
                {expanded === server.id ? t('mlx.hideDetails') : t('mlx.showDetails')}
              </Button>
            </div>

            {expanded === server.id && (
              <div className="mt-3 flex flex-col gap-3">
                {server.command.length > 0 && (
                  <div>
                    <div className="text-xs font-medium">{t('mlx.command')}</div>
                    {/* The bearer token is redacted core-side before it
                        reaches this payload. */}
                    <pre className="mt-1 overflow-x-auto rounded bg-surface-subtle p-2 text-xs">
                      {server.command.join(' ')}
                    </pre>
                  </div>
                )}
                <div>
                  <div className="text-xs font-medium">{t('mlx.logs')}</div>
                  <pre className="mt-1 max-h-48 overflow-auto rounded bg-surface-subtle p-2 text-xs">
                    {(logs[server.id] ?? []).join('\n') || t('mlx.noLogs')}
                  </pre>
                </div>
                {server.models.length > 0 && (
                  <div>
                    <div className="text-xs font-medium">
                      {t('mlx.availableModels').replace('{count}', String(server.models.length))}
                    </div>
                    <ul className="mt-1 max-h-32 overflow-auto text-xs text-content-muted">
                      {server.models.map(model => (
                        <li key={model}>{model}</li>
                      ))}
                    </ul>
                  </div>
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
