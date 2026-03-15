# Long-running stability and memory trend validation

Use the burn-in profile to validate router/worker stability under representative
routing traffic over multi-hour runs.

## Burn-in profile

The profile is implemented by `PipoSupervisor.BurnIn` and exposed via:

```bash
cd apps/pipo_supervisor
mix pipo.burn_in --duration-hours 4 --publish-interval-ms 25 --sample-interval-ms 5000
```

### Traffic model

- Continuous `publish` flow on bus `alpha` from a source identity.
- Three subscribers (`stable_a`, `stable_b`, `flaky`).
- The `flaky` worker is periodically forced into transient timeout behavior,
  then restored.

This creates realistic fanout traffic with intermittent worker degradation and
recovery while maintaining message throughput.

## Collected signals

- Per-process memory samples for:
  - `Router`
  - `stable_a`
  - `stable_b`
  - `flaky`
- Delivery and publish counters.
- Degraded/restored transition notifications emitted by the router.

## Pass/fail guardrails

The run fails if any guardrail fails:

1. **No monotonic, unbounded memory growth**
   - For each tracked process, detect monotonic non-decreasing memory series.
   - If monotonic and growth exceeds configured limit (`growth_limit_bytes`),
     fail.

2. **No deadlock/livelock**
   - Require published and delivered counters to continue increasing.
   - Fail if the `last_delivery_at` stall exceeds `max_stall_ms`.

3. **Stable message flow despite transient worker failures**
   - Require a minimum delivery ratio (`min_delivery_ratio`) even with flaky
     worker timeouts.
   - Require both degradation and restoration transitions to be observed.

## Useful options

- `--duration-hours` / `--duration-ms`
- `--publish-interval-ms`
- `--sample-interval-ms`
- `--call-timeout-ms`
- `--growth-limit-mb`
- `--max-stall-ms`
- `--min-delivery-ratio`
