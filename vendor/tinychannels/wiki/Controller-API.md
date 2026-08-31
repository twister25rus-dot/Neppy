# Controller API

The controller and backend layer provides UI-facing channel setup metadata and
delegates side effects back to OpenHuman.

## ChannelDefinition

`ChannelDefinition` describes a setup-capable channel:

- machine id
- display name
- description
- icon id
- supported auth modes
- field requirements per auth mode
- runtime capability list

Auth modes are:

- `api_key`
- `bot_token`
- `oauth`
- `managed_dm`

Credential validation happens before backend delegation. Required fields must be
present and non-empty for the selected auth mode.

## Setup Registry

`all_channel_definitions` returns the channel definitions exposed to the setup
UI. This registry is not the same thing as every provider in `src/providers/`;
it only lists setup flows that are ready to surface.

`channel_config_connected` maps runtime config plus auth mode into connected
state for known config-backed channels.

## ChannelBackend

`ChannelBackend` is the OpenHuman-owned side-effect boundary. OpenHuman
implements it with its REST, JWT, config storage, and runtime logic.

The trait includes operations for:

- connect and disconnect
- status and health testing
- typed and raw sends
- normalized outbound intent sends
- reactions
- thread create/update/list
- Telegram managed-DM login start/check
- Discord link start/check
- Discord guild/channel/permission setup
- default-channel get/set

Default methods keep unsupported optional operations explicit. For example,
`send_message_value` returns an error unless a backend supports raw channel
message payloads.

## ChannelManager

`ChannelManager` combines config plus backend:

- lists static definitions
- describes a channel by id
- validates credentials against the definition
- normalizes selected credentials where needed
- delegates side effects to `ChannelBackend`

This keeps controller callers from reaching directly into provider modules or
OpenHuman backend internals.

## Response DTOs

Controller responses are typed for setup UI and API use:

- `ChannelConnectionResult`
- `ChannelDisconnectResult`
- `ChannelStatusEntry`
- `ChannelTestResult`
- `ChannelSendMessageResult`
- `ChannelReactionResult`
- `ChannelThreadResult`
- `ChannelThreadListResult`
- Telegram login results
- Discord link, guild, channel, and permission results
- account snapshot and disconnect metadata

When adding controller behavior, prefer extending these DTOs over returning
opaque JSON unless the platform payload is intentionally provider-native.
