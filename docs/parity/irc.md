# IRC parity (Rust -> Go)

## Completed
- Added dedicated IRC adapter wiring in factory.
- Adapter has reconnect lifecycle scaffold consistent with Rust transport lifecycle expectations.
- Preserved channel mapping representation for bus<->channel routing.
- Added nickname collision detection and fallback nick retries during registration.
- Added lifecycle heartbeat handling (`PING`/`PONG`) with idle-timeout-driven reconnect.
- Added structured lifecycle logging for `dial`, `tls`, `register`, `ready`, `disconnect`, and `reconnect` phases.
- Added IRC parser/dispatcher handling for membership lifecycle events (`JOIN`, `PART`, `KICK`, `QUIT`, `NICK`) plus `353/366` names replies.
- Added in-memory channel membership tracking and normalized `Names` bus snapshots on joins/leaves/nick changes/reconnect bootstrap.
- Added mapped/unmapped channel behavior for membership events by dropping unmapped channels with warning logs.

## Pending
- CTCP action mapping parity.
- Edit/delete compatibility translation from IRC command conventions (`!edit`, `!delete`) with compatibility metadata and capability-gated fallback annotations.
- Reaction compatibility translation from configurable IRC command prefix (`react ` by default), gated by explicit capability toggle.
- DB interactions for `pipo_id` allocation and lookup on IRC-originated messages.
