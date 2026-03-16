# Go rewrite deployment and rollback plan

## Deployment stages

### Stage 1: shadow / limited environment
- Deploy the Go binary to a shadow environment that reads production-equivalent traffic but does not become the primary forwarder.
- Keep the Rust binary as the active forwarding path.
- Enable health endpoint (`HEALTH_ADDR`) and collect structured logs for:
  - connect attempts/success/failure per transport,
  - queue-pressure drops,
  - forwarding/session failures,
  - DB operation errors.
- Gate progression on sustained healthy telemetry (no elevated DB errors, stable connect success ratio, acceptable queue pressure rate).

### Stage 2: partial production channels
- Shift a bounded subset of production channels/buses to the Go binary.
- Keep the Rust binary warm and runnable with the same config and DB path.
- Validate parity in message forwarding and ID mapping for selected channels.
- Monitor the same health metrics with SLO thresholds stricter than Stage 1.
- Increase the channel set gradually if metrics stay green and no regressions are observed.

### Stage 3: full replacement
- Promote the Go binary to all production channels.
- Keep Rust binary and deployment manifests available for immediate rollback.
- Continue monitoring health endpoint and structured logs during the first full-release window.
- Declare rollout complete only after a full business cycle with no critical forwarding or DB incidents.

## Observability contract

- Structured logs are emitted in JSON for transport lifecycle, queue pressure, forwarding failures, and DB errors.
- Health metrics are exposed via `/healthz` when `HEALTH_ADDR` is configured.
- `/healthz` returns `status=ok` and counters:
  - `connect_attempts`
  - `connect_successes`
  - `connect_failures`
  - `queue_pressure_drops`
  - `forwarding_failures`
  - `db_errors`

## Rollback procedure (Go -> Rust)

1. **Stop Go traffic ownership**
   - Remove Go from active forwarding (or scale to zero) and restore Rust as active.
2. **Reuse existing DB file**
   - Point Rust to the same SQLite DB path used by Go.
3. **Reuse existing config**
   - Use the same config JSON (or equivalent channel mappings) previously validated in staging.
4. **Do not run destructive migrations**
   - The Go migration path is idempotent and non-destructive (`CREATE TABLE IF NOT EXISTS`, `CREATE TRIGGER IF NOT EXISTS`).
   - Rollback does not depend on schema rewrites or data deletion.
5. **Verify continuity**
   - Confirm Rust starts cleanly and can read/write existing message ID mappings.
   - Confirm forwarding resumes with expected channel routes.

## Continuity guarantees

- **DB continuity:** Go uses Rust-compatible `messages` table semantics; rollback reuses the same DB file.
- **Config continuity:** CLI/env config path handling remains compatible (`CONFIG_PATH` / positional config arg and `DB_PATH` / positional DB arg).
- **No destructive dependency:** No stage or rollback step requires destructive migration.
