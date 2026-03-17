# IRC parity (Rust -> Go)

## Completed
- Added dedicated IRC adapter wiring in factory.
- Adapter has reconnect lifecycle scaffold consistent with Rust transport lifecycle expectations.
- Preserved channel mapping representation for bus<->channel routing.
- Added nickname collision detection and fallback nick retries during registration.
- Added lifecycle heartbeat handling (`PING`/`PONG`) with idle-timeout-driven reconnect.
- Added structured lifecycle logging for `dial`, `tls`, `register`, `ready`, `disconnect`, and `reconnect` phases.

## Pending
- Join/part/names handling parity.
- CTCP action mapping parity.
- Edit/delete/reaction compatibility paths (where represented through normalized events).
- DB interactions for `pipo_id` allocation and lookup on IRC-originated messages.
