import { listen } from '@tauri-apps/api/event';
import { render, waitFor } from '@testing-library/react';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { isTauri } from '../../../utils/tauriCommands/common';
import CompanionNavigateListener, {
  COMPANION_NAVIGATE_EVENT,
  isPetPath,
} from './CompanionNavigateListener';

vi.mock('../../../utils/tauriCommands/common', () => ({ isTauri: vi.fn() }));

function Where() {
  const loc = useLocation();
  return <p data-testid="where">{`${loc.pathname}${loc.search}`}</p>;
}

describe('CompanionNavigateListener', () => {
  let handler: ((e: { payload: unknown }) => void) | null;
  const unlisten = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    handler = null;
    vi.mocked(isTauri).mockReturnValue(true);
    vi.mocked(listen).mockImplementation((async (_name: string, cb: typeof handler) => {
      handler = cb;
      return unlisten;
    }) as never);
  });

  const mount = () =>
    render(
      <MemoryRouter initialEntries={['/chat']}>
        <CompanionNavigateListener />
        <Where />
      </MemoryRouter>
    );

  it('navigates to the Pet path the tray asks for', async () => {
    const view = mount();
    await waitFor(() => expect(handler).not.toBeNull());
    expect(listen).toHaveBeenCalledWith(COMPANION_NAVIGATE_EVENT, expect.any(Function));
    handler?.({ payload: { path: '/pet?tab=now' } });
    await waitFor(() => expect(view.getByTestId('where')).toHaveTextContent('/pet?tab=now'));
  });

  it('ignores paths outside the Pet page', async () => {
    const view = mount();
    await waitFor(() => expect(handler).not.toBeNull());
    handler?.({ payload: { path: '/settings/security' } });
    handler?.({ payload: { path: 'https://evil.example' } });
    handler?.({ payload: null });
    expect(view.getByTestId('where')).toHaveTextContent('/chat');
  });

  it('does nothing outside Tauri', () => {
    vi.mocked(isTauri).mockReturnValue(false);
    mount();
    expect(listen).not.toHaveBeenCalled();
  });

  it('stops listening on unmount', async () => {
    const view = mount();
    await waitFor(() => expect(handler).not.toBeNull());
    view.unmount();
    expect(unlisten).toHaveBeenCalled();
  });

  it('survives a failing listen call', async () => {
    vi.mocked(listen).mockRejectedValue(new Error('no bridge'));
    expect(() => mount()).not.toThrow();
    await waitFor(() => expect(listen).toHaveBeenCalled());
  });

  it('isPetPath accepts only /pet with a simple query', () => {
    expect(isPetPath('/pet')).toBe(true);
    expect(isPetPath('/pet?tab=now')).toBe(true);
    expect(isPetPath('/pet/../settings')).toBe(false);
    expect(isPetPath('//evil.com')).toBe(false);
    expect(isPetPath(42)).toBe(false);
  });
});
