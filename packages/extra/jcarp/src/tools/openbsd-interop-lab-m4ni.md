# OpenBSD Interop VM Lab Plan (m4ni, macOS arm64)

This plan builds a reproducible CARP lab where `jcarp` on jonerix is validated
against OpenBSD CARP behavior.

## Host and Network Targets

- Host: `m4ni` (macOS arm64).
- VM hypervisor: Homebrew QEMU with HVF acceleration.
- QEMU tools: `/opt/homebrew/bin/qemu-system-aarch64` and
  `/opt/homebrew/bin/qemu-img`.
- CARP network segment: L2 broadcast domain shared by:
  - OpenBSD VM (`openbsd-carp-a`),
  - jonerix environment running `jcarp` packet tests.

CARP requires one L2 segment that carries multicast traffic for
`224.0.0.18` and IP protocol `112`. QEMU user-mode networking is not enough.
Use QEMU `vmnet-bridged` or another bridged L2 backend.

On `m4ni`, the Homebrew QEMU binary has the Hypervisor entitlement but not
Apple's restricted vmnet entitlement. The reproducible bridge probe therefore
uses `sudo -n` by default when passwordless sudo is available.

## Version Pinning

Record and keep these values with each test run:

- OpenBSD release image filename and SHA256.
- QEMU version.
- `rustc -Vv` and `cargo -V`.
- `jcarp` git commit hash.

Use one OpenBSD release across both VMs for each run. Do not mix snapshots and
stable releases in the same interop matrix.

Collect host state first:

```sh
sh tools/m4ni-vm-probe.sh
sh tools/m4ni-vmnet-bridge-probe.sh
```

## VM Layout

For each OpenBSD VM:

1. 1 vCPU, 1024 MB RAM (or higher if needed for host stability).
2. Virtio NIC attached to the same bridged QEMU network.
3. Static IPv4 on data interface (example: `10.88.0.11/24`, `10.88.0.12/24`).
4. CARP VIP configured with the same VHID/passphrase/VIP used by `jcarp`.

Example OpenBSD `hostname.if` shape (adapt interface names per VM):

```sh
# /etc/hostname.vio0
inet 10.88.0.11 255.255.255.0

# /etc/hostname.carp0
vhid 42 pass interop-pass carpdev vio0 advbase 1 advskew 0 10.88.0.100 255.255.255.0
```

Use a larger OpenBSD `advskew` than the `jcarp` sender to prove OpenBSD backs
down to a more frequent remote advertiser.

Example QEMU network shape, adapted to the real bridge interface on `m4ni`:

```sh
export PATH="/opt/homebrew/bin:$PATH"
sudo -n env PATH="$PATH" qemu-system-aarch64 \
  -machine virt,accel=hvf \
  -cpu host \
  -m 1024 \
  -smp 1 \
  -drive file=openbsd-carp-a.qcow2,if=virtio,format=qcow2 \
  -netdev vmnet-bridged,id=carp0,ifname=en0 \
  -device virtio-net-pci,netdev=carp0,mac=52:54:00:42:00:11
```

On `m4ni`, unprivileged QEMU `vmnet-bridged` returns `cannot create vmnet
interface: general failure (possibly not enough privileges)`. The sudo-backed
probe succeeds and is the expected path for this lab.

## jcarp Node Configuration

Create a dedicated config for interop runs:

```ini
interface=eth0
vhid=42
advbase=1
advskew=50
demote=0
preempt=true
peer=224.0.0.18
vip=10.88.0.100
passphrase=interop-pass
```

Or run the reproducible sender helper directly on the jonerix host:

```sh
JCARP_BIN=/tmp/jcarp-target/release/jcarp \
VHID=42 ADVSKEW=50 VIP=10.0.253.42 COUNT=30 \
brash tools/jcarp-send-loop.sh
```

## Validation Sequence

1. Baseline local reproducibility:
   - `sh tools/jcarp-validate.sh local`
2. Local packet smoke (privileged):
   - `sudo sh tools/packet-smoke.sh`
3. OpenBSD-only baseline:
   - confirm one VM is MASTER and one is BACKUP,
   - capture CARP with `tcpdump -ni <if> proto 112`.
4. Inject `jcarp` packets:
   - run `jcarp send-once` repeatedly from jonerix node,
   - capture on both OpenBSD VMs and jonerix side.
5. Validate interop invariants:
   - protocol is `112`,
   - TTL is always `255`,
   - destination is `224.0.0.18`,
   - OpenBSD `tcpdump` decodes `CARPv2-advertise`,
   - VHID/passphrase-compatible packets are accepted by OpenBSD and affect
     CARP state.
6. Fault simulation:
   - stop MASTER advertisements,
   - verify BACKUP takeover timing from `advbase`/`advskew` expectations,
   - re-enable and observe preemption behavior.

## Load-Balancing Control-Plane Check

OpenBSD CARP supports load balancing with one `carp0` interface containing up
to 32 `carpnodes` and a balancing mode of `ip`, `ip-stealth`, or `ip-unicast`.
This is hash-based CARP load sharing, not a general L4 load balancer.

Configure the OpenBSD VM for two nodes:

```sh
ifconfig carp0 destroy 2>/dev/null || true
ifconfig carp0 create
ifconfig carp0 inet 10.0.253.42 netmask 255.255.0.0 pass interop-pass \
  carpdev vio0 advbase 1 carpnodes 42:100,43:0 balancing ip up
```

Expected OpenBSD baseline after master-down timers expire:

```text
carp: carpdev vio0 advbase 1 balancing ip
        state MASTER vhid 42 advskew 100
        state MASTER vhid 43 advskew 0
```

Advertise only VHID 42 from jonerix:

```sh
JCARP_BIN=/tmp/jcarp-target/release/jcarp \
VHID=42 ADVSKEW=50 VIP=10.0.253.42 COUNT=30 \
brash tools/jcarp-send-loop.sh
```

Expected OpenBSD state while the loop is active:

```text
state BACKUP vhid 42 advskew 100
state MASTER vhid 43 advskew 0
```

Advertise only VHID 43 from jonerix:

```sh
JCARP_BIN=/tmp/jcarp-target/release/jcarp \
VHID=43 ADVSKEW=0 VIP=10.0.253.42 COUNT=30 \
brash tools/jcarp-send-loop.sh
```

Expected OpenBSD state while the loop is active:

```text
state MASTER vhid 42 advskew 100
state BACKUP vhid 43 advskew 0
```

After each sender loop stops, OpenBSD should return to MASTER for the affected
VHID after the master-down interval.

## Observed m4ni Run (2026-05-02)

- OpenBSD artifact: `miniroot78.img`.
- SHA256: `6554b83277360578ccdb47c9a84a82b872a7664f78481f67a60836bba0012444`.
- OpenBSD kernel after install: `OpenBSD 7.8 GENERIC.MP#38 arm64`.
- QEMU: `qemu-system-aarch64` 10.2.1 with HVF and sudo-backed
  `vmnet-bridged`.
- jonerix sender: `jonerix-tormenta` on the same 10.0/16 L2 segment.

Observed failover behavior:

- OpenBSD decoded `jcarp` as `CARPv2-advertise 36: vhid=42 advbase=1
  advskew=50 demote=0 ... ttl 255`.
- OpenBSD transitioned `MASTER -> BACKUP` while corrected `jcarp` advertised
  VHID 42.
- OpenBSD transitioned `BACKUP -> MASTER` after the `jcarp` sender stopped.

Observed `balancing ip` behavior:

- With `carpnodes 42:100,43:0 balancing ip`, OpenBSD became MASTER for both
  VHIDs after baseline timeout.
- Advertising only VHID 42 from `jcarp` moved only VHID 42 to BACKUP.
- Advertising only VHID 43 from `jcarp` moved only VHID 43 to BACKUP.
- After each sender loop stopped, OpenBSD returned the affected VHID to MASTER.

## Evidence Collection

Keep a per-run artifact directory with:

- `tools/m4ni-vm-probe.sh` output,
- `tools/m4ni-vmnet-bridge-probe.sh` output,
- OpenBSD `tcpdump` outputs (both VMs),
- jonerix `tcpdump` output,
- `jcarp` command logs,
- exact configs used for each node,
- timestamp and host (`m4ni`) metadata.

## Current Runtime Coverage

- `jcarp run` now includes the protocol-112 receive loop, per-VHID replay
  windows, IPv4/IPv6 VIP lifecycle, gratuitous ARP, unsolicited IPv6 NA, and
  multi-`carpnode` election state.
- `balancing=ip|ip-stealth|ip-unicast` updates the local MASTER bitmask and,
  unless `load_filter=off`, installs a stormwall/nft netdev ingress filter that
  mirrors OpenBSD `carp_lsdrop()` for IPv4 and IPv6 VIP traffic.
- Linux still depends on the surrounding L2 network to deliver load-shared VIP
  traffic to every node. Verify multicast/flooding behavior before diagnosing
  CARP state.
- Host firewalling or hypervisor network mode can suppress CARP multicast;
  verify L2 multicast reachability before diagnosing protocol behavior.
