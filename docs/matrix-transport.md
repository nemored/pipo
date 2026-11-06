# Matrix transport

PIPO bridges Matrix by acting as a single Matrix **Application Service**
(appservice). Because one bridge can register a user-namespace regex that spans
every service PIPO connects to, remote senders are puppeted as distinct "ghost"
Matrix users (`@pipo_irc_alice`, `@pipo_slack_bob`, ...) rather than relayed
through a single bot account.

- **Inbound** events are pushed to PIPO by the homeserver over HTTP
  (`PUT /_matrix/app/v1/transactions/{txnId}`); PIPO hosts that listener itself.
- **Outbound** messages are sent through the Client-Server API, masquerading as
  the relevant ghost via the appservice `?user_id=` parameter with the
  `as_token` as the access token.

Event content is built and parsed with the community-maintained
[`ruma`](https://github.com/ruma/ruma) types; PIPO does not depend on
`matrix-sdk` (its client-session/sync layer is unused by an appservice, and it
cannot perform the `?user_id=` masquerade).

## v1 scope

Bridged in **both directions**: plain text messages, edits (`m.replace`),
deletes (redactions), and reactions (`m.reaction`).

Deferred: attachments/media and threads (the code marks where they hook in),
and **encrypted rooms** — bridged Matrix rooms must have encryption disabled.

## 1. Registration file (operator-provided)

PIPO does not generate this. Write it, register it with your homeserver, and
copy the two tokens into PIPO's config. Example `pipo-registration.yaml`:

```yaml
id: pipo
url: "http://127.0.0.1:8090"        # must match listen_addr and be reachable by the homeserver
as_token: "REPLACE_WITH_A_LONG_RANDOM_STRING"
hs_token: "REPLACE_WITH_A_DIFFERENT_LONG_RANDOM_STRING"
sender_localpart: "pipo"            # -> @pipo:example.org
rate_limited: false
namespaces:
  users:
    - exclusive: true
      regex: "@pipo_.*"             # ONE regex spanning every bridged service
  aliases: []
  rooms: []
```

On Synapse, reference it from `homeserver.yaml`:

```yaml
app_service_config_files:
  - /path/to/pipo-registration.yaml
```

The single `@pipo_.*` namespace intentionally covers `@pipo_irc_*`,
`@pipo_slack_*`, `@pipo_discord_*`, etc. — the whole point of running PIPO as one
multi-service bridge instead of one bridge per service.

## 2. PIPO config block

Add a transport of type `Matrix` to your PIPO config. Note that the homeserver
is given as a host (and optional port) plus `use_tls`, **not** a full URL —
PIPO's config parser strips `//` as a comment, so a `https://` value cannot be
written directly.

```json
{
  "transport": "Matrix",
  "homeserver": "matrix.example.org",
  "use_tls": true,
  "server_name": "example.org",
  "as_token": "REPLACE_WITH_A_LONG_RANDOM_STRING",
  "hs_token": "REPLACE_WITH_A_DIFFERENT_LONG_RANDOM_STRING",
  "sender_localpart": "pipo",
  "ghost_prefix": "pipo_",
  "listen_addr": "0.0.0.0:8090",
  "channel_mapping": { "!abcdef:example.org": "bus1" }
}
```

| Field | Meaning |
|-------|---------|
| `homeserver` | Homeserver client-server API host, optionally `host:port`. |
| `use_tls` | `true` for `https`, `false` for `http`. |
| `server_name` | The `:domain` half of user/room IDs, e.g. `example.org`. |
| `as_token` / `hs_token` | Must match the registration file exactly. |
| `sender_localpart` | Must equal the registration's `sender_localpart`. |
| `ghost_prefix` | Must be the literal prefix of the users namespace regex (`pipo_` for `@pipo_.*`). |
| `listen_addr` | Address the inbound appservice listener binds to; the registration `url` must reach it. |
| `channel_mapping` | Matrix room ID → PIPO bus id. |

## Operational notes

- The appservice sender user (`@pipo:example.org`) performs redactions for
  bridged deletes, so it should hold a **moderator** power level in bridged
  rooms.
- Ghost users auto-join their mapped rooms. In invite-only rooms the sender user
  must invite the ghost first (not yet automated).
- Bridged rooms must be **unencrypted** in this version.
