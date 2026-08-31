//! Pluggable channel and messaging primitives for OpenHuman harness
//! communication.
//!
//! TinyChannels owns portable channel configuration, controller metadata,
//! runtime helpers, and the backend boundary that OpenHuman implements for
//! channel side effects.

// Re-export the WebSocket transport crate so downstream consumers (and their
// tests that exercise provider WS seams) can construct version-matched
// `tungstenite` messages without pinning the same version themselves.
pub use tokio_tungstenite;

pub mod adapters;
pub mod backend;
pub mod channel;
pub mod config;
pub mod context;
pub mod controllers;
pub mod delivery;
pub mod error;
pub mod harness;
pub mod host;
pub mod providers;
pub mod relay;
pub mod routes;
pub mod runtime;
pub mod security;
pub mod text;
pub mod traits;

pub use backend::{ChannelBackend, ChannelManager};
pub use channel::{
    ChannelInboundEnvelope, ChannelOutboundIntent, DeliveryDurability, OutboundPayload,
    build_session_key_for_inbound_envelope, inbound_envelope_from_legacy_message,
    legacy_message_from_inbound_envelope, legacy_message_value_from_outbound_intent,
    outbound_intent_from_legacy_message, outbound_intent_from_send_message,
};
pub use config::ChannelsConfig;
pub use controllers::{ChannelAuthMode, ChannelDefinition};
pub use error::{Result, TinyChannelsError};
pub use host::{ChannelHost, ChannelHostBuilder, HostCapabilities, NoopHost, ProviderContext};
pub use providers::{
    DingTalkChannel, DiscordChannel, IMessageChannel, IrcChannel, IrcChannelConfig, LinqChannel,
    MattermostChannel, QQChannel, SignalChannel, SlackChannel, TelegramChannel, WhatsAppChannel,
    WhatsAppWebChannel, YuanbaoChannel,
};
// Re-exported separately so each can follow its provider's feature gate.
#[cfg(feature = "email")]
pub use providers::EmailChannel;
#[cfg(feature = "lark")]
pub use providers::LarkChannel;
pub use traits::{Channel, ChannelMessage, ChannelSendExt, SendMessage};

#[cfg(all(test, feature = "email"))]
mod email_feature_smoke_tests {
    use crate::EmailChannel;

    #[test]
    fn email_channel_is_available_with_email_feature() {
        // Verify EmailChannel is exported when `email` feature is enabled.
        // This test ensures the export is reachable at compile time.
        let _ = std::any::type_name::<EmailChannel>();
    }
}

#[cfg(all(test, feature = "lark"))]
mod lark_feature_smoke_tests {
    use crate::LarkChannel;

    #[test]
    fn lark_channel_is_available_with_lark_feature() {
        // Verify LarkChannel is exported when `lark` feature is enabled.
        // This test ensures the export is reachable at compile time.
        let _ = std::any::type_name::<LarkChannel>();
    }
}
