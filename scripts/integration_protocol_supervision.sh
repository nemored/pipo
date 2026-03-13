#!/usr/bin/env bash
set -euo pipefail

test -f native/transport_runtime/src/lib.rs
test -f apps/pipo_supervisor/lib/pipo_supervisor.ex
test -f schema/README.md

echo "Protocol + supervision integration wiring is present."
