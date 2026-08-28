import { describe, expect, it } from 'vitest';

import { isLocalModeUnsupported, LOCAL_MODE_UNSUPPORTED_CODE } from '../localModeApi';

describe('isLocalModeUnsupported', () => {
  it('recognises a local backend 501 body', () => {
    expect(
      isLocalModeUnsupported({
        code: LOCAL_MODE_UNSUPPORTED_CODE,
        error: 'not available locally',
        path: '/payments/credits/balance',
        service_id: 'account.billing',
        local_alternative: 'Nothing to bill.',
      })
    ).toBe(true);
  });

  it('accepts a body without the optional fields', () => {
    // The fallback shape for a route no service entry claims.
    expect(
      isLocalModeUnsupported({
        code: LOCAL_MODE_UNSUPPORTED_CODE,
        error: 'x',
        path: '/future/route',
        local_alternative: 'Turn local mode off to reach the hosted backend.',
      })
    ).toBe(true);
  });

  it('rejects anything else', () => {
    expect(isLocalModeUnsupported(null)).toBe(false);
    expect(isLocalModeUnsupported(undefined)).toBe(false);
    expect(isLocalModeUnsupported('local_mode_unsupported')).toBe(false);
    expect(isLocalModeUnsupported({ code: 'unauthorized' })).toBe(false);
    // The code alone is not enough — a body missing the alternative would make
    // callers render an empty "use this instead" line.
    expect(isLocalModeUnsupported({ code: LOCAL_MODE_UNSUPPORTED_CODE })).toBe(false);
  });
});
