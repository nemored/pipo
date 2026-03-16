# IRC parity (Rust -> Go)

## Completed
- Added dedicated IRC adapter wiring in factory.
- Adapter has reconnect lifecycle scaffold consistent with Rust transport lifecycle expectations.
- Preserved channel mapping representation for bus<->channel routing.

## Pending
- IRC socket/TLS connection and registration lifecycle.
- Join/part/names handling parity.
- CTCP action mapping parity.
- Edit/delete/reaction compatibility paths (where represented through normalized events).
- DB interactions for `pipo_id` allocation and lookup on IRC-originated messages.
