import type { ChannelDefinition, ChannelType } from '../../types/channels';
import CredentialChannelConfig from './CredentialChannelConfig';
import DiscordConfig from './DiscordConfig';
import McpServersTab from './mcp/McpServersTab';
import TelegramConfig from './TelegramConfig';
import WebChannelConfig from './WebChannelConfig';
import YuanbaoConfig from './YuanbaoConfig';

/**
 * The one place that maps a channel to its configuration UI.
 *
 * There were two of these and they had drifted apart in both directions: the
 * Channels page rendered `web` and `mcp` but not `yuanbao`, while the setup
 * modal rendered `yuanbao` but not `web` or `mcp` — so opening Web from the
 * modal showed "Configuration for Web" over an empty panel even though
 * `WebChannelConfig` existed and the page had been rendering it all along.
 * Neither copy was wrong on purpose; each had simply missed an addition made to
 * the other.
 *
 * Returning `null` for an unmapped channel lets each caller keep its own
 * fallback — the modal says so in words, the page renders nothing — without
 * either of them owning the mapping.
 */
export function channelConfigFor(
  channelId: ChannelType,
  definition: ChannelDefinition
): React.ReactNode {
  switch (channelId) {
    case 'telegram':
      return <TelegramConfig definition={definition} />;
    case 'discord':
      return <DiscordConfig definition={definition} />;
    case 'web':
      return <WebChannelConfig definition={definition} />;
    case 'yuanbao':
      return <YuanbaoConfig definition={definition} />;
    case 'mcp':
      return <McpServersTab />;
    // Credential-form channels share one generic form.
    case 'lark':
    case 'dingtalk':
    case 'email':
      return <CredentialChannelConfig definition={definition} />;
    default:
      return null;
  }
}
