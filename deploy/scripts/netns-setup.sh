#!/usr/bin/env bash
# Create/destroy the network namespace + veth pair for one pipo color.
# Invoked by pipo-netns-setup@<color>.service (ExecStart=up, ExecStop=down).
set -euo pipefail

ACTION="${1:?usage: netns-setup.sh <up|down> <blue|green>}"
COLOR="${2:?usage: netns-setup.sh <up|down> <blue|green>}"

case "$COLOR" in
  blue)  ID=1 ;;
  green) ID=2 ;;
  *) echo "unknown color: $COLOR (expected blue or green)" >&2; exit 1 ;;
esac

NETNS="pipo-${COLOR}"
VETH_HOST="veth-${COLOR}-h"
VETH_NS="veth-${COLOR}-n"
HOST_ADDR="10.200.${ID}.1/30"
NS_ADDR="10.200.${ID}.2/30"
NS_GW="10.200.${ID}.1"

up() {
  ip netns add "$NETNS"

  ip link add "$VETH_HOST" type veth peer name "$VETH_NS"
  ip link set "$VETH_NS" netns "$NETNS"

  ip addr add "$HOST_ADDR" dev "$VETH_HOST"
  ip link set "$VETH_HOST" up

  ip netns exec "$NETNS" ip addr add "$NS_ADDR" dev "$VETH_NS"
  ip netns exec "$NETNS" ip link set "$VETH_NS" up
  ip netns exec "$NETNS" ip link set lo up
  ip netns exec "$NETNS" ip route add default via "$NS_GW"

  # Allow the host to forward/NAT traffic arriving on this veth.
  sysctl -qw "net.ipv4.conf.${VETH_HOST}.forwarding=1"
}

down() {
  # Deleting either end of a veth pair removes both ends.
  ip link del "$VETH_HOST" 2>/dev/null || true
  ip netns del "$NETNS" 2>/dev/null || true
}

case "$ACTION" in
  up)   up ;;
  down) down ;;
  *) echo "unknown action: $ACTION (expected up or down)" >&2; exit 1 ;;
esac
