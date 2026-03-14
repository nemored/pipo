//! Transport runtime protocol primitives.

use std::collections::HashMap;

use serde_json::Value;

pub mod protocol;

/// Returns the supported protocol version string.
pub fn protocol_version() -> &'static str {
    protocol::SUPPORTED_PROTOCOL_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    OrderlyShutdown = 0,
    InvalidArgsOrConfig = 2,
    ProtocolVersionIncompatible = 3,
    TransportAuthFailure = 10,
    TransportNetworkUnreachable = 11,
    TransportFatal = 12,
    InternalPanic = 20,
}

impl ExitCode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderMeta {
    pub channel: String,
    pub pipo_id: Option<i64>,
    pub origin_transport: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    Message,
}

pub type ChannelBusMap = HashMap<String, String>;

#[derive(Debug, Clone, PartialEq)]
pub struct RoutedOutboundFrame {
    pub kind: FrameKind,
    pub bus: String,
    pub pipo_id: Option<i64>,
    pub origin_transport: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeRouter {
    channel_bus_map: ChannelBusMap,
}

impl RuntimeRouter {
    pub fn new(channel_bus_map: ChannelBusMap) -> Self {
        Self { channel_bus_map }
    }

    pub fn bus_for_channel(&self, channel: &str) -> Option<&str> {
        self.channel_bus_map.get(channel).map(String::as_str)
    }

    pub fn outbound_message(
        &self,
        sender: &SenderMeta,
        payload: Value,
    ) -> Option<RoutedOutboundFrame> {
        let bus = self.bus_for_channel(&sender.channel)?.to_owned();
        Some(RoutedOutboundFrame {
            kind: FrameKind::Message,
            bus,
            pipo_id: sender.pipo_id,
            origin_transport: sender.origin_transport.clone(),
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        protocol_version, ChannelBusMap, ExitCode, FrameKind, RuntimeRouter, SenderMeta,
    };
    use serde_json::json;

    #[test]
    fn protocol_version_is_stable() {
        assert_eq!(protocol_version(), "v1");
    }

    #[test]
    fn exit_codes_match_spec() {
        assert_eq!(ExitCode::OrderlyShutdown.as_i32(), 0);
        assert_eq!(ExitCode::InvalidArgsOrConfig.as_i32(), 2);
        assert_eq!(ExitCode::ProtocolVersionIncompatible.as_i32(), 3);
        assert_eq!(ExitCode::TransportAuthFailure.as_i32(), 10);
        assert_eq!(ExitCode::TransportNetworkUnreachable.as_i32(), 11);
        assert_eq!(ExitCode::TransportFatal.as_i32(), 12);
        assert_eq!(ExitCode::InternalPanic.as_i32(), 20);
    }

    #[test]
    fn channel_bus_mapping_resolves_for_known_channel() {
        let mut map = ChannelBusMap::new();
        map.insert("alerts".to_owned(), "ops".to_owned());
        let router = RuntimeRouter::new(map);

        assert_eq!(router.bus_for_channel("alerts"), Some("ops"));
        assert_eq!(router.bus_for_channel("unknown"), None);
    }

    #[test]
    fn outbound_message_routes_with_sender_metadata() {
        let mut map = ChannelBusMap::new();
        map.insert("alerts".to_owned(), "ops".to_owned());
        let router = RuntimeRouter::new(map);

        let sender = SenderMeta {
            channel: "alerts".to_owned(),
            pipo_id: Some(42),
            origin_transport: "slack".to_owned(),
        };

        let payload = json!({"text": "hello"});
        let frame = router
            .outbound_message(&sender, payload.clone())
            .expect("channel should be routable");

        assert_eq!(frame.kind, FrameKind::Message);
        assert_eq!(frame.bus, "ops");
        assert_eq!(frame.pipo_id, Some(42));
        assert_eq!(frame.origin_transport, "slack");
        assert_eq!(frame.payload, payload);
    }
}
