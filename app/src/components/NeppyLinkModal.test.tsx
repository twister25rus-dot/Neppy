import { describe, expect, it } from 'vitest';

import type { AccountStatus } from '../types/accounts';
import { statusDisplay } from './NeppyLinkModal';

describe('statusDisplay', () => {
  it('maps every account lifecycle status to a translation key and dot color', () => {
    const cases: Array<[AccountStatus, string, string]> = [
      ['open', 'app.neppyLink.status.connected', 'bg-emerald-500'],
      ['loading', 'app.neppyLink.status.loading', 'bg-amber-400'],
      ['pending', 'app.neppyLink.status.needsSignIn', 'bg-amber-400'],
      ['timeout', 'app.neppyLink.status.timedOut', 'bg-red-400'],
      ['error', 'app.neppyLink.status.error', 'bg-red-400'],
      ['closed', 'app.neppyLink.status.closed', 'bg-stone-300'],
    ];

    for (const [status, labelKey, dotClass] of cases) {
      expect(statusDisplay(status)).toEqual({ labelKey, dotClass });
    }
  });

  it('returns a key under the app.neppyLink.status namespace for every status', () => {
    const statuses: AccountStatus[] = ['open', 'loading', 'pending', 'timeout', 'error', 'closed'];
    for (const status of statuses) {
      expect(statusDisplay(status).labelKey).toMatch(/^app\.neppyLink\.status\./);
    }
  });
});
