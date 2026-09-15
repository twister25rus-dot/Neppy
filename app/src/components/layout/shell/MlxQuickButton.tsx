import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import { getLocalModelPreset, setLocalModelPreset } from '../../../services/api/localPresetApi';
import { callCoreRpc } from '../../../services/coreRpcClient';
import ChatPresetPill, { type PresetId } from '../../chat/ChatPresetPill';
import { inferSlot, occupiedSlot, type Slot } from '../../settings/panels/mlx/slots';
import {
  Badge,
  type BadgeVariant,
  Button,
  PopoverContent,
  PopoverRoot,
  PopoverTrigger,
  Tooltip,
} from '../../ui';

/**
 * MLX, one click from any thread.
 *
 * This replaced a settings gear whose menu was five links to settings pages.
 * The local runtime is the thing actually worth reaching mid-task — whether it
 * is up, which checkpoint it holds, and how hard it should work all change
 * while you are using it, and all three previously meant leaving the thread.
 * The settings the gear offered are still here, at the bottom, because
 * removing the only always-visible route to them would be a regression rather
 * than a simplification.
 *
 * The trigger carries the state as a dot so the common question ("is it
 * running?") is answered without opening anything.
 */

type ServerState = 'stopped' | 'starting' | 'ready' | 'degraded' | 'crashed';

interface MlxServerStatus {
  id: string;
  state: ServerState;
  loaded_model: string | null;
  /** Every stored field of the `[[mlx.server]]` block, api_key redacted. */
  settings: Record<string, string | number | boolean | null>;
}

interface MlxStatus {
  enabled: boolean;
  chat_uses_mlx: boolean;
  memory_used_gib: number;
  memory_budget_gib: number;
  servers: MlxServerStatus[];
}

interface CachedModel {
  id: string;
  size_gib: number;
}

/** Matches `MlxPanel`, so one runtime never reads as two different states. */
const STATE_VARIANT: Record<ServerState, BadgeVariant> = {
  ready: 'success',
  starting: 'neutral',
  stopped: 'neutral',
  degraded: 'warning',
  crashed: 'danger',
};

const STATE_DOT: Record<ServerState, string> = {
  ready: 'bg-success',
  starting: 'bg-content-faint animate-pulse',
  stopped: 'bg-content-faint',
  degraded: 'bg-warning',
  crashed: 'bg-danger',
};

const STATE_LABEL_KEY: Record<ServerState, string> = {
  ready: 'mlx.state.ready',
  starting: 'mlx.state.starting',
  stopped: 'mlx.state.stopped',
  degraded: 'mlx.state.degraded',
  crashed: 'mlx.state.crashed',
};

/** Polled only while the menu is open; the runtime is not worth a background timer. */
const POLL_MS = 4000;

/** The one state that speaks for the whole runtime, worst first. */
function aggregateState(servers: readonly MlxServerStatus[]): ServerState {
  for (const state of ['crashed', 'degraded', 'starting', 'ready'] as const) {
    if (servers.some(server => server.state === state)) return state;
  }
  return 'stopped';
}

/** A repo id is long and its tail is the identifying half. */
function shortModelId(modelId: string): string {
  const tail = modelId.split('/').pop() ?? modelId;
  return tail.length > 34 ? `${tail.slice(0, 33)}…` : tail;
}

export default function MlxQuickButton() {
  const { t } = useT();
  const navigate = useNavigate();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<MlxStatus | null>(null);
  const [cache, setCache] = useState<CachedModel[]>([]);
  const [preset, setPreset] = useState<PresetId>('auto');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);

  const load = useCallback(async () => {
    try {
      const [statusResp, cacheResp] = await Promise.all([
        callCoreRpc<MlxStatus>({ method: 'openhuman.mlx_status', params: {} }),
        callCoreRpc<{ models: CachedModel[] }>({ method: 'openhuman.mlx_models_list', params: {} }),
      ]);
      if (!mounted.current) return;
      setStatus(statusResp);
      setCache(cacheResp?.models ?? []);
      setError(null);
    } catch (e) {
      if (!mounted.current) return;
      // A runtime that cannot be read is worth saying so: the alternative is a
      // menu that renders "Stopped" for a core that never answered.
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  // One read on mount for the trigger's dot, then a poll only while open.
  useEffect(() => {
    mounted.current = true;
    void load();
    return () => {
      mounted.current = false;
    };
  }, [load]);

  useEffect(() => {
    if (!open) return;
    void load();
    void getLocalModelPreset()
      .then(value => {
        if (mounted.current) setPreset(value);
      })
      .catch(() => {
        // Unreadable means "show the default", not "block the menu".
      });
    const timer = setInterval(() => void load(), POLL_MS);
    return () => clearInterval(timer);
  }, [open, load]);

  const act = useCallback(
    async (method: string, params: Record<string, unknown>) => {
      setBusy(true);
      setError(null);
      try {
        await callCoreRpc({ method, params });
        await load();
      } catch (e) {
        // A refused start names the shortfall and the next step, so it is
        // worth showing verbatim rather than reduced to "failed".
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (mounted.current) setBusy(false);
      }
    },
    [load]
  );

  const choosePreset = useCallback(
    (next: PresetId) => {
      const previous = preset;
      setPreset(next);
      void setLocalModelPreset(next).catch(() => setPreset(previous));
    },
    [preset]
  );

  const servers = status?.servers ?? [];
  const state = aggregateState(servers);

  // Checkpoints that can serve chat: the ones whose id reads like a chat model,
  // plus whatever is already ticked into the chat slot — a slot is correctable
  // by hand, so an unusual id that a user already chose must stay listed.
  const chatModels = cache
    .map(model => model.id)
    .filter(
      id =>
        inferSlot(id) === 'model' || servers.some(server => (server.settings.model ?? '') === id)
    );

  return (
    <PopoverRoot open={open} onOpenChange={setOpen}>
      <Tooltip label={t('settings.ai.mlx')}>
        <PopoverTrigger asChild>
          <Button
            type="button"
            variant="tertiary"
            size="xs"
            aria-label={t('settings.ai.mlx')}
            analyticsId="mlx-quick-open"
            data-testid="mlx-quick-trigger"
            className="h-7 gap-1.5 rounded-md px-2 text-xs font-medium text-content-muted hover:text-content-secondary">
            <span
              aria-hidden
              data-testid="mlx-quick-dot"
              className={`h-1.5 w-1.5 shrink-0 rounded-full ${STATE_DOT[state]}`}
            />
            MLX
          </Button>
        </PopoverTrigger>
      </Tooltip>

      <PopoverContent align="end" className="w-72 p-2">
        <div className="flex items-center justify-between gap-2 px-1 pb-1.5">
          <span className="text-sm font-semibold text-content">{t('settings.ai.mlx')}</span>
          <Badge variant={STATE_VARIANT[state]}>{t(STATE_LABEL_KEY[state])}</Badge>
        </div>

        {status && !status.enabled && (
          <p className="rounded-md bg-surface-subtle px-2 py-1.5 text-xs text-content-muted">
            {t('mlx.disabledNotice')}
          </p>
        )}

        {error && (
          <p
            data-testid="mlx-quick-error"
            className="rounded-md bg-danger-subtle px-2 py-1.5 text-xs text-danger">
            {error}
          </p>
        )}

        {!status && !error && (
          <p className="px-1 py-2 text-xs text-content-muted">{t('common.loading')}</p>
        )}

        {status && status.memory_budget_gib > 0 && (
          <p className="px-1 pb-1.5 text-xs text-content-muted">
            {t('mlx.memoryUsage')
              .replace('{used}', status.memory_used_gib.toFixed(1))
              .replace('{budget}', status.memory_budget_gib.toFixed(1))}
          </p>
        )}

        {servers.map(server => {
          const held = String(server.settings.model ?? '');
          const up = server.state === 'ready' || server.state === 'starting';
          return (
            <div key={server.id} className="rounded-md border border-line p-2">
              <div className="flex items-center justify-between gap-2">
                <span className="truncate text-xs font-medium text-content">{server.id}</span>
                <Button
                  type="button"
                  variant="tertiary"
                  size="xs"
                  disabled={busy}
                  analyticsId={up ? 'mlx-quick-stop' : 'mlx-quick-start'}
                  data-testid={`mlx-quick-toggle-${server.id}`}
                  onClick={() =>
                    void act(up ? 'openhuman.mlx_stop' : 'openhuman.mlx_start', { id: server.id })
                  }>
                  {t(up ? 'mlx.stop' : 'mlx.start')}
                </Button>
              </div>

              <label className="mt-1.5 block">
                <span className="sr-only">{t('mlx.modelLabel')}</span>
                <select
                  className="w-full rounded-md border border-line bg-surface px-2 py-1 text-xs text-content"
                  value={held}
                  disabled={busy}
                  data-analytics-id="mlx-quick-model"
                  data-testid={`mlx-quick-model-${server.id}`}
                  onChange={event => {
                    const next = event.target.value;
                    // Clearing a tick means clearing whichever slot the old
                    // checkpoint held, not the chat slot by assumption.
                    const previous = held
                      ? occupiedSlot(held, server.settings as Partial<Record<Slot, string | null>>)
                      : null;
                    const patch: Record<string, string> = { model: next };
                    if (previous && previous !== 'model') patch[previous] = '';
                    void act('openhuman.mlx_update_server', { id: server.id, patch });
                  }}>
                  <option value="">{t('mlx.modelNone')}</option>
                  {chatModels.map(id => (
                    <option key={id} value={id}>
                      {shortModelId(id)}
                    </option>
                  ))}
                </select>
              </label>

              {server.loaded_model && (
                <p className="mt-1 truncate text-[11px] text-content-faint">
                  {shortModelId(server.loaded_model)}
                </p>
              )}
            </div>
          );
        })}

        {status && servers.length === 0 && !error && (
          <p className="px-1 py-1.5 text-xs text-content-muted">{t('mlx.noServers')}</p>
        )}

        {/* How hard the model works. The same core setting the chat bar and the
            MLX panel both read, so all three cannot disagree. */}
        <div className="mt-2 flex items-center justify-between gap-2 px-1">
          <span className="text-xs text-content-muted">{t('mlx.preset.title')}</span>
          <ChatPresetPill value={preset} onChange={choosePreset} />
        </div>

        <div className="my-1.5 border-t border-line" />

        <button
          type="button"
          data-analytics-id="mlx-quick-settings"
          onClick={() => {
            setOpen(false);
            navigate('/connections?tab=llm#mlx');
          }}
          className="block w-full rounded-md px-2 py-1.5 text-left text-sm text-content-secondary hover:bg-surface-hover hover:text-content">
          {t('mlx.quick.openSettings')}
        </button>
        <button
          type="button"
          data-analytics-id="mlx-quick-all-settings"
          onClick={() => {
            setOpen(false);
            navigate('/settings');
          }}
          className="block w-full rounded-md px-2 py-1.5 text-left text-sm font-medium text-content hover:bg-surface-hover">
          {t('mlx.quick.allSettings')}
        </button>
      </PopoverContent>
    </PopoverRoot>
  );
}

export { aggregateState, shortModelId };
