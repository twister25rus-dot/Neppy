import { useT } from '../../lib/i18n/I18nContext';
import { useAppSelector } from '../../store/hooks';
import type { ChannelDefinition } from '../../types/channels';
import ChannelStatusBadge from './ChannelStatusBadge';

interface WebChannelConfigProps {
  definition: ChannelDefinition;
}

/**
 * The built-in web chat's panel.
 *
 * **There is nothing to configure here, and that is a property of the core, not
 * a gap in this file.** `ChannelsConfig` carries an `Option<…Config>` for every
 * other provider and no field at all for `web`; the channel declares one auth
 * mode (`managed_dm`) with an empty `fields` array, needs no credentials, no
 * endpoint and no listener. So this panel cannot grow a settings form without
 * the core growing something to put in it.
 *
 * What it can do is stop being blank. The panel previously said only "Always
 * available", which is true and useless. The thing a reader actually wants to
 * know about this channel is what it does with proactive output, and that is
 * both non-obvious and easy to get wrong: `ProactiveMessageSubscriber::handle`
 * ALWAYS publishes to web first (`proactive.rs`, "1. Always deliver to the web
 * channel"), and `channels_config.active_channel` selects an *additional*
 * external mirror rather than a destination. Someone who sets Telegram as the
 * default channel reasonably assumes web stopped receiving; it did not. Saying
 * so here is the useful content this panel was missing.
 *
 * The default-channel control itself is deliberately NOT repeated here — the
 * channel row on the Connections page already owns it, and two controls over
 * one persisted value is how they drift.
 */
const WebChannelConfig = ({ definition }: WebChannelConfigProps) => {
  const { t } = useT();
  // The mirror target, read from the same Redux value the row's control writes,
  // so the two can never disagree about what is selected.
  const mirror = useAppSelector(state => state.channelConnections.defaultMessagingChannel);
  // `web` (or unset) means web-only delivery — there is no mirror to name.
  const mirrorsElsewhere = Boolean(mirror) && mirror !== 'web';
  const mirrorName = mirrorsElsewhere
    ? t(`channels.${mirror}.displayName`, mirror as string)
    : null;
  // The core ships this in English; the key is the translated form when present.
  const setupNote =
    definition.auth_modes[0]?.description ?? t('channels.web.authMode.managed_dm.description');

  return (
    <div className="space-y-3" data-testid="web-channel-config">
      <div className="flex items-start justify-between">
        <div>
          <ChannelStatusBadge status="connected" />
        </div>
      </div>
      <p className="text-sm text-content-muted">{t('channels.web.alwaysAvailable')}</p>
      <p className="text-xs text-content-muted">{setupNote}</p>

      <div className="rounded-lg border border-line bg-surface-subtle p-3">
        <p className="text-xs font-semibold text-content">{t('channels.web.proactiveTitle')}</p>
        <p className="mt-1 text-xs text-content-muted">{t('channels.web.proactiveAlways')}</p>
        <p className="mt-1 text-xs text-content-muted" data-testid="web-channel-mirror">
          {mirrorsElsewhere
            ? t('channels.web.proactiveMirror').replace('{channel}', mirrorName ?? '')
            : t('channels.web.proactiveNoMirror')}
        </p>
      </div>
    </div>
  );
};

export default WebChannelConfig;
