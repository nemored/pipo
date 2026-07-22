#!/usr/bin/env bash
# Flip which color's egress is live and forcibly drop the demoted color's
# established connections, so blue and green are never both relaying at once.
set -euo pipefail

COLOR="${1:?usage: promote.sh <blue|green>}"

case "$COLOR" in
  blue)  SUBNET=10.200.1.0/30; DEMOTED_SUBNET=10.200.2.0/30; DEMOTED_ADDR=10.200.2.2 ;;
  green) SUBNET=10.200.2.0/30; DEMOTED_SUBNET=10.200.1.0/30; DEMOTED_ADDR=10.200.1.2 ;;
  *) echo "unknown color: $COLOR (expected blue or green)" >&2; exit 1 ;;
esac

nft flush set inet pipo_gateway active_subnet
nft add element inet pipo_gateway active_subnet "{ $SUBNET }"

# Evict the demoted color's tracked connections in both directions so its
# sockets die immediately instead of lingering until a keepalive fails.
conntrack -D -s "$DEMOTED_ADDR" >/dev/null 2>&1 || true
conntrack -D -d "$DEMOTED_ADDR" >/dev/null 2>&1 || true

mkdir -p /run/pipo
printf '%s' "$COLOR" > /run/pipo/active-color

echo "promoted $COLOR; flushed conntrack entries for demoted subnet $DEMOTED_SUBNET"
