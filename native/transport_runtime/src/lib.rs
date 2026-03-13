//! Minimal transport runtime placeholder crate.

/// Returns a stable placeholder protocol version string.
pub fn protocol_version() -> &'static str {
    "v0-placeholder"
}

#[cfg(test)]
mod tests {
    use super::protocol_version;

    #[test]
    fn protocol_version_is_stable() {
        assert_eq!(protocol_version(), "v0-placeholder");
    }
}
