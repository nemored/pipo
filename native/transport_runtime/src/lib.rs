//! Transport runtime integration primitives.

use std::collections::HashMap;

/// Returns the transport runtime protocol version.
pub fn protocol_version() -> &'static str {
    "v1"
}

/// Metadata attached to outbound frames for observability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SenderMeta {
    pub transport_id: usize,
    pub transport_name: String,
}

/// A runtime frame emitted from adapter boundaries.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutboundFrame {
    pub bus: String,
    pub sender: SenderMeta,
    pub kind: FrameKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FrameKind {
    Message { body: String },
}

/// Maps transport channels to bus identifiers.
#[derive(Clone, Debug, Default)]
pub struct ChannelBusMap {
    inner: HashMap<String, String>,
}

impl ChannelBusMap {
    pub fn from_pairs<I, C, B>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (C, B)>,
        C: Into<String>,
        B: Into<String>,
    {
        Self {
            inner: pairs
                .into_iter()
                .map(|(channel, bus)| (channel.into(), bus.into()))
                .collect(),
        }
    }

    pub fn bus_for_channel(&self, channel: &str) -> Option<&str> {
        self.inner.get(channel).map(String::as_str)
    }
}

/// Adapter-side router that emits outbound frames tagged with bus + sender metadata.
#[derive(Clone, Debug)]
pub struct RuntimeRouter {
    sender: SenderMeta,
    mapping: ChannelBusMap,
}

impl RuntimeRouter {
    pub fn new(sender: SenderMeta, mapping: ChannelBusMap) -> Self {
        Self { sender, mapping }
    }

    pub fn outbound_message(&self, channel: &str, body: impl Into<String>) -> Option<OutboundFrame> {
        let bus = self.mapping.bus_for_channel(channel)?;

        Some(OutboundFrame {
            bus: bus.to_string(),
            sender: self.sender.clone(),
            kind: FrameKind::Message { body: body.into() },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_stable() {
        assert_eq!(protocol_version(), "v1");
    }

    #[test]
    fn channel_routes_to_bus() {
        let map = ChannelBusMap::from_pairs([("#general", "bridge"), ("#ops", "infra")]);

        assert_eq!(map.bus_for_channel("#general"), Some("bridge"));
        assert_eq!(map.bus_for_channel("#missing"), None);
    }

    #[test]
    fn outbound_message_includes_bus_and_sender_metadata() {
        let router = RuntimeRouter::new(
            SenderMeta {
                transport_id: 7,
                transport_name: "Discord".to_string(),
            },
            ChannelBusMap::from_pairs([("123", "bridge")]),
        );

        let frame = router.outbound_message("123", "hello").unwrap();

        assert_eq!(
            frame,
            OutboundFrame {
                bus: "bridge".to_string(),
                sender: SenderMeta {
                    transport_id: 7,
                    transport_name: "Discord".to_string(),
                },
                kind: FrameKind::Message {
                    body: "hello".to_string(),
                },
            }
        );
    }

    #[test]
    fn does_not_apply_rust_side_self_send_filtering() {
        let sender = SenderMeta {
            transport_id: 5,
            transport_name: "Slack".to_string(),
        };
        let router = RuntimeRouter::new(sender.clone(), ChannelBusMap::from_pairs([("C1", "bus-1")]));

        // same sender metadata still produces an outbound frame; no in-router self-filter.
        let frame = router.outbound_message("C1", "echo").unwrap();
        assert_eq!(frame.sender, sender);
    }
}
