/**
 * Vitest coverage for the two new cron tauriCommand wrappers added by the
 * skills runner PR: neppyCronRun and neppyCronRuns.
 *
 * Follows the same mocking pattern as subconscious.test.ts — isTauri()
 * guard + callCoreRpc mock, no real Tauri runtime.
 */
import { isTauri } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, type Mock, test, vi } from 'vitest';

import { callCoreRpc } from '../../services/coreRpcClient';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock('../../services/coreRpcClient', () => ({ callCoreRpc: vi.fn() }));

describe('tauriCommands/cron — neppyCronRun / neppyCronRuns', () => {
  const mockIsTauri = isTauri as Mock;
  const mockCallCoreRpc = callCoreRpc as Mock;
  let neppyCronAdd: typeof import('./cron').neppyCronAdd;
  let neppyCronRun: typeof import('./cron').neppyCronRun;
  let neppyCronRuns: typeof import('./cron').neppyCronRuns;

  beforeEach(async () => {
    vi.clearAllMocks();
    mockIsTauri.mockReturnValue(true);
    const m = await vi.importActual<typeof import('./cron')>('./cron');
    neppyCronAdd = m.neppyCronAdd;
    neppyCronRun = m.neppyCronRun;
    neppyCronRuns = m.neppyCronRuns;
  });

  afterEach(() => vi.restoreAllMocks());

  describe('neppyCronAdd', () => {
    const params = { schedule: { kind: 'cron' as const, expr: '*/5 * * * *' }, name: 'test' };

    test('throws when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(neppyCronAdd(params)).rejects.toThrow('Not running in Tauri');
    });

    test('calls cron_add with params', async () => {
      mockCallCoreRpc.mockResolvedValue({ id: 'job-1' });
      await neppyCronAdd(params);
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.cron_add' })
      );
    });
  });

  describe('neppyCronRun', () => {
    test('throws when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(neppyCronRun('job-1')).rejects.toThrow('Not running in Tauri');
    });

    test('calls cron_run with job_id', async () => {
      mockCallCoreRpc.mockResolvedValue({
        job_id: 'job-1',
        status: 'ok',
        duration_ms: 100,
        output: '',
      });
      await neppyCronRun('job-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ method: 'openhuman.cron_run', params: { job_id: 'job-1' } })
      );
    });
  });

  describe('neppyCronRuns', () => {
    test('throws when not in Tauri', async () => {
      mockIsTauri.mockReturnValue(false);
      await expect(neppyCronRuns('job-1')).rejects.toThrow('Not running in Tauri');
    });

    test('calls cron_runs with job_id and default limit', async () => {
      mockCallCoreRpc.mockResolvedValue({ runs: [] });
      await neppyCronRuns('job-1');
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({
          method: 'openhuman.cron_runs',
          params: expect.objectContaining({ job_id: 'job-1', limit: 20 }),
        })
      );
    });

    test('passes custom limit', async () => {
      mockCallCoreRpc.mockResolvedValue({ runs: [] });
      await neppyCronRuns('job-1', 5);
      expect(mockCallCoreRpc).toHaveBeenCalledWith(
        expect.objectContaining({ params: expect.objectContaining({ limit: 5 }) })
      );
    });
  });
});
