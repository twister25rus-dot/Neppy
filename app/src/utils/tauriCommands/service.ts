/**
 * Service and daemon management commands.
 */
import { invoke } from '@tauri-apps/api/core';

import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri, parseServiceCliOutput } from './common';

export type ServiceState = 'Running' | 'Stopped' | 'NotInstalled' | { Unknown: string };

export interface ServiceStatus {
  state: ServiceState;
  unit_path?: string | null;
  label: string;
  details?: string | null;
}

export interface AgentServerStatus {
  running: boolean;
  url: string;
}

export interface DaemonHostConfig {
  show_tray: boolean;
}

export interface RestartStatus {
  accepted: boolean;
  source: string;
  reason: string;
}

export async function neppyServiceInstall(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({
      method: 'openhuman.service_install',
    });
  } catch {
    const raw = await invoke<string>('service_install_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function neppyServiceStart(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({ method: 'openhuman.service_start' });
  } catch {
    const raw = await invoke<string>('service_start_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function neppyServiceStop(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({ method: 'openhuman.service_stop' });
  } catch {
    const raw = await invoke<string>('service_stop_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function neppyServiceStatus(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({
      method: 'openhuman.service_status',
    });
  } catch {
    const raw = await invoke<string>('service_status_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function neppyServiceUninstall(): Promise<CommandResponse<ServiceStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  try {
    return await callCoreRpc<CommandResponse<ServiceStatus>>({
      method: 'openhuman.service_uninstall',
    });
  } catch {
    const raw = await invoke<string>('service_uninstall_direct');
    return parseServiceCliOutput<ServiceStatus>(raw);
  }
}

export async function neppyServiceRestart(
  source?: string,
  reason?: string
): Promise<CommandResponse<RestartStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<RestartStatus>>({
    method: 'openhuman.service_restart',
    params: { source, reason },
  });
}

export async function neppyAgentServerStatus(): Promise<CommandResponse<AgentServerStatus>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<AgentServerStatus>>({
    method: 'openhuman.agent_server_status',
  });
}

export async function neppyGetDaemonHostConfig(): Promise<CommandResponse<DaemonHostConfig>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DaemonHostConfig>>({
    method: 'openhuman.service_daemon_host_get',
  });
}

export async function neppySetDaemonHostConfig(
  showTray: boolean
): Promise<CommandResponse<DaemonHostConfig>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<DaemonHostConfig>>({
    method: 'openhuman.service_daemon_host_set',
    params: { show_tray: showTray },
  });
}
