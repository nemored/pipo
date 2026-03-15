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
