import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  applyLocalInstall,
  commitDebugTask,
  getDebugDiff,
  getDebugSettings,
  getDebugStatus,
  getDebugTask,
  getLocalInstallResult,
  getLocalInstallStatus,
  listDebugCheckpoints,
  listDebugTasks,
  quitApp,
  rollbackDebug,
  startLocalInstallBuild,
  tailDebugAudit,
  updateDebugSettings,
} from './debugModeApi';

const mockCallCoreRpc = vi.fn();
const mockInvoke = vi.fn();
const mockIsTauri = vi.fn();

vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => mockInvoke(...args) }));
vi.mock('../../utils/tauriCommands/common', () => ({ isTauri: () => mockIsTauri() }));

vi.mock('../coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

describe('debugModeApi', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  it('getDebugStatus calls debug_mode_status and returns the bare value', async () => {
    mockCallCoreRpc.mockResolvedValue({ project_root: '/r', branch: 'main', head: 'abc' });
    const status = await getDebugStatus();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({ method: 'neppy.debug_mode_status', params: {} });
    expect(status.branch).toBe('main');
  });

  it('unwraps the { result, logs } envelope', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: [{ id: 't1' }], logs: ['x'] });
    const tasks = await listDebugTasks(1);
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'neppy.debug_mode_task_list',
      params: { limit: 1 },
    });
    expect(tasks).toEqual([{ id: 't1' }]);
  });

  it('omits optional params when not given', async () => {
    mockCallCoreRpc.mockResolvedValue([]);
    await listDebugTasks();
    await listDebugCheckpoints();
    await tailDebugAudit();
    await getDebugDiff();
    expect(mockCallCoreRpc.mock.calls.map(c => (c[0] as { params: unknown }).params)).toEqual([
      {},
      {},
      {},
      {},
    ]);
  });

  it('getDebugTask and getDebugDiff send snake_case ids', async () => {
    mockCallCoreRpc.mockResolvedValue({});
    await getDebugTask('task-1');
    await getDebugDiff('cp-1');
    expect(mockCallCoreRpc).toHaveBeenNthCalledWith(1, {
      method: 'neppy.debug_mode_task_get',
      params: { task_id: 'task-1' },
    });
    expect(mockCallCoreRpc).toHaveBeenNthCalledWith(2, {
      method: 'neppy.debug_mode_diff',
      params: { checkpoint_id: 'cp-1' },
    });
  });

  it('rollbackDebug sends the checkpoint id with confirm:true', async () => {
    mockCallCoreRpc.mockResolvedValue({
      checkpoint_id: 'cp-1',
      pre_rollback_checkpoint_id: 'cp-2',
    });
    const r = await rollbackDebug('cp-1');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'neppy.debug_mode_rollback',
      params: { checkpoint_id: 'cp-1', confirm: true },
    });
    expect(r.pre_rollback_checkpoint_id).toBe('cp-2');
  });

  it('commitDebugTask sends task id and message with confirm:true', async () => {
    mockCallCoreRpc.mockResolvedValue({ task_id: 't1', commit: 'deadbeef', files: ['a'] });
    const r = await commitDebugTask('t1', 'feat(debug): x');
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'neppy.debug_mode_commit',
      params: { task_id: 't1', message: 'feat(debug): x', confirm: true },
    });
    expect(r.commit).toBe('deadbeef');
  });

  it('propagates core errors', async () => {
    mockCallCoreRpc.mockRejectedValue(new Error('commit refused'));
    await expect(commitDebugTask('t1', 'm')).rejects.toThrow('commit refused');
  });

  it('getDebugSettings calls settings_get with no params', async () => {
    mockCallCoreRpc.mockResolvedValue({ enabled: true, max_repair_iterations: 5 });
    const settings = await getDebugSettings();
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'neppy.debug_mode_settings_get',
      params: {},
    });
    expect(settings.max_repair_iterations).toBe(5);
  });

  it('updateDebugSettings sends only the patch and unwraps the envelope', async () => {
    mockCallCoreRpc.mockResolvedValue({ result: { auto_repair: false }, logs: [] });
    const next = await updateDebugSettings({ auto_repair: false });
    expect(mockCallCoreRpc).toHaveBeenCalledWith({
      method: 'neppy.debug_mode_settings_update',
      params: { patch: { auto_repair: false } },
    });
    expect(next).toEqual({ auto_repair: false });
  });

  it('updateDebugSettings propagates RPC errors', async () => {
    mockCallCoreRpc.mockRejectedValue(new Error('max_repair_iterations out of range'));
    await expect(updateDebugSettings({ max_repair_iterations: 99 })).rejects.toThrow(
      'out of range'
    );
  });
});

describe('debugModeApi local install', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
    mockInvoke.mockReset();
  });

  it('maps each local install call to its RPC, sending confirm only on apply', async () => {
    mockCallCoreRpc.mockResolvedValue({ phase: 'ready' });
    await startLocalInstallBuild();
    await getLocalInstallStatus();
    await applyLocalInstall();
    const calls = mockCallCoreRpc.mock.calls.map(c => c[0]);
    expect(calls).toEqual([
      { method: 'neppy.debug_mode_install_local_build', params: {} },
      { method: 'neppy.debug_mode_install_local_status', params: {} },
      { method: 'neppy.debug_mode_install_local_apply', params: { confirm: true } },
    ]);
  });

  it('reads the installer result and acknowledges only when asked', async () => {
    mockCallCoreRpc.mockResolvedValue(null);
    expect(await getLocalInstallResult()).toBeNull();
    await getLocalInstallResult(true);
    expect(mockCallCoreRpc.mock.calls.map(c => c[0].params)).toEqual([{}, { acknowledge: true }]);
  });

  it('quits through the Tauri app_quit command, and is a no-op outside Tauri', async () => {
    mockIsTauri.mockReturnValue(false);
    await quitApp();
    expect(mockInvoke).not.toHaveBeenCalled();
    mockIsTauri.mockReturnValue(true);
    await quitApp();
    expect(mockInvoke).toHaveBeenCalledWith('app_quit');
  });
});
