import { screen } from '@testing-library/react';
import { Route, Routes, useLocation } from 'react-router-dom';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../test/test-utils';
import DebugRedirect from './DebugRedirect';

function LocationProbe() {
  const loc = useLocation();
  return <div data-testid="loc">{loc.pathname}</div>;
}

function renderAt(path: string) {
  renderWithProviders(
    <>
      <Routes>
        <Route path="/debug/:threadId?" element={<DebugRedirect />} />
        <Route path="/chat/:threadId?" element={<div data-testid="chat" />} />
      </Routes>
      <LocationProbe />
    </>,
    { initialEntries: [path] }
  );
}

describe('DebugRedirect', () => {
  it('sends /debug to /chat', () => {
    renderAt('/debug');
    expect(screen.getByTestId('chat')).toBeInTheDocument();
    expect(screen.getByTestId('loc')).toHaveTextContent('/chat');
    expect(screen.getByTestId('loc')).not.toHaveTextContent('/debug');
  });

  it('sends /debug/:threadId to /chat/:threadId', () => {
    renderAt('/debug/dbg-1');
    expect(screen.getByTestId('loc')).toHaveTextContent('/chat/dbg-1');
  });
});
