use transport_runtime::protocol_version;

#[test]
fn smoke_protocol_version_is_not_empty() {
    assert!(!protocol_version().is_empty());
}
