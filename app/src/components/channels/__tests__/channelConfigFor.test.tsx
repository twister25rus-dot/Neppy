import { describe, expect, it } from 'vitest';

import { KNOWN_CHANNEL_TYPES } from '../../../types/channels';
import type { ChannelDefinition } from '../../../types/channels';
import { channelConfigFor } from '../channelConfigFor';

const definitionFor = (id: string): ChannelDefinition =>
  ({ id, display_name: id, description: '', capabilities: [] }) as unknown as ChannelDefinition;

describe('channelConfigFor', () => {
  it('maps every known channel to a configuration UI', () => {
    // The guard that matters. Two copies of this mapping had drifted in both
    // directions — the page rendered `web` and `mcp` but not `yuanbao`, the
    // modal the reverse — and the visible symptom was "Configuration for Web"
    // over an empty panel while `WebChannelConfig` existed and worked. A
    // channel added to the union without a config here now fails this test
    // rather than shipping as an empty panel.
    const unmapped = KNOWN_CHANNEL_TYPES.filter(
      id => channelConfigFor(id, definitionFor(id)) == null
    );
    expect(unmapped).toEqual([]);
  });
});
