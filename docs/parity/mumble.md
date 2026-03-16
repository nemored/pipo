# Mumble parity (Rust -> Go)

## Completed
- Added dedicated Mumble adapter wiring in factory.
- Added lifecycle scaffold with reconnect support.
- Preserved text channel mapping in adapter state and config parsing already includes voice mapping.

## Pending
- Full Mumble protocol client implementation.
- Protobuf packet encode/decode integration from `protos/Mumble.proto`.
- Voice channel mapping behavior parity.
- Message/edit/delete/reaction parity tied to shared `pipo_id`.
- Certificate handling parity (`client_cert` and `server_cert` behavior).
