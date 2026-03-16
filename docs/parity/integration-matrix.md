# Integration parity matrix (Rust ↔ Go)

This document defines integration scenarios used to close parity gaps tracked in `docs/parity/*.md`.

## Scope and exclusions

- In scope transports: **IRC, Mumble, Rachni, Slack, Discord**.
- **Minecraft is explicitly excluded** from integration execution because it is intentionally not implemented in runtime parity work.

## Scenario ID format

- `<TRANSPORT>-<AREA>-<NN>` (example: `SLACK-MSG-01`).
- Environment suffix in matrix cells:
  - `L` = local emulator/mock service.
  - `R` = real third-party API.
  - `L+R` = both (local contract run + real API parity run).
- `N/A` = scenario does not apply to this transport.

## Integration matrix

| Transport | connect | reconnect | message create/edit/delete | reaction | threading (where applicable) | mapping persistence | failure/retry |
|---|---|---|---|---|---|---|---|
| IRC | `IRC-CON-01 (L)` socket/TLS + registration | `IRC-REC-01 (L)` reconnect + re-register | `IRC-MSG-01 (L)` create + CTCP action normalization; `IRC-MSG-02 (L)` edit/delete compatibility path | `IRC-REA-01 (L)` reaction compatibility (normalized fallback event) | `N/A` | `IRC-MAP-01 (L)` `pipo_id` allocate/lookup durability | `IRC-RET-01 (L)` transient socket failure + retry/backoff |
| Mumble | `MUMBLE-CON-01 (L)` protocol connect + cert negotiation | `MUMBLE-REC-01 (L)` reconnect after server restart | `MUMBLE-MSG-01 (L)` message create/edit/delete normalization | `MUMBLE-REA-01 (L)` reaction parity if represented in normalized event stream | `N/A` | `MUMBLE-MAP-01 (L)` shared `pipo_id` mapping persistence | `MUMBLE-RET-01 (L)` decode/stream failure recovery |
| Rachni | `RACHNI-CON-01 (L)` polling stream bootstrap | `RACHNI-REC-01 (L)` polling loop resume after interruption | `RACHNI-MSG-01 (L)` emitted action text on start/stop stream lifecycle events | `N/A` | `N/A` | `RACHNI-MAP-01 (L)` `InsertAllocatedIDRachni` parity + mapping persistence checks | `RACHNI-RET-01 (L)` stream endpoint failures + retry behavior |
| Slack | `SLACK-CON-01 (L+R)` RTM/WebSocket auth and start | `SLACK-REC-01 (L+R)` reconnect/session continuity | `SLACK-MSG-01 (L+R)` text/action/bot create + edit/delete mapping | `SLACK-REA-01 (L+R)` add/remove reaction parity | `SLACK-THR-01 (L+R)` thread reply + parent linkage | `SLACK-MAP-01 (L+R)` `pipo_id` + channel/user cache persistence | `SLACK-RET-01 (L+R)` rate limit/disconnect retry and backoff |
| Discord | `DISCORD-CON-01 (L+R)` gateway identify/bootstrap | `DISCORD-REC-01 (L+R)` resume + sequence continuity | `DISCORD-MSG-01 (L+R)` create/edit/delete + attachment/embed translation | `DISCORD-REA-01 (L+R)` reaction add/remove parity | `DISCORD-THR-01 (L+R)` thread + reply behavior | `DISCORD-MAP-01 (L+R)` channel ID mapping + `pipo_id` persistence | `DISCORD-RET-01 (L+R)` gateway interruption retry/resume |
| Minecraft | `EXC-MC-01` excluded / not implemented | `EXC-MC-01` excluded / not implemented | `EXC-MC-01` excluded / not implemented | `EXC-MC-01` excluded / not implemented | `EXC-MC-01` excluded / not implemented | `EXC-MC-01` excluded / not implemented | `EXC-MC-01` excluded / not implemented |

## Local emulator vs real API policy

### Local emulator required

- IRC: mock IRC daemon covering registration, joins/parts/names, CTCP action paths, and disconnect/reconnect simulation.
- Mumble: local Mumble server + protobuf fixture stream for packet encode/decode coverage.
- Rachni: local HTTP stream endpoint emulator with controllable start/stop/failure states.
- Slack/Discord: local replay harness for deterministic normalization and DB delta assertions.

### Real third-party API required

- Slack: RTM/WebSocket auth/session behavior, channel/user hydration, reaction/thread semantics, and retry behavior under real API constraints.
- Discord: gateway identify/resume, guild/channel bootstrap, attachments/embeds translation, thread/reply semantics, and reconnect behavior.

## Shared assertions required in **both Go and Rust** runs

For every scenario above, the test runner must evaluate both implementations against the same fixture/session and assert:

1. **Event normalization parity**
   - Same ordered normalized event type sequence.
   - Equivalent normalized payload fields for actor/channel/content/thread/reaction metadata.
   - Equivalent treatment of unsupported platform-native operations (fallback normalized events where applicable).

2. **`messages` table delta parity**
   - Same inserted/updated/deleted row counts in `messages` per scenario.
   - Same `pipo_id` linkage behavior (allocation, lookup hits/misses, and update targets).
   - Same idempotency outcomes on retries/replays.

## Pending checklist coverage map

Each pending parity checklist item is mapped to at least one scenario ID.

### IRC (`docs/parity/irc.md`)

- IRC socket/TLS connection and registration lifecycle → `IRC-CON-01`.
- Join/part/names handling parity → `IRC-CON-01`, `IRC-REC-01`.
- CTCP action mapping parity → `IRC-MSG-01`.
- Edit/delete/reaction compatibility paths → `IRC-MSG-02`, `IRC-REA-01`.
- DB interactions for `pipo_id` allocation and lookup on IRC-originated messages → `IRC-MAP-01`.

### Mumble (`docs/parity/mumble.md`)

- Full Mumble protocol client implementation → `MUMBLE-CON-01`, `MUMBLE-REC-01`.
- Protobuf packet encode/decode integration from `protos/Mumble.proto` → `MUMBLE-CON-01`, `MUMBLE-RET-01`.
- Voice channel mapping behavior parity → `MUMBLE-CON-01`, `MUMBLE-MAP-01`.
- Message/edit/delete/reaction parity tied to shared `pipo_id` → `MUMBLE-MSG-01`, `MUMBLE-REA-01`, `MUMBLE-MAP-01`.
- Certificate handling parity (`client_cert` and `server_cert` behavior) → `MUMBLE-CON-01`.

### Rachni (`docs/parity/rachni.md`)

- Polling implementation against Rachni stream endpoint → `RACHNI-CON-01`.
- Start/stop stream detection parity and emitted action text parity → `RACHNI-MSG-01`.
- Full event publication loop integration using shared runtime API → `RACHNI-CON-01`, `RACHNI-REC-01`, `RACHNI-RET-01`.

### Slack (`docs/parity/slack.md`)

- Slack RTM/WebSocket auth + event stream handling → `SLACK-CON-01`.
- Slack users/channels hydration (`conversations.list`, user list caching) → `SLACK-CON-01`, `SLACK-MAP-01`.
- Message forwarding parity: text/action/bot → `SLACK-MSG-01`.
- Edit/delete/reaction parity using shared `pipo_id` + DB lookup/update methods → `SLACK-MSG-01`, `SLACK-REA-01`, `SLACK-MAP-01`.
- Threading semantics parity → `SLACK-THR-01`.

### Discord (`docs/parity/discord.md`)

- Gateway lifecycle/session resumption parity → `DISCORD-CON-01`, `DISCORD-REC-01`.
- Guild/channel bootstrap and channel ID mapping from config → `DISCORD-CON-01`, `DISCORD-MAP-01`.
- Message create/edit/delete/reaction parity bound to shared `pipo_id` and SQLite mapping queries → `DISCORD-MSG-01`, `DISCORD-REA-01`, `DISCORD-MAP-01`.
- Attachment/embed translation parity → `DISCORD-MSG-01`.
- Discord thread and reply behavior parity → `DISCORD-THR-01`.

### Minecraft (`docs/parity/minecraft.md`)

- All runtime behavior remains intentionally unimplemented → `EXC-MC-01` (explicit exclusion guard).
