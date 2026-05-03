# jcarp Reproducible Validation Harness

This directory provides a POSIX `sh` test harness that validates `jcarp`
behavior beyond unit tests while keeping dependencies explicit.

## Scope

1. Unprivileged local tests (`cargo test --locked --lib --bins --tests`).
2. Privileged packet smoke validation with `tcpdump` when available:
   - IPv4 protocol `112` (CARP),
   - `ttl 255`,
   - destination multicast `224.0.0.18`,
   - daemon runtime smoke can be run with `manage_vip=false`, `announce=false`,
     and `mac=interface` when the host should not mutate addresses or MACs.
3. OpenBSD interoperability VM lab runbook for macOS arm64 host `m4ni`,
   including failover and `balancing ip` checks.
4. Reproducible load-filter checks through the normal unit test suite; generated
   stormwall/nft rules are asserted without requiring root in CI.

## Files

- `jcarp-validate.sh`:
  top-level runner for `local`, `smoke`, or `all`.
- `packet-smoke.sh`:
  root-only packet capture/smoke test.
- `daemon-smoke.sh`:
  root-only runtime loop smoke test with VIP/MAC mutation disabled.
- `jcarp-send-loop.sh`:
  repeated `send-once` sender for OpenBSD interop and failover tests.
- `openbsd-interop-lab-m4ni.md`:
  reproducible interop lab plan.
- `m4ni-vm-probe.sh`:
  read-only probe for VM tooling and QEMU bridge support on `m4ni`.
- `m4ni-vmnet-bridge-probe.sh`:
  non-booting QEMU probe that checks whether `vmnet-bridged` can actually be
  created on `m4ni`. It uses `sudo -n` by default when available because
  macOS requires privileges for QEMU vmnet.

## Dependencies

Required:

- POSIX shell (`/bin/sh`)
- `cargo`

Additional for packet smoke:

- root privileges
- `tcpdump`
- `awk`, `mktemp`, `id`, `sleep`, `kill`

Additional for load-balancing runtime checks:

- jonerix `nft`/stormwall
- `CAP_NET_ADMIN` or root privileges

Additional for the `m4ni` VM probe:

- `ssh`
- login access to `m4ni`

## Usage

From `packages/extra/jcarp/src`:

```sh
sh tools/jcarp-validate.sh local
sh tools/jcarp-validate.sh smoke
sh tools/jcarp-validate.sh daemon
sh tools/jcarp-validate.sh all
```

On a jonerix host where `/bin/sh` is toybox, invoke these with a shell that
supports `set -e`, for example:

```sh
mksh tools/jcarp-validate.sh local
brash tools/jcarp-validate.sh local
```

Packet smoke can also be run directly:

```sh
sudo sh tools/packet-smoke.sh
sudo brash tools/packet-smoke.sh
```

If `tcpdump` is not installed, smoke exits with `skip` and status `0`.

Daemon smoke keeps the daemon running briefly without changing VIPs or MACs:

```sh
sudo INTERFACE=eth0 sh tools/daemon-smoke.sh
sudo INTERFACE=eth0 brash tools/daemon-smoke.sh
```

The interop sender can be run directly on a jonerix host. It writes a temporary
config and sends one CARP advertisement per second by default:

```sh
JCARP_BIN=/path/to/jcarp VHID=42 ADVSKEW=50 COUNT=30 brash tools/jcarp-send-loop.sh
```

For a two-VHID OpenBSD `balancing ip` test, run one loop for each VHID:

```sh
JCARP_BIN=/path/to/jcarp VHID=42 ADVSKEW=50 COUNT=30 brash tools/jcarp-send-loop.sh
JCARP_BIN=/path/to/jcarp VHID=43 ADVSKEW=0 COUNT=30 brash tools/jcarp-send-loop.sh
```

The local harness runs library, binary, and integration tests directly. It does
not run doctests because the jonerix builder image used by Forgejo CI ships
`cargo` and `rustc` without `rustdoc`.

The `m4ni` VM probe is read-only:

```sh
sh tools/m4ni-vm-probe.sh
sh tools/m4ni-vmnet-bridge-probe.sh
```

To force the bridge probe to run unprivileged:

```sh
M4NI_QEMU_USE_SUDO=0 sh tools/m4ni-vmnet-bridge-probe.sh
```
