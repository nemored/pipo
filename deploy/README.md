# Blue/green egress gating (systemd + network namespaces)

No containers: blue and green each run as a systemd service inside their
own network namespace, with no default internet route of their own. All
egress passes through the host, which NATs and gates traffic for whichever
color is currently "active" via nftables. This exists to solve one
problem: Discord/Slack/IRC/Mumble connections are long-lived and stateful,
so a naive blue-green switch can leave both colors connected and relaying
every message twice. Only the active color's traffic is ever allowed out.

## Layout

- `scripts/netns-setup.sh` — creates/destroys a color's netns + veth pair.
  Called by `systemd/pipo-netns-setup@.service`.
- `systemd/pipo-netns-setup@.service` — templated oneshot unit, instantiate
  as `@blue` / `@green`.
- `systemd/pipo@.service` — templated unit for the pipo binary itself,
  joins the matching namespace via `NetworkNamespacePath=`. Instantiate as
  `@blue` / `@green`. Requires systemd >= 245.
- `nftables/pipo-gateway.nft` — host-side ruleset: masquerades and forwards
  only the active color's subnet, drops the other.
- `scripts/promote.sh <blue|green>` — flips the active color and forcibly
  evicts the demoted color's conntrack entries so its sockets die
  immediately.

## Prerequisites

- Linux host with `nftables`, `conntrack-tools`, and `iproute2` installed.
- `net.ipv4.ip_forward=1` set on the host.
- systemd >= 245 (for `NetworkNamespacePath=`).

## Install

```
install -Dm755 deploy/scripts/netns-setup.sh /opt/pipo/deploy/scripts/netns-setup.sh
install -Dm755 deploy/scripts/promote.sh /opt/pipo/deploy/scripts/promote.sh
install -Dm644 deploy/systemd/pipo-netns-setup@.service /etc/systemd/system/pipo-netns-setup@.service
install -Dm644 deploy/systemd/pipo@.service /etc/systemd/system/pipo@.service
install -Dm644 deploy/nftables/pipo-gateway.nft /opt/pipo/deploy/nftables/pipo-gateway.nft
```

Add to `/etc/nftables.conf`:

```
include "/opt/pipo/deploy/nftables/pipo-gateway.nft"
```

then `systemctl reload nftables`.

Create per-color config, e.g. `/etc/pipo/blue.env`:

```
CONFIG_PATH=/etc/pipo/blue-config.json
DB_PATH=/var/lib/pipo/blue/pipo.sqlite
```

and `/etc/pipo/green.env` pointing at `green-config.json` /
`/var/lib/pipo/green/pipo.sqlite`. `StateDirectory=pipo/%i` in
`pipo@.service` creates and owns `/var/lib/pipo/<color>` for the
service's dynamic user.

`systemctl daemon-reload` after installing units.

## Runbook

Start blue and promote it (nothing is active by default — the nftables
set starts empty, so neither color can reach the internet until you run
`promote.sh` at least once):

```
systemctl start pipo-netns-setup@blue pipo@blue
/opt/pipo/deploy/scripts/promote.sh blue
```

Deploy green alongside it (still gated closed):

```
systemctl start pipo-netns-setup@green pipo@green
journalctl -u pipo@green -f   # confirm it starts and is waiting/backing off, not crash-looping
```

Cut over:

```
/opt/pipo/deploy/scripts/promote.sh green
journalctl -u pipo@green -f   # confirm it connects to Discord/Slack/IRC/Mumble
journalctl -u pipo@blue -f    # confirm its connections were dropped and it's backing off cleanly
```

Decommission blue once green is confirmed healthy:

```
systemctl stop pipo@blue pipo-netns-setup@blue
```

Rollback (before decommissioning blue) is symmetric: `promote.sh blue`.

## Notes

- Gating happens at L3/L4 (source subnet), so it applies uniformly to
  HTTP, WebSocket, and raw TCP+TLS traffic (Mumble, IRC) with no
  protocol-specific proxy logic and no changes to pipo's own client code.
- Per-request authorization (Slack HMAC verification, Discord/Slack bearer
  tokens, Mumble TLS certs) is unaffected — it stays inside pipo's own
  client code exactly as today. The gateway only ever makes a yes/no
  decision based on which subnet a packet came from.
