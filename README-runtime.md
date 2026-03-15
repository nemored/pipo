# Pipo Runtime Bundle

This runtime bundle is relocatable. Keep the extracted directory layout intact
and launch `bin/pipo_supervisor` from the extraction root.

## Extraction and startup flow

1. Extract the release tarball (`pipo-<version>-<target>.tar.gz`) into a clean
   directory.
2. Verify required runtime members exist:
   - `bin/pipo_supervisor`
   - `bin/pipo-transport`
   - `etc/pipo/config.example.json`
   - `etc/pipo/transports.example.json`
   - `releases/manifest.json`
   - `releases/SHA256SUMS`
   - `README-runtime.md`
3. Copy and adapt your runtime config from `etc/pipo/config.example.json`.
4. Start the supervisor with an explicit config path, for example:

   ```bash
   ./bin/pipo_supervisor --config ./etc/pipo/config.json
   ```

5. On startup, workers launch `pipo-transport`, then wait for a `ready` signal.
   If `require_all_ready` is enabled, supervisor boot fails if any worker does
   not become ready within the configured timeout.

## Configuration format (split runtime + transport catalog)

The runtime bundle uses a **two-file configuration format**:

1. `etc/pipo/config.json` (supervisor/runtime controls)
2. `etc/pipo/transports.json` (transport inventory and per-transport config paths)

This replaces the prior single-file style where runtime and transport entries
were mixed together.

### 1) Supervisor/runtime config (`config.json`)

`config.json` contains process-level behavior and orchestration limits:

- `protocol_version`: protocol contract version (`"v1"`)
- `shutdown_timeout_ms`: worker shutdown grace period
- `ready_timeout_ms`: max wait before a worker is marked degraded
- `require_all_ready`: fail startup if any worker misses readiness timeout
- `restart_intensity_max_restarts` / `restart_intensity_window_seconds`:
  restart-storm circuit breaker
- `backoff_min_ms` / `backoff_max_ms`: worker restart backoff window
- `router_delivery_timeout_ms`: router-to-worker call timeout
- `router_degraded_drop_threshold`: dropped-delivery threshold before degraded

Example:

```json
{
  "protocol_version": "v1",
  "shutdown_timeout_ms": 2000,
  "ready_timeout_ms": 5000,
  "require_all_ready": false,
  "restart_intensity_max_restarts": 3,
  "restart_intensity_window_seconds": 10,
  "backoff_min_ms": 100,
  "backoff_max_ms": 1000,
  "router_delivery_timeout_ms": 200,
  "router_degraded_drop_threshold": 10
}
```

### 2) Transport catalog (`transports.json`)

`transports.json` describes which transport instances exist and where each
transport-specific credential/settings document lives.

Each `transports[]` entry supports:

- `name`: transport selector passed to `pipo-transport --transport`
- `enabled`: whether this transport should be launched
- `instance_id`: stable worker instance identifier for logs/routing
- `subscriptions`: logical buses consumed by the instance
- `config_path`: path to transport-native JSON used by `pipo-transport --config`

Example:

```json
{
  "transports": [
    {
      "name": "slack",
      "enabled": false,
      "instance_id": "slack_0",
      "subscriptions": ["general"],
      "config_path": "./transport.slack.json"
    }
  ]
}
```

### 3) Individual transport config files (`transport.<name>.json`)

Each catalog entry points to its own transport-native config file via
`config_path` (for example `./transport.slack.json`). The worker launches
`pipo-transport --transport <name> --config <config_path>` for each enabled
entry.

Common requirements for every individual transport config file:

- Must be valid JSON (invalid JSON exits with code `2`).
- May contain transport-specific credentials/endpoints/channels as required by
  that transport implementation.
- Optionally may include `startup_failure` with `"auth"`, `"network"`, or
  `"fatal"` to intentionally simulate startup failures for testing.

Template files are provided under `etc/pipo/`:

- `transport.slack.example.json`
- `transport.discord.example.json`
- `transport.irc.example.json`

Slack-style example (`transport.slack.json`):

```json
{
  "token": "xapp-***",
  "bot_token": "xoxb-***",
  "channel_mapping": {
    "C012345": "general"
  }
}
```

Discord-style example (`transport.discord.json`):

```json
{
  "token": "discord-bot-token",
  "guild_id": 123456789012345678,
  "channel_mapping": {
    "987654321098765432": "general"
  }
}
```

IRC-style example (`transport.irc.json`):

```json
{
  "nickname": "pipo",
  "server": "irc.example.net:6697",
  "use_tls": true,
  "img_root": "https://cdn.example.net/pipo",
  "channel_mapping": {
    "#general": "general"
  }
}
```

### Migration note

When moving from older config layouts, split shared runtime options into
`config.json` and keep transport/credential pointers in `transports.json`.
This separation makes global supervision tuning independent from transport
enablement and secrets rotation.

## Environment overrides (binary/config resolution)

### Runtime process resolution

- `PIPO_TRANSPORT_PATH`: forces the transport executable path used by workers.
  This override is checked first.
- If `PIPO_TRANSPORT_PATH` is not set, the runtime falls back to built-in
  release-relative paths for `pipo-transport`.

### Packaging-time path overrides

`packaging/build_tarball.sh` supports these environment overrides when
constructing artifacts:

- `PIPO_SUPERVISOR_BIN`
- `PIPO_TRANSPORT_BIN`
- `RUNTIME_README`
- `CONFIG_EXAMPLE`
- `TRANSPORTS_EXAMPLE`

This is primarily for CI/release assembly and custom staging pipelines.

### Smoke-test overrides

`packaging/smoke_test.sh` supports:

- `SMOKE_READY_TIMEOUT_SECONDS`: readiness wait window (default `15`).
- `SMOKE_RUN_CMD`: custom launch command executed from extraction root.

## Supported target triples

Initial supported release artifacts are produced for:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`

Each release includes one tarball per target triple plus
`releases/manifest.json` with target and checksum metadata.

Target-specific known issues and mitigations are tracked in
`packaging/compatibility_matrix.conf` and are part of release-readiness review.

## Expected smoke-test behavior

`packaging/smoke_test.sh` validates release behavior end-to-end:

- Extracts the tarball.
- Verifies all required files are present.
- Verifies runtime binaries are executable.
- Writes a smoke config with `require_all_ready: true`.
- Starts supervisor (or `SMOKE_RUN_CMD`) and waits for a `ready` signal in logs.
- Fails if supervisor exits before readiness or no readiness signal appears
  before timeout.

### Smoke-test exit codes

- `10`: artifact/layout mismatch (including missing archive or required files)
- `11`: required runtime binary is not executable
- `12`: supervisor exited before readiness was observed
- `13`: readiness timeout or invalid readiness timeout configuration

## Troubleshooting (readiness/auth/restart failures)

### Readiness failures

Symptoms:
- smoke test exits `12` or `13`
- logs show no `ready` frame
- worker reported as `starting` or `degraded`

Checks:
- confirm `bin/pipo-transport` exists and is executable
- verify `PIPO_TRANSPORT_PATH` (if set) points to a valid executable
- verify config points at valid transport settings and credentials
- increase `SMOKE_READY_TIMEOUT_SECONDS` for slow startup environments

### Authentication failures

Transport processes use defined exit semantics. Exit code `10` indicates
transport authentication failure (for example bad token/password/credential).
Treat this as an operator-actionable configuration issue, not a transient crash.
Avoid unbounded retry loops until credentials are corrected.

### Restart-policy failures

- If workers repeatedly crash, supervisor restart intensity limits may be hit,
  causing escalation/failure depending on configured `max_restarts`/
  `max_seconds`.
- Persistent readiness or network failures can look like restart storms.
- Prioritize root-cause fixes (auth, network reachability, config parse) before
  increasing restart thresholds.

### Transport/runtime exit-code reference

Use transport process exit codes to classify failures:

- `0`: orderly shutdown
- `2`: invalid CLI arguments or config parse failure
- `3`: protocol version incompatibility
- `10`: transport authentication failure
- `11`: transport network unreachable at startup
- `12`: transport-specific fatal error (details logged)
- `20`: internal runtime panic

These codes are useful for alert routing and for deciding whether automatic
restart should continue or pause for operator intervention.
