import { useCallback, useEffect, useRef, useState } from 'react';
import LocalPresetSection from './mlx/LocalPresetSection';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import Badge, { type BadgeVariant } from '../../ui/Badge';
import Button from '../../ui/Button';
import MlxServerParams, { type ParamValue } from './mlx/MlxServerParams';
import { inferSlot, occupiedSlot, type Slot, SLOT_LABEL_KEY, SLOTS } from './mlx/slots';

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
  /** The checkpoint the block is configured to launch with. */
  configured_model: string | null;
  /** Every stored field of the block, api_key redacted. */
  settings: Record<string, ParamValue | null>;
}

interface CachedModel {
  id: string;
  size_gib: number;
  looks_like_mlx: boolean;
}

interface CacheListing {
  models: CachedModel[];
  total_gib: number;
  cache_dir: string;
}

interface MlxStatus {
  enabled: boolean;
  chat_provider: string | null;
  chat_uses_mlx: boolean;
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

/**
 * The checkpoint a card is configured to load.
 *
 * `loaded_model` is what the server currently holds, which is not the same
 * thing: with `model_discovery = "hf-cache"` a server can be configured for one
 * checkpoint and holding none, or holding one a request asked for. The select
 * reflects configuration, so it falls back to whatever is resident only when
 * nothing is configured.
 */
function serverModelOf(server: MlxServerStatus, cache: CacheListing | null): string {
  if (server.loaded_model && cache?.models?.some(model => model.id === server.loaded_model)) {
    return server.loaded_model;
  }
  return server.configured_model ?? '';
}

export default function MlxPanel() {
  const { t } = useT();
  const [status, setStatus] = useState<MlxStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [logs, setLogs] = useState<Record<string, string[]>>({});
  const [cache, setCache] = useState<CacheListing | null>(null);
  // Deleting a multi-gigabyte download is worth a second click.
  const [confirmingDelete, setConfirmingDelete] = useState<string | null>(null);
  const mounted = useRef(true);

  const load = useCallback(async () => {
    try {
      const [statusResp, cacheResp] = await Promise.all([
        callCoreRpc<MlxStatus>({ method: 'openhuman.mlx_status', params: {} }),
        callCoreRpc<CacheListing>({ method: 'openhuman.mlx_models_list', params: {} }),
      ]);
      if (!mounted.current) return;
      setStatus(statusResp);
      setCache(cacheResp);
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

  const deleteModel = useCallback(
    async (modelId: string) => {
      setError(null);
      try {
        await callCoreRpc({ method: 'openhuman.mlx_models_delete', params: { model_id: modelId } });
        setConfirmingDelete(null);
        await load();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [load]
  );

  /**
   * Change any subset of a server's settings.
   *
   * The core restarts the server only when a changed field is part of its
   * command line, and skips the save entirely when nothing actually differs.
   */
  const updateServer = useCallback(
    async (id: string, patch: Record<string, ParamValue>) => {
      setBusyId(id);
      setError(null);
      try {
        await callCoreRpc({ method: 'openhuman.mlx_update_server', params: { id, patch } });
        await load();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (mounted.current) setBusyId(null);
      }
    },
    [load]
  );

  /**
   * Tick or untick a model.
   *
   * Ticking puts it in a slot; unticking clears whichever slot it held. A slot
   * holds one checkpoint, so ticking a second model for the same slot replaces
   * the first rather than erroring — that is what the tick means.
   */
  const toggleModel = useCallback(
    (server: MlxServerStatus, modelId: string, slot: Slot) => {
      const held = occupiedSlot(modelId, server.settings as Record<Slot, string | null>);
      void updateServer(server.id, { [held ?? slot]: held ? '' : modelId });
    },
    [updateServer]
  );

  const setEmbeddingsBackend = useCallback(
    async (backend: string) => {
      setError(null);
      try {
        await callCoreRpc({ method: 'openhuman.mlx_set_embeddings_backend', params: { backend } });
        await load();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
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
      <LocalPresetSection />

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

      {/* Which backend serves embeddings. Ollama stays the default and is
          never removed: this is a choice, not a migration. */}
      <label className="flex items-center gap-2 rounded-md border border-border px-3 py-2 text-sm">
        <span className="font-medium">{t('mlx.embeddingsLabel')}</span>
        <select
          className="rounded-md border border-border bg-surface px-2 py-1 text-sm"
          value={status.embeddings_backend}
          onChange={event => void setEmbeddingsBackend(event.target.value)}>
          <option value="ollama">{t('mlx.embeddingsOllama')}</option>
          <option value="mlx">{t('mlx.embeddingsMlx')}</option>
        </select>
        <span className="text-xs text-content-muted">{t('mlx.embeddingsHint')}</span>
      </label>

      {status.servers.length === 0 && (
        <div className="rounded-md bg-surface-subtle px-3 py-2 text-sm text-content-muted">
          {t('mlx.noServers')}
        </div>
      )}

      {status.servers.map(server => {
        const running = server.state !== 'stopped' && server.state !== 'crashed';
        const busy = busyId === server.id;
        // Chat names a model, not a server, so a card is "the chat server"
        // when the provider string names the checkpoint it is configured to
        // load.
        const configuredModel = serverModelOf(server, cache);
        const usesThisServer =
          status.chat_uses_mlx &&
          configuredModel !== '' &&
          status.chat_provider === `mlx:${configuredModel}`;
        return (
          <div key={server.id} className="rounded-md border border-border p-3">
            <div className="flex flex-wrap items-center gap-2">
              <span className="font-medium">{server.id}</span>
              {/* kind is the binary family: literal, not prose. */}
              <span className="text-xs text-content-muted">{server.kind}</span>
              <Badge variant={STATE_VARIANT[server.state]}>{t(`mlx.state.${server.state}`)}</Badge>
              {usesThisServer && <Badge variant="primary">{t('mlx.servingChat')}</Badge>}
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

            {/* Tick the models this server should load. One process serves
                six slots, so several can be held at once — a chat model, an
                embedder and a speech model together. */}
            <div className="mt-2">
              <div className="flex items-baseline justify-between">
                <span className="text-xs font-medium">{t('mlx.modelsLabel')}</span>
                {/* Guarded on the number, not on `cache` being truthy: a
                    payload that arrives without this field would otherwise
                    throw here and blank the whole panel. */}
                {typeof cache?.total_gib === 'number' && (
                  <span className="text-xs text-content-muted">
                    {t('mlx.cacheTotal').replace('{total}', cache.total_gib.toFixed(1))}
                  </span>
                )}
              </div>
              <p className="text-xs text-content-muted">{t('mlx.modelsHint')}</p>
              <ul className="mt-1 flex flex-col gap-1">
                {(cache?.models ?? []).map(model => {
                  const held = occupiedSlot(
                    model.id,
                    server.settings as Record<Slot, string | null>
                  );
                  const suggested = inferSlot(model.id);
                  const id = `mlx-${server.id}-${model.id}`;
                  return (
                    <li key={model.id} className="flex items-center gap-2 text-sm">
                      <input
                        id={id}
                        type="checkbox"
                        disabled={busy}
                        checked={held !== null}
                        onChange={() => toggleModel(server, model.id, suggested)}
                      />
                      <label htmlFor={id} className="min-w-0 flex-1 truncate" title={model.id}>
                        {model.id}
                      </label>
                      <span className="shrink-0 text-xs text-content-muted">
                        {model.size_gib.toFixed(1)} GiB
                      </span>
                      {/* The slot it occupies, or the one ticking would use.
                          Shown rather than inferred silently, and changeable. */}
                      <select
                        aria-label={t('mlx.slotLabel')}
                        className="shrink-0 rounded-md border border-border bg-surface px-1 py-0.5 text-xs"
                        disabled={busy}
                        value={held ?? suggested}
                        onChange={event => {
                          const next = event.target.value as Slot;
                          if (held === next) return;
                          void updateServer(server.id, {
                            ...(held ? { [held]: '' } : {}),
                            [next]: model.id,
                          });
                        }}>
                        {SLOTS.map(slot => (
                          <option key={slot} value={slot}>
                            {t(SLOT_LABEL_KEY[slot])}
                          </option>
                        ))}
                      </select>
                      {/* Deleting the download lives on the same row as the
                          tick: one list, one place per model. */}
                      {confirmingDelete === model.id ? (
                        <Button
                          variant="secondary"
                          analyticsId="mlx-model-delete-confirm"
                          onClick={() => void deleteModel(model.id)}>
                          {t('mlx.confirmDelete')}
                        </Button>
                      ) : (
                        <Button
                          variant="tertiary"
                          analyticsId="mlx-model-delete"
                          onClick={() => setConfirmingDelete(model.id)}>
                          {t('mlx.delete')}
                        </Button>
                      )}
                    </li>
                  );
                })}
              </ul>
              {(cache?.models ?? []).length === 0 && (
                <p className="text-xs text-content-muted">{t('mlx.noCachedModels')}</p>
              )}
              <p className="mt-1 text-xs text-content-muted">{t('mlx.deleteHint')}</p>
            </div>

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
                <MlxServerParams
                  values={server.settings}
                  isVlm={server.kind !== 'lm'}
                  disabled={busy}
                  onChange={patch => void updateServer(server.id, patch)}
                />
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
