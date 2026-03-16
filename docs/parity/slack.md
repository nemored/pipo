# Slack parity (Rust -> Go)

## Completed
- Added a dedicated Slack adapter type in the transport factory (no longer generic noop), preserving transport naming/indexing.
- Added reconnect-aware adapter lifecycle loop (`connect` + `session` with retry/backoff).
- Preserved channel mapping semantics by maintaining bus->remote and reverse remote->bus maps in adapter state.

## Pending
- Slack RTM/WebSocket auth + event stream handling.
- Slack users/channels hydration (`conversations.list`, user list caching).
- Message forwarding parity: text/action/bot.
- Edit/delete/reaction parity using shared `pipo_id` + DB lookup/update methods.
- Threading semantics parity.
