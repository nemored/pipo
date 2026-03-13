# Protocol fixture set (line-delimited JSON)

Each `*.ndjson` file in this directory is canonical test data for a specific frame shape.
All timestamps are RFC3339 UTC with millisecond precision (`...000Z`).

- `ready.ndjson`: outbound ready frame.
- `message.ndjson`: outbound message frames including nullable and omitted `pipo_id` variants.
- `inject_message.ndjson`: inbound inject_message frames with required `pipo_id` and `origin_transport`.
- `lifecycle.ndjson`: lifecycle/status frames (`health`, `warn`, `fatal`, `shutdown`, `health_check`).
- `reload_config_inbound.ndjson`: reserved inbound frame example.
- `reload_config_expected_warn.ndjson`: expected warn response (`unimplemented`).
- `malformed_lines.ndjson`: intentionally malformed lines for negative tests.
