# Pipo Runtime Bundle

This runtime bundle is relocatable. Keep the directory layout intact and run
`bin/pipo_supervisor` from the extracted root, using `etc/pipo/*.json` as
templates for your runtime configuration.

## Supported target triples

Initial supported release artifacts are produced for:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`

Each release includes one tarball per target triple and a per-artifact
`releases/manifest.json` describing the exact target and checksums.
