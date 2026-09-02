import { describe, expect, it } from 'vitest';

import { neppyMigrateHermes } from '../core';

describe('neppyMigrateHermes', () => {
  it('throws when not running in Tauri', async () => {
    await expect(neppyMigrateHermes()).rejects.toThrow('Not running in Tauri');
  });
});
