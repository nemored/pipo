# Packaging

- `build_tarball.sh` assembles the runtime tarball as `pipo-<version>-<target>.tar.gz`.
- `smoke_test.sh` extracts a tarball and validates runtime startup behavior.

## Initial supported target triples

The initial post-MVP supported release targets are:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`

CI builds and smoke-tests a tarball for each supported target to keep packaging
and release artifacts aligned with the declared compatibility matrix.

The CI workflow resolves the target matrix from `packaging/compatibility_matrix.conf`
(filtered by `SUPPORTED_TARGET_TRIPLES`) and runs:

- tarball assembly checks per target,
- extraction + smoke checks per target, and
- resilience checks per target (negative-path validation of expected failure modes).

Release publishing is guarded by a `release-readiness` job that requires all
matrix targets to pass all packaging checks (assembly + smoke + resilience),
not just build success.

## Target-specific known issues and mitigations

Track per-target caveats in `packaging/compatibility_matrix.conf` under each
`known_issues` entry (`id`, `summary`, `release_blocking`, `mitigation`).
These notes are versioned with the compatibility matrix and reviewed as part of
release readiness.
