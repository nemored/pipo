# Operational readiness runbooks

This document finalizes operator runbooks used during release readiness and
on-call handoff. It focuses on recovery actions that can be applied quickly
without requiring source-code changes.

## 1) Degraded worker recovery

### Trigger conditions

Use this runbook when one or more workers stay in `degraded` state, oscillate
between `degraded` and `restored`, or when message delivery drops while publish
traffic remains steady.

### Immediate checks (first 5 minutes)

1. Confirm scope:
   - Single worker degraded: likely endpoint/config issue.
   - Multiple workers degraded simultaneously: likely shared dependency
     (network, auth backend, router pressure).
2. Check supervisor logs for worker lifecycle transitions and transport exit
   codes.
3. Validate runtime binary/path assumptions:
   - `PIPO_TRANSPORT_PATH` (if set) resolves to a valid executable.
   - release layout still contains `bin/pipo-transport`.
4. Validate worker credentials and endpoint reachability.

### Recovery procedure

1. **Stabilize blast radius**
   - If one worker is flapping, temporarily disable or isolate its transport
     config to protect healthy workers from restart churn.
2. **Fix root cause**
   - Correct credential, endpoint, TLS, or config parse issues indicated by
     transport exit codes and logs.
3. **Perform controlled restart**
   - Restart only the affected worker path first.
   - Confirm it emits `ready` and starts receiving routed deliveries.
4. **Re-enable normal routing**
   - Restore full config once metrics show sustained recovery.

### Exit criteria

- Worker remains non-degraded for at least 2 consecutive observation windows.
- Delivery counters resume growth and no prolonged `last_delivery_at` stall is
  observed.
- No restart-intensity escalation is occurring in the supervisor.

---

## 2) Handling auth failure exit code 10 without retry storm

Exit code `10` means transport authentication failure and should be treated as
operator-actionable misconfiguration, not a transient failure.

### Goals

- Stop unbounded retries quickly.
- Preserve healthy workers.
- Restore service only after credential validation.

### Procedure

1. **Classify correctly**
   - Confirm process exits with code `10` (not network startup `11` or
     transport fatal `12`).
2. **Contain retries**
   - Pause or isolate the offending worker config path so supervisor restarts do
     not create a credential retry storm.
3. **Fix credentials out-of-band**
   - Rotate or correct secret/token/password.
   - Validate with a single-worker canary before broad re-enable.
4. **Resume gradually**
   - Re-enable one worker instance, verify successful auth + `ready`.
   - Re-enable remaining instances in batches.

### Anti-patterns to avoid

- Increasing restart limits before correcting credentials.
- Simultaneously restarting all workers with unknown auth state.
- Treating exit `10` as a generic transient crash.

### Exit criteria

- No new exit `10` events during a full observation window.
- Ready/delivery signals recover without restart burst behavior.

---

## 3) Interpreting smoke and burn-in metrics

Use smoke tests for startup-readiness gating and burn-in for sustained behavior
validation.

### Smoke metrics interpretation

Smoke test outcomes (from `packaging/smoke_test.sh`):

- `0`: readiness observed successfully.
- `10`: artifact/layout mismatch.
- `11`: required runtime binary not executable.
- `12`: supervisor exited before readiness.
- `13`: readiness timeout / invalid timeout configuration.

Operator guidance:

- `10`/`11` => packaging/release integrity problem (block rollout).
- `12` => crash-loop or fatal startup path (inspect early logs + exit codes).
- `13` => startup slowness or readiness path blocked (check timeout sizing,
  endpoint reachability, and worker auth).

### Burn-in metrics interpretation

Primary signals:

- Memory trend per process (`Router`, stable workers, flaky worker).
- Publish and delivery counters.
- Degraded/restored transitions.

Decision guide:

- **Healthy**:
  - Memory remains non-monotonic or within growth guardrail.
  - Publish + delivery counters keep increasing.
  - Degraded worker restores and delivery ratio stays above threshold.
- **Investigate**:
  - Monotonic memory growth approaching configured limit.
  - Intermittent delivery stalls but eventual recovery.
- **Fail/block**:
  - Guardrail failure for monotonic growth > limit.
  - Stall exceeds `max_stall_ms` (deadlock/livelock risk).
  - Delivery ratio under `min_delivery_ratio` or no restore transitions.

---

## 4) Incident triage paths for router bottleneck symptoms

### Symptom patterns

- Publish counter grows while delivery counter flattens.
- Frequent worker `degraded` transitions during traffic spikes.
- Increased call timeouts to workers and rising delivery stall duration.

### Triage path A: Worker-side bottleneck

Indicators:

- One/few workers degrade disproportionately.
- Router remains responsive for other routes.

Actions:

1. Isolate slow worker(s) and reduce per-worker load.
2. Inspect downstream endpoint latency/timeouts.
3. Recover worker and verify restored transitions + delivery catch-up.

### Triage path B: Router-wide pressure

Indicators:

- Broad delivery slowdown across multiple workers.
- Burn-in stall metrics trend upward globally.

Actions:

1. Reduce ingress publish rate temporarily.
2. Verify runtime host pressure (CPU, memory, scheduler contention).
3. Check for synchronized restart churn creating control-plane overhead.
4. Restore gradually while monitoring delivery ratio and stall metrics.

### Triage path C: Dependency/auth cascade

Indicators:

- Spike in exit code `10` and/or network startup failures.
- Router symptoms coincide with repeated reconnect/auth attempts.

Actions:

1. Apply exit-`10` containment runbook first.
2. Validate shared secrets/identity provider/network paths.
3. Reintroduce workers in controlled batches.

### Escalation matrix

- **SEV-2**: sustained delivery stall with partial traffic impact.
- **SEV-1**: complete delivery outage, or restart/auth storm affecting most
  workers.

Escalate to engineering with:

- recent smoke result + exit code,
- burn-in guardrail deltas,
- affected workers/routes,
- first observed degradation timestamp and mitigation steps applied.
