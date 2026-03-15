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

The CI workflow resolves the target matrix from `SUPPORTED_TARGET_TRIPLES` and
runs both tarball assembly and extraction smoke tests per resolved target.
Release publishing is guarded by the `publish-gate` job, which requires all
upstream CI jobs to succeed.
