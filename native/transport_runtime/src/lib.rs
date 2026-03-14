//! Transport runtime protocol primitives.

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

#[cfg(test)]
mod tests {
    use super::{protocol_version, ExitCode};

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
}
