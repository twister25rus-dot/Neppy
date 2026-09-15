import { screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import type { ChannelDefinition } from '../../../types/channels';
import WebChannelConfig from '../WebChannelConfig';

const definition: ChannelDefinition = {
  id: 'web',
  display_name: 'Web',
  description: 'Chat via the built-in web UI.',
  icon: 'web',
  auth_modes: [
    {
      mode: 'managed_dm',
      description: 'Use the embedded web chat: no setup required.',
      fields: [],
    },
  ],
  capabilities: ['send_text', 'send_rich_text', 'receive_text'],
};

function renderWith(defaultMessagingChannel: string) {
  return renderWithProviders(<WebChannelConfig definition={definition} />, {
    preloadedState: {
      channelConnections: {
        schemaVersion: 1,
        migrationCompleted: true,
        defaultMessagingChannel,
        connections: {},
      },
    } as never,
  });
}

describe('<WebChannelConfig />', () => {
  it('explains that proactive messages always land here', () => {
    renderWith('web');

    // The panel used to say only "Always available", which told a reader
    // nothing they could act on.
    expect(screen.getByText(/always arrive here first/i)).toBeInTheDocument();
  });

  it('names the channel that additionally mirrors them', () => {
    renderWith('telegram');

    // `active_channel` picks an extra mirror, NOT a replacement destination, so
    // the copy has to read as "as well as", never "instead of".
    expect(screen.getByTestId('web-channel-mirror')).toHaveTextContent(/also sent to/i);
    expect(screen.getByTestId('web-channel-mirror')).toHaveTextContent(/Telegram/i);
  });

  it('says so when nothing mirrors them', () => {
    renderWith('web');

    expect(screen.getByTestId('web-channel-mirror')).toHaveTextContent(/No other channel/i);
  });

  it('shows the setup note the channel definition carries', () => {
    renderWith('web');

    // Real data from the core that the panel previously fetched and discarded.
    expect(screen.getByText(/no setup required/i)).toBeInTheDocument();
  });

  it('does not repeat the default-channel control the channel row owns', () => {
    renderWith('telegram');

    // Two controls over one persisted value is how they drift; the row keeps it.
    expect(screen.queryByRole('button', { name: /set as default/i })).not.toBeInTheDocument();
  });
});
