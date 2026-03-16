# Rachni parity (Rust -> Go)

## Completed
- Added dedicated Rachni adapter wiring and explicit `buses` routing usage.
- Added lifecycle scaffold suitable for periodic polling + reconnect behavior.
- Preserved Rust DB quirk with `InsertAllocatedIDRachni` returning post-increment value after insert.

## Pending
- Polling implementation against Rachni stream endpoint.
- Start/stop stream detection parity and emitted action text parity.
- Full event publication loop integration using shared runtime API.
