# Go Rewrite Compatibility Spec

This document defines the startup/config/database behaviors that a Go rewrite must preserve to stay wire-compatible with the current Rust implementation.

## 1) Process startup and CLI/env argument semantics

Rust entrypoint flow is:

1. `src/main.rs` runs `pipo::inner_main().await` and prints any returned error to stderr, without propagating a non-zero code itself. 
2. `inner_main` reads positional args first, then environment fallbacks:
   - `config_path = args[1]` or `CONFIG_PATH`
   - `db_path = args[2]` or `DB_PATH`
3. If either path is missing, it prints usage and returns `Ok(())` early.

### Required compatibility behavior

- Accepted positional syntax:
  - `pipo path-to-config.json [path-to-db.sqlite3]`
- Fallbacks:
  - `CONFIG_PATH` is used only when positional config arg is absent.
  - `DB_PATH` is used only when positional DB arg is absent.
- Missing-arg behavior:
  - If config path **or** DB path resolves to `None`, print:
    - `Usage: <argv0-or-pipo> path-to-config.json [path-to-db.sqlite3]`
  - Return successfully (exit code 0) without starting transports.

## 2) Config JSON schema (`ParsedConfig` / `ConfigTransport`)

Top-level JSON object:

```json
{
  "buses": [{ "id": "string" }],
  "transports": [
    { "transport": "... variant-specific fields ..." }
  ]
}
```

Notes:
- The transport enum is `#[serde(tag = "transport")]`, so each transport object is internally tagged by a `transport` discriminator.
- Field names are case-sensitive and match Rust variant names exactly (`IRC`, `Discord`, `Slack`, `Minecraft`, `Mumble`, `Rachni`).
- JSON comments are tolerated because `//...` single-line comments are stripped before deserialization.

### `buses`

- `buses` is required.
- Each entry requires:
  - `id: string`

### Transport variants

#### `IRC`
Required fields:
- `transport: "IRC"`
- `nickname: string`
- `server: string`
- `use_tls: bool`
- `img_root: string`
- `channel_mapping: object<string,string>`

#### `Discord`
Required fields:
- `transport: "Discord"`
- `token: string`
- `guild_id: u64`
- `channel_mapping: object<string,string>`

#### `Slack`
Required fields:
- `transport: "Slack"`
- `token: string`
- `bot_token: string`
- `channel_mapping: object<string,string>`

#### `Minecraft`
Required fields:
- `transport: "Minecraft"`
- `username: string`
- `buses: string[]`

Behavior note:
- Runtime is currently `todo!("Minecraft")`; config still deserializes with this schema.

#### `Mumble`
Required fields:
- `transport: "Mumble"`
- `server: string`
- `nickname: string`
- `password: string | null` (required key, nullable value)
- `client_cert: string | null` (required key, nullable value)
- `server_cert: string | null` (required key, nullable value)
- `channel_mapping: object<string,string>`
- `voice_channel_mapping: object<string,string>`

Optional fields:
- `comment?: string`

#### `Rachni`
Required fields:
- `transport: "Rachni"`
- `server: string`
- `api_key: string`
- `interval: u64`
- `buses: string[]`

## 3) SQLite database contract

On startup, if table `messages` does not exist, Rust creates:

```sql
CREATE TABLE messages (
  id        INTEGER PRIMARY KEY,
  slackid   TEXT,
  discordid INTEGER,
  modtime   DEFAULT (strftime('%Y-%m-%d %H:%M:%S:%s', 'now', 'localtime'))
);

CREATE TRIGGER updatemodtime
BEFORE update ON messages
begin
  update messages
     set modtime = strftime('%Y-%m-%d %H:%M:%S:%s', 'now', 'localtime')
   where id = old.id;
end;
```

### Required compatibility behavior

- Preserve table name and columns exactly:
  - `messages(id, slackid, discordid, modtime)`
- Preserve trigger name and behavior:
  - Trigger `updatemodtime` updates `modtime` whenever a row is updated.

## 4) ID allocation and persistence semantics

### Initial in-memory `pipo_id`

At startup, Rust computes initial ID seed as:
- Query one row from `SELECT id FROM messages ORDER BY modtime DESC`
- Use that `id` when query succeeds, otherwise `0`
- Set in-memory counter to `(that value + 1)`

Important compatibility note:
- Query lacks `LIMIT 1`; implementation relies on `query_row` consuming the first row from the sorted result.
- Selection is by latest `modtime`, **not** by `MAX(id)`.

### Per-message allocation

For inserts in Slack/Discord/IRC/Rachni:
- Read current shared counter value.
- Insert row using that ID (`INSERT OR REPLACE ...`).
- Increment counter by 1.
- If counter exceeds 40000, wrap to 0.

Observed return-value behavior:
- Slack/Discord/IRC return the inserted ID (`ret`, pre-increment).
- Rachni currently returns post-increment value (`*pipo_id`) after insert (off-by-one relative to others). Rewrite should preserve behavior unless intentionally corrected as a breaking change.

## 5) Slack/Discord mapping lookup contract

### Slack-side lookups/updates

- Insert mapping:
  - `INSERT OR REPLACE INTO messages (id, slackid) VALUES (?1, ?2)`
- Update Slack ID by pipo ID:
  - `UPDATE messages SET slackid = ?2 WHERE id = ?1`
- Lookup pipo ID from Slack TS:
  - `SELECT id FROM messages WHERE slackid = ?1`
- Lookup Slack TS from pipo ID:
  - `SELECT slackid FROM messages WHERE id = ?1`
- Cross-map Slack TS from Discord ID:
  - `SELECT slackid FROM messages WHERE discordid = ?1`
- Cross-map Discord ID from Slack TS:
  - `SELECT discordid FROM messages WHERE slackid = ?1`

### Discord-side lookups/updates

- Insert mapping:
  - `INSERT OR REPLACE INTO messages (id, discordid) VALUES (?1, ?2)`
- Lookup pipo ID from Discord message ID:
  - `SELECT id FROM messages WHERE discordid = ?1`
- Update Discord ID by pipo ID:
  - `UPDATE messages SET discordid = ?2 WHERE id = ?1`
- Lookup Discord ID from pipo ID:
  - `SELECT discordid FROM messages WHERE id = ?1`
- Cross-map Discord ID from Slack TS:
  - `SELECT discordid FROM messages WHERE slackid = ?1`

## 6) Acceptance criteria for rewrite

A rewrite is compatible when all of the following hold:

1. Startup argument resolution and zero-exit usage path match section 1.
2. Config accepts/rejects JSON with the same schema and discriminator behavior in section 2.
3. Database bootstrap creates identical `messages` table + `updatemodtime` trigger (section 3).
4. ID seed and increment/wrap semantics match section 4, including latest-`modtime` seed logic.
5. Slack/Discord SQL mapping operations and lookup directions match section 5.
