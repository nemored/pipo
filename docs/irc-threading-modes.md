# IRC thread presentation modes

Pipo supports configurable IRC thread presentation for each IRC transport block.

## Config keys

- `thread_presentation_mode` (`auto` | `ircv3_only` | `plaintext_only`)
  - `auto` (default): use IRCv3 `+draft/reply` tags when available and resolvable; fallback to plaintext prefixes otherwise.
  - `ircv3_only`: only use IRCv3 reply tags. If unavailable/unresolvable, no plaintext thread prefix is emitted.
  - `plaintext_only`: always emit plaintext thread prefixes; IRCv3 reply tags are never used.
- `thread_excerpt_len` (default: `120`)
  - Max character length for plaintext root excerpts in `↪ reply to ...` prefixes.
- `show_thread_root_marker` (default: `true`)
  - Controls whether root messages include a plaintext thread-start announcement, e.g. `started a new thread: … (reply: >>K7F2)`.

## Migration and operator guidance

These options are optional and backward-compatible.
If omitted, behavior is equivalent to the previous default (`auto` with plaintext fallback and excerpt length 120).

Use `plaintext_only` on legacy networks lacking IRCv3 support when consistent visible context is required.
Use `ircv3_only` when operators want clean messages with no plaintext prefixes and accept that some replies may appear unthreaded on older servers.

## Logging

For each outbound threaded message, Pipo logs the selected mode as one of:

- `ircv3_tag`
- `plaintext_fallback`
- `ircv3_unavailable`

The log line intentionally includes only transport/channel/id metadata and mode, without message content.
