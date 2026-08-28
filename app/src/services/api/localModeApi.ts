import { callCoreRpc } from '../coreRpcClient';
import { CORE_RPC_METHODS } from '../rpcMethods';

/**
 * How completely a hosted service is replaced when Local Mode is on.
 * Mirrors `LocalReplacementKind` in the Rust core (snake_case on the wire).
 */
export type LocalReplacementKind = 'replaced' | 'requires_setup' | 'unavailable' | 'not_applicable';

/** One hosted service and what stands in for it locally. */
export interface LocalServiceEntry {
  /** Stable dotted id — the join key between the core, this UI, and the docs. */
  id: string;
  hosted: string;
  routes: string[];
  kind: LocalReplacementKind;
  local_alternative: string;
  /** Present only for `requires_setup`; empty otherwise. */
  setup: string;
}

export interface LocalServiceInventory {
  entries: LocalServiceEntry[];
  replaced: number;
  requires_setup: number;
  unavailable: number;
  not_applicable: number;
}

export interface LocalModeStatus {
  /** The persisted setting. */
  enabled: boolean;
  /**
   * What the core is *actually* doing. Differs from `enabled` across an
   * `OPENHUMAN_LOCAL_MODE` override and while a restart is still pending, so
   * the UI must never conflate the two — that is how a settings panel reports
   * a change as applied when it is not.
   */
  active: boolean;
  backendPort: number;
  backendUrl: string;
  applyLocalDefaults: boolean;
  proxyInference: boolean;
  /** True when the change needs a core restart to bind or release the listener. */
  restartRequired: boolean;
  services: LocalServiceInventory;
}

export interface LocalModePatch {
  enabled?: boolean;
  backend_port?: number;
  apply_local_defaults?: boolean;
  proxy_inference?: boolean;
}

/** Read the Local Mode posture and the hosted-service inventory. */
export async function getLocalMode(): Promise<LocalModeStatus> {
  const response = await callCoreRpc<{ result: LocalModeStatus }>({
    method: CORE_RPC_METHODS.configGetLocalMode,
    params: {},
  });
  return response.result;
}

/** Update the Local Mode settings. Returns the posture after the write. */
export async function setLocalMode(patch: LocalModePatch): Promise<LocalModeStatus> {
  const response = await callCoreRpc<{ result: LocalModeStatus }>({
    method: CORE_RPC_METHODS.configSetLocalMode,
    params: patch,
  });
  return response.result;
}

/**
 * The error code the local backend returns for a hosted route with no local
 * implementation. Clients branch on this rather than on the message text.
 */
export const LOCAL_MODE_UNSUPPORTED_CODE = 'local_mode_unsupported';

/** Body of a `local_mode_unsupported` response. */
export interface LocalModeUnsupportedError {
  code: typeof LOCAL_MODE_UNSUPPORTED_CODE;
  error: string;
  path: string;
  service_id?: string;
  local_alternative: string;
  setup?: string;
}

/**
 * Narrow an unknown thrown value or parsed response body to a
 * `local_mode_unsupported` error.
 *
 * Used by feature code that calls a hosted route so it can render the local
 * alternative in place of a generic failure. Deliberately structural rather
 * than an `instanceof` check: the payload reaches callers as parsed JSON from
 * several transports (fetch bodies, RPC error data, thrown `Error`s carrying a
 * serialized cause), and none of them share a class.
 */
export function isLocalModeUnsupported(value: unknown): value is LocalModeUnsupportedError {
  if (typeof value !== 'object' || value === null) return false;
  const candidate = value as Partial<LocalModeUnsupportedError>;
  return (
    candidate.code === LOCAL_MODE_UNSUPPORTED_CODE &&
    typeof candidate.local_alternative === 'string'
  );
}
