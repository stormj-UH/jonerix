# jcarp

<p align="center">
  <img src="assets/jcarp-mascot.png" alt="jcarp mascot" width="240">
</p>

`jcarp` is a Rust port of current OpenBSD CARP behavior for jonerix Linux
userland. OpenBSD's CARP source is BSD-licensed and license-compatible with
jonerix; protocol constants, header layout, HMAC ordering, and election
semantics are intentionally tracked against OpenBSD `ip_carp.c` and
`ip_carp.h`.

The current port tracks OpenBSD `ip_carp.c` rev. 1.372 and `ip_carp.h` rev.
1.52. It includes:

- CARP v2 advertisement header parsing/serialization.
- Internet checksum handling for CARP advertisement payloads.
- OpenBSD-style HMAC-SHA1 preparation over version/type, VHID, sorted IPv4 and
  IPv6 VIPs, and replay counter. CARP passphrases match OpenBSD `ifconfig pass`
  behavior: copy passphrase bytes directly into the 20-byte key buffer,
  zero-padded and truncated.
- IPv6 HMAC compatibility contexts for OpenBSD's scoped link-local handling.
- Election state logic for `INIT`, `BACKUP`, and `MASTER`, including preemption,
  demotion comparison, and master-down timing.
- Linux raw IPv4 and IPv6 protocol-112 sockets. IPv4 uses TTL 255 and
  `224.0.0.18`; IPv6 uses hop-limit 255 and `ff02::12`.
- A daemon receive loop with per-VHID replay windows.
- VIP add/remove lifecycle over rtnetlink for IPv4 and IPv6.
- Gratuitous ARP and unsolicited IPv6 Neighbor Advertisement on non-balancing
  MASTER transitions.
- OpenBSD-style MASTER shutdown bow-out advertisements using `advbase=255` and
  `advskew=255`.
- Parent/effective link-state suppression with dynamic demotion while the link
  is down, plus OpenBSD-style send-error demotion and recovery thresholds.
- OpenBSD-style `carpnodes` with up to 32 `vhid:advskew` entries.
- `balancing=ip|ip-stealth|ip-unicast` control-plane state plus a
  stormwall/nft netdev ingress filter that applies the OpenBSD load-sharing
  source/destination fold hash against the local MASTER bitmask.
- Optional Linux child link modes: `macvlan-bridge`, `macvlan-private`, and
  `ipvlan-l2`. These let `jcarp` put the CARP MAC/VIPs on an effective child
  interface while also suppressing CARP when the parent link is down.

Runtime CARP operation needs `CAP_NET_RAW` for protocol 112 sockets and
`CAP_NET_ADMIN` for VIP lifecycle, neighbor signaling, MAC changes, and
load-sharing ingress rules.

## Configuration

Single-node failover remains compatible with the original shape:

```text
interface=eth1
vhid=42
advbase=1
advskew=50
preempt=true
vip=10.0.253.42/24
passphrase=interop-pass
```

Dual-stack and load-sharing options:

```text
interface=eth1
carpnode=42:50
carpnode=43:0
link_mode=macvlan-bridge
link_name=jcarp42
balancing=ip
load_filter=auto
vip=10.0.253.42/24
vip6=2001:db8:253::42/64
passphrase=interop-pass
mac=interface
```

Important options:

- `vip=`, `vip4=`, and `vip6=` accept optional prefix lengths.
- `peer=` and `peer6=` override the default multicast peers for unicast CARP.
- `manage_vip=false` leaves address add/remove to external configuration.
- `announce=false` suppresses gratuitous ARP/NA.
- `mac=virtual` uses the CARP virtual MAC. `mac=interface` leaves the parent
  interface MAC unchanged; this is usually the safer choice for `ip-stealth` or
  labs where the hypervisor already floods traffic.
- `link_mode=parent|macvlan-bridge|macvlan-private|ipvlan-l2` selects the
  Linux interface strategy. `parent` is the backward-compatible default.
- `link_name=` names the child link for child modes. If omitted, `jcarp<VHID>`
  is used.
- `link_parent=` names the parent for child modes. If omitted, `interface=` is
  the parent.
- `load_filter=auto|nft|off` controls the balancing dataplane hook. In
  balancing modes, `auto` currently uses the jonerix `nft`/stormwall frontend.

## Validation

Forgejo CI runs inside a `ghcr.io/stormj-uh/jonerix:builder` container and must pass
`cargo build --release --locked --bin jcarp` plus the reproducible local test
harness before changes land on `main`.

The reproducible validation package lives in `tools/`:

- `sh tools/jcarp-validate.sh local` runs the unprivileged unit and
  OpenBSD-compatibility integration tests.
- `sudo sh tools/packet-smoke.sh` captures one protocol-112 advertisement and
  checks TTL `255` and destination `224.0.0.18`.
- `sh tools/m4ni-vm-probe.sh` and `sh tools/m4ni-vmnet-bridge-probe.sh` record
  the `m4ni` VM prerequisites for OpenBSD interop testing.
- `tools/jcarp-send-loop.sh` emits repeated `send-once` advertisements for
  OpenBSD failover and `balancing ip` control-plane tests.

On `jonerix-tormenta`, the current static aarch64 release binary is 337 KiB.
Measured with `/usr/bin/time -v`, `jcarp check` peaked at 1088 KiB RSS and
`sudo jcarp send-once` peaked at 2160 KiB RSS. Re-measure steady-state daemon
RSS after each runtime change because VIP management and load-filter updates
exercise privileged kernel paths.

## Linux Notes

OpenBSD implements CARP as a kernel pseudo-interface. `jcarp` runs in Linux
userland, so the port maps those responsibilities onto raw sockets, rtnetlink,
packet sockets, and stormwall/nft ingress rules.

For appliance deployments where changing the physical NIC MAC is risky, prefer
`link_mode=macvlan-bridge` or `link_mode=ipvlan-l2`. `macvlan-bridge` is the
closest Linux mapping for normal CARP L2 behavior; `ipvlan-l2` is useful on
consumer switches or routers that only tolerate one learned MAC on the parent
port. `link_mode=parent` remains available for explicit low-level setups.

For load balancing, OpenBSD assumes the participating hosts all receive the VIP
traffic and then drops non-owned hash slots in the kernel. `jcarp` mirrors the
drop decision with netdev ingress rules, but the network still has to deliver
the traffic to every participating node. `balancing=ip` usually needs multicast
or flooded L2 delivery; `ip-stealth` avoids learning the virtual MAC from CARP
advertisements; `ip-unicast` expects external replication.
