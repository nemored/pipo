# Discord parity (Rust -> Go)

## Completed
- Added a dedicated Discord adapter in factory wiring.
- Adapter now uses common connect/reconnect lifecycle scaffold.
- Preserved channel mapping semantics in adapter state.

## Pending
- Gateway lifecycle/session resumption parity.
- Guild/channel bootstrap and channel ID mapping from config.
- Message create/edit/delete/reaction parity bound to shared `pipo_id` and SQLite mapping queries.
- Attachment/embed translation parity.
- Discord thread and reply behavior parity.
