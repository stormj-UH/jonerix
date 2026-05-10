# stormwall

Pure-Rust firewall front-end that speaks **three** CLI dialects — Linux
`nft`, OpenBSD `pfctl`, and the legacy `iptables` family
(`iptables` / `ip6tables` / `iptables-save` / `iptables-restore` /
`ip6tables-save` / `ip6tables-restore`) — all dispatching to the same
in-kernel `nf_tables` backend over raw netlink. No `libmnl`, no
`libnftnl`, no C dependencies. Statically links against musl: a single,
zero-dependency binary.

The same `/bin/stormwall` ELF handles every dialect. Which one runs is
selected by the `argv[0]` basename of the invocation, so `/bin/iptables`
(a symlink to `/bin/stormwall`) speaks iptables, `/bin/nft` speaks nft,
`/bin/pfctl` speaks pfctl, and so on. There is no separate executable
per dialect — just symlinks.

**Version 1.1.11** | [Releases](https://castle.great-morpho.ts.net:3000/jonerik/stormwall/releases) | MIT license

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Usage by dialect](#usage-by-dialect)
  - [nft mode](#nft-mode)
  - [iptables / ip6tables mode](#iptables--ip6tables-mode)
  - [iptables-save / iptables-restore](#iptables-save--iptables-restore)
  - [pfctl mode](#pfctl-mode)
- [Inspecting kernel state](#inspecting-kernel-state)
- [Environment variables](#environment-variables)
- [Required kernel modules](#required-kernel-modules)
- [Drop-in compatibility for tools that probe `iptables --version`](#drop-in-compatibility-for-tools-that-probe-iptables---version)
- [Multi-mode dispatch / symlink layout](#multi-mode-dispatch--symlink-layout)
- [Common patterns](#common-patterns)
- [Debugging](#debugging)
- [nft compatibility](#nft-compatibility)
- [iptables compatibility](#iptables-compatibility)
- [pf.conf translation](#pfconf-translation)
- [pfSense Docker container](#pfsense-docker-container)
- [Architecture](#architecture)
- [Test suites](#test-suites)
- [Performance](#performance)
- [Build from source](#build-from-source)
- [Known gaps](#known-gaps)
- [License](#license)

---

## Install

### One-liner (any Linux)

Auto-detects arch, downloads the latest published `.jpkg`, drops
`bin/stormwall` and `bin/pfctl` (plus license and man pages, when shipped)
under `/usr/local`. **No system paths touched** by default —
`/usr/sbin/iptables`, `/sbin/iptables`, `/etc/`, etc. are all left alone:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/stormwall/install.sh | sh
```

When run interactively, the script asks once whether to also create the
`nft` / `iptables` / `ip6tables` / `iptables-{save,restore}` /
`ip6tables-{save,restore}` dispatch symlinks under `$PREFIX/bin`. Default
answer is **no** — answer `y` to opt in, or pass `--with-symlinks`
non-interactively. Either way, only `$PREFIX/bin` is touched; the
system's own `/usr/sbin/iptables` is never replaced.

The script is **strict POSIX shell** — validated with `dash -n`, `mksh -n`,
and `shellcheck -s sh`. Works under `dash`, `busybox ash`, `mksh`, `bash`.
Required tools on the target host: `curl` or `wget`, `zstd`, `tar`, `od`,
`dd`, `install`. Distro-specific install hints are printed if any are
missing. No root needed if you `--prefix` somewhere writable.

| Flag                   | Default       | Effect                                                                  |
|------------------------|---------------|-------------------------------------------------------------------------|
| `--prefix DIR`         | `/usr/local`  | install under `$DIR/bin` and `$DIR/share`                                |
| `--version VER`        | `1.1.11`      | published `.jpkg` version to fetch                                       |
| `--arch ARCH`          | `uname -m`    | override autodetect; `aarch64` or `x86_64`                              |
| `--with-symlinks`      | off           | also create dispatch symlinks for `nft`, `iptables`, `iptables-save`, `iptables-restore`, `ip6tables`, `ip6tables-save`, `ip6tables-restore` under `$PREFIX/bin` |
| `--no-symlinks`        | (default)     | explicit scriptable opt-out                                             |
| `--no-prompt`, `--yes` | off           | non-interactive; honor flags only                                       |
| `-h`, `--help`         |               | show full usage                                                          |

Long-form `--key=value` is accepted for every flag that takes an argument.

```sh
# Install into ~/.local instead of /usr/local
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/stormwall/install.sh \
  | sh -s -- --prefix "$HOME/.local"

# Pin a specific version
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/stormwall/install.sh \
  | sh -s -- --version 1.1.11

# Opt in to nft/iptables/... dispatch symlinks (no prompt)
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/core/stormwall/install.sh \
  | sh -s -- --with-symlinks --no-prompt
```

### jonerix users

You don't need the script. Use `jpkg`:

```sh
jpkg install stormwall
```

The recipe ships `iptables` / `ip6tables` / `nft` / `pfctl` symlinks under
`/bin` plus `/usr/sbin/` and `/sbin/` compat symlinks so dockerd /
tailscaled / firewalld / NetworkManager all find a working `iptables` on
their hard-coded paths.

### Where the bits come from

- **install.sh source:** mirrored from this repo into [`stormj-UH/jonerix`](https://github.com/stormj-UH/jonerix/blob/main/packages/core/stormwall/install.sh) (canonical at `castle.great-morpho.ts.net` is private Tailscale).
- **`.jpkg` packages:** mirrored to GitHub Releases at [`stormj-UH/jonerix`](https://github.com/stormj-UH/jonerix/releases/tag/packages) — the script downloads `stormwall-<VERSION>-<ARCH>.jpkg` from there directly, no auth required.

---

## Quick start

```sh
# nft mode — same syntax as the upstream nft CLI
sudo stormwall list ruleset
sudo stormwall add table inet filter
sudo stormwall 'add chain inet filter input { type filter hook input priority filter ; policy drop ; }'
sudo stormwall add rule inet filter input ct state established,related accept
sudo stormwall add rule inet filter input tcp dport 22 accept

# iptables mode (after `--with-symlinks` install or via the jonerix recipe)
sudo iptables -L
sudo iptables -A INPUT -p tcp --dport 22 -j ACCEPT
sudo iptables-save > /etc/iptables/rules.v4

# pfctl mode
sudo pfctl -sr                   # show rules
sudo pfctl -f /etc/pf.conf       # apply

# Apply a multi-rule batch from a file (nft mode)
sudo stormwall -f /etc/nftables/firewall.nft

# Same, dry-run only — parser + lower validation, no kernel apply
sudo stormwall -c -f /etc/nftables/firewall.nft

# pf-mode dry run, no root needed (parser-only)
stormwall --pf --dump-exprs -f /etc/pf.conf
```

---

## Usage by dialect

### nft mode

The default dispatch when invoked as `stormwall` or `nft`. Mirrors the
`nft(8)` command surface.

```
Usage: stormwall [ options ] [ cmds... ]

Options:
  -h, --help                Show this help
  -v, --version             Show version information
  -V                        Extended version / capability info
  -f, --file <filename>     Read input from <filename> (use - for stdin)
  -i, --interactive         Read commands from an interactive REPL
  -c, --check               Check commands only; do not apply to kernel
  -a, --handle              Show object handles in output
  -s, --stateless           Omit stateful counters / byte counts
  -n, --numeric             Fully numeric output (addresses + protocols)
  -y, --numeric-priority    Numeric chain priorities
  -p, --numeric-protocol    Numeric layer-4 protocol numbers
  -j, --json                JSON output for list operations
  -t, --terse               Omit set element content in output
  -e, --echo                Echo the ruleset after a change
  -D, --define NAME=VALUE   Set a preprocessor variable
  -I, --include-path <dir>  Add <dir> to the include search path

pf mode (when --pf is given or invoked as pfctl):
  --pf                      Parse input as pf.conf(5) instead of nft
  --dump-exprs              Parse only; dump netlink IR (for testing)
```

**Examples:**

```sh
# Add a table + chain + rule
sudo stormwall add table ip filter
sudo stormwall 'add chain ip filter input { type filter hook input priority filter ; policy accept ; }'
sudo stormwall add rule ip filter input tcp dport 22 accept

# List
sudo stormwall list ruleset
sudo stormwall list table ip filter
sudo stormwall list chain ip filter input

# Atomic batch from file
sudo stormwall -f /etc/nftables/firewall.nft

# Dry-run a batch (parser + lower validation, no kernel apply)
sudo stormwall -c -f /etc/nftables/firewall.nft

# JSON output for tooling
sudo stormwall -j list ruleset

# Interactive REPL
sudo stormwall -i
```

### iptables / ip6tables mode

Activated when invoked as `iptables`, `ip6tables`, or any of the
`*-save` / `*-restore` variants. Mirrors the `iptables(8)` command
surface. See [iptables compatibility](#iptables-compatibility) below for
the full operation/match/target surface.

```sh
# Append a rule
sudo iptables -A INPUT -p tcp --dport 22 -m conntrack --ctstate NEW -j ACCEPT

# Insert at position 1
sudo iptables -I INPUT 1 -i lo -j ACCEPT

# Delete a rule (by spec — see compat below for limitations)
sudo iptables -D INPUT -p tcp --dport 22 -j ACCEPT

# Replace rule N
sudo iptables -R INPUT 3 -p tcp --dport 22 -j DROP

# Rename a user chain
sudo iptables -E OLDNAME NEWNAME

# Default policy on a built-in
sudo iptables -P FORWARD DROP

# Create / delete user chains
sudo iptables -N MYCHAIN
sudo iptables -F MYCHAIN
sudo iptables -X MYCHAIN
sudo iptables -X            # delete every empty user chain in this table

# Zero counters
sudo iptables -Z

# Test syntax + matching without applying
sudo iptables -C INPUT -p tcp --dport 22 -j ACCEPT && echo "rule exists"

# IPv6 — same surface, different family
sudo ip6tables -A INPUT -s fe80::/10 -j DROP
```

The `-w` / `-W` xtables-lock flags are accepted for compatibility (no-op).
`-4`/`--ipv4` and `-6`/`--ipv6` are accepted; family is normally inferred
from the binary name.

### iptables-save / iptables-restore

```sh
# Dump filter + nat + mangle + raw + security
sudo iptables-save > /etc/iptables/rules.v4

# Or scope to one table
sudo iptables-save -t nat

# Restore atomically (single nft batch under the hood)
sudo iptables-restore < /etc/iptables/rules.v4

# Restore but parse-validate only
sudo iptables-restore --test < /etc/iptables/rules.v4

# Don't flush before restore (additive; default is flush-then-restore)
sudo iptables-restore -n < /etc/iptables/rules.v4
```

Note: rule listing currently emits chain headers and `COMMIT` only — the
nft → iptables-save inverse renderer is a known limitation. The forward
path (`iptables -A` etc.) is fully covered.

### pfctl mode

Activated when invoked as `pfctl`. FreeBSD-compatible `pfctl(8)` shim
that parses `pf.conf(5)` and lowers it to the same nf_tables backend.

```
usage: pfctl [-f file] [-nf file] [-e|-d] [-D] [-sr|-sn|-ss|-si|-sa|-sT|-st|-sm]
             [-t table -T show|add|delete|flush|kill [addr ...]]
             [-F states|Sources|all] [-k host] [-M -k label -k str]
             [-a anchor -f file] [-o basic] [-q] [-v] [--version]

  -f file        Load and apply pf rules from file
  -nf file       Parse-check only (do not apply)
  -e / -d        Enable / disable the packet filter
  -D             Daemon mode: stay resident, SIGHUP reload, SIGTERM exit
                 PID written to /var/run/pfctl.pid (override: PFCTL_PID_FILE)
  -s<x>          Show: r=rules n=NAT s=state i=info a=all T=tables t=timeouts
                       m=memory, I=interfaces, q=queues
  -F <what>      Flush: states, Sources, all
  -t T -T <cmd>  Table operations: show, add, delete, flush, kill
  -k host        Kill states matching host
  -q             Quiet — suppress non-error output
  -v             Verbose (repeat for more detail)
```

```sh
# Apply a pf.conf
sudo pfctl -f /etc/pf.conf

# Parse-check only
sudo pfctl -nf /etc/pf.conf

# Show running rules (translated back to pf.conf form)
sudo pfctl -sr

# Show NAT rules
sudo pfctl -sn

# Show all (rules + NAT + state + info + tables + timeouts)
sudo pfctl -sa

# Daemon mode for SIGHUP-reload service patterns
sudo pfctl -D -f /etc/pf.conf
# kill -HUP $(cat /var/run/pfctl.pid)
```

---

## Inspecting kernel state

stormwall is a thin layer over the real `nf_tables` kernel state.
**Anything installed via any of its dialects is visible to all of them.**
A rule added with `iptables -A INPUT ...` shows up in `nft list ruleset`,
in `pfctl -sr` (after translation), and in `iptables-save`. The kernel
holds a single source of truth.

### Quick state dump (any dialect)

```sh
# Human-readable, every table / chain / rule / set / map / object
sudo stormwall list ruleset

# As nft text — same view
sudo nft list ruleset

# JSON, for jq / tooling
sudo stormwall -j list ruleset

# As iptables-save dump
sudo iptables-save
sudo ip6tables-save
```

### Scoped views

```sh
# One table
sudo nft list table ip filter
sudo nft list table inet ts-input          # tailscale's hook table
sudo nft list table ip mangle              # connmark / TOS rewrites

# One chain
sudo nft list chain ip filter input
sudo nft list chain ip nat postrouting

# Just the chain definitions (no rules)
sudo nft list chains

# All tables across all families (ip, ip6, inet, bridge, netdev, arp)
sudo nft list tables

# Rule handles (so you can `delete rule ... handle N`)
sudo stormwall -a list ruleset
```

### Stateful object inspection

```sh
# Show counters / quotas / limits / connlimit objects
sudo nft list counters
sudo nft list limits
sudo nft list quotas
sudo nft list synproxys

# Show named sets and their elements
sudo nft list sets
sudo nft list set ip filter blocked-ips

# Show meters (per-key rate limits)
sudo nft list meters

# Show flowtables (offload acceleration)
sudo nft list flowtables
```

### iptables-style listing

```sh
# All chains in the filter table, with packet/byte counters
sudo iptables -L -v

# Numeric (don't resolve IPs / ports)
sudo iptables -L -n

# Combined: numeric + verbose + line numbers
sudo iptables -L -n -v --line-numbers

# One chain
sudo iptables -L FORWARD -n -v

# Show rules in restore format (machine-readable)
sudo iptables -S
sudo iptables -S FORWARD

# Verify a specific rule exists (exit code 0 = present)
sudo iptables -C INPUT -p tcp --dport 22 -j ACCEPT && echo yes || echo no

# Per-table listing
sudo iptables -t nat -L POSTROUTING -n -v
sudo iptables -t mangle -L
sudo iptables -t raw -L
```

### pfctl-style listing

```sh
# Rules (filter, in pf.conf syntax)
sudo pfctl -sr

# NAT rules
sudo pfctl -sn

# Active state table (conntrack)
sudo pfctl -ss

# Interface list
sudo pfctl -sI

# Tables and their addresses
sudo pfctl -sT
sudo pfctl -t my_block_list -T show

# Everything at once
sudo pfctl -sa
```

### Compact one-liners for monitoring

```sh
# Count rules per chain
sudo nft -j list ruleset | jq '.nftables[] | select(.rule) | .rule.chain' | sort | uniq -c

# Tail packet drops (when a chain has DROP rules with counters)
watch -n 1 'sudo nft list table ip filter | grep -A1 "drop"'

# What chains are hooked at INPUT priority filter (0)?
sudo nft list ruleset | grep -B1 'hook input priority filter'

# All tables a process installed (e.g. tailscaled installs to inet ts-input)
sudo nft list tables | grep ts-

# Quick health check — every chain that's hooked plus its table
sudo nft -a list ruleset | grep -E 'hook|table '

# Top-N rules by packet count (filter table)
sudo nft -j list ruleset | \
  jq -r '.nftables[] | select(.rule.expr[]?.counter) |
         "\(.rule.expr[].counter.packets // 0)\t\(.rule.chain)\t\(.rule.handle)"' \
  | sort -rn | head
```

### Counter / state reset

```sh
# Zero packet/byte counters in a chain (iptables-style)
sudo iptables -Z FORWARD

# Or per-rule via nft
sudo nft 'reset counters chain ip filter forward'

# Reset all counters in a table
sudo nft 'reset counters table ip filter'

# Reset a single named counter
sudo nft 'reset counter ip filter dropped'

# Clear conntrack state (forces re-evaluation; needs conntrack-tools)
sudo conntrack -F
```

### What stormwall version are you running?

```sh
# Default — self-identify as stormwall
stormwall --version
# stormwall 1.1.11

# Extended — capability summary plus version
stormwall -V

# When invoked as iptables, the test-mode dispatch flips on
# byte-identical impersonation (so version-probe regexes match)
iptables --version
# iptables v1.8.10 (nf_tables)

nft --version
# nftables v1.1.3 (Commodore Bullmoose)
```

---

## Environment variables

| Variable                | Values                                  | Effect                                                                                                                                                                                                                                                                                                                                                                |
|-------------------------|-----------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `STORMWALL_TEST_MODE`   | unset / `1` / `nft` / `iptables`         | Impersonate upstream version strings on `--version`. *unset* (default): self-identify as stormwall. *`1`* or *`nft`*: emit byte-identical strings to upstream `nft 1.1.3`. *`iptables`*: emit `iptables v1.8.10 (nf_tables)` for tools (tailscaled, firewalld, NetworkManager, dockerd libnetwork) that pattern-match `iptables\s+v(\d+)\.(\d+)\.(\d+)`. The iptables dispatch path automatically sets `STORMWALL_TEST_MODE=iptables` when unset, so consumers parsing `/usr/sbin/iptables --version` get the right byte sequence with no configuration. |
| `STORMWALL_NLDUMP`      | path                                     | When set, **every outgoing netlink batch is appended to `<path>` as a hex dump** (one record per batch, prefixed with `---`, 16 bytes per row), and the actual `sendto(2)` is short-circuited so the call returns success without touching the kernel. Used by the pf-parity harness to diff wire bytes against upstream `nft --debug=mnl` captures. Useful for byte-comparing what a specific iptables flag set produces vs upstream. |
| `STORMWALL_PF_CBQ_FALLBACK` | `hfsc` / `error`                     | pf-mode CBQ qdisc fallback strategy when the kernel rejected CBQ (Linux 6.5+ removed it). `hfsc` (default): silently rewrite to HFSC. `error`: surface as an error and refuse to apply.                                                                                                                                                                                |
| `PFCTL_PID_FILE`        | path                                     | Override default `/var/run/pfctl.pid` location for `pfctl -D` daemon mode.                                                                                                                                                                                                                                                                                              |

---

## Required kernel modules

stormwall only does the netlink dance. The actual rule semantics depend
on kernel modules that may not be loaded by default. Most auto-load on
demand on stock kernels (Debian, Alpine, Ubuntu, jonerix-with-pre_install).
On bare configurations or container hosts you may need to `modprobe`
them.

| Module                                       | Required for                                                                                              |
|----------------------------------------------|-----------------------------------------------------------------------------------------------------------|
| `nf_tables`                                  | every nft-family rule (usually built-in or autoloaded)                                                     |
| `nft_reject` + `nft_reject_ipv4` / `_ipv6`   | `iptables -j REJECT` (any form). Without these the kernel returns `ENOENT` at NEWRULE and stormwall surfaces it as `iptables: No such file or directory`. |
| `nft_log`                                    | `iptables -j LOG`                                                                                          |
| `nft_limit`                                  | `iptables -m limit`                                                                                        |
| `nft_redir`                                  | `iptables -j REDIRECT`                                                                                     |
| `nft_queue`                                  | `iptables -j NFQUEUE`                                                                                      |
| `nft_ct`                                     | `iptables -j CT`, `-m connmark`, `-m conntrack`                                                            |
| `nft_connlimit`                              | `iptables -m connlimit`                                                                                    |
| `nft_hash` / `nft_numgen`                    | `iptables -m hashlimit`, `-m statistic --mode random`                                                      |
| `nft_meta` / `nft_socket` / `nft_fib`        | meta keys, `-m socket`, `-m rpfilter`                                                                      |
| `bridge` + `br_netfilter`                    | bridged-frame visibility for forwarding rules                                                              |
| `overlay`                                    | dockerd's overlay2 storage driver                                                                          |
| `veth`                                       | container ↔ host interface pair                                                                            |

The `extra/docker` jonerix package's `pre_install` hook prompts to load
the bridge / br_netfilter / veth / overlay / nft_reject / nft_log /
nft_limit / nft_redir modules at install time. For tailscaled / dockerd /
kubernetes hosts, run all of the above once at boot.

---

## Drop-in compatibility for tools that probe `iptables --version`

Several daemons run `iptables --version` and parse the output to decide
whether to use the legacy or nft-backend command set. With
`STORMWALL_TEST_MODE=iptables` (auto-set when invoked as `iptables`),
stormwall returns:

```
iptables v1.8.10 (nf_tables)
```

— the byte-identical format the upstream nf_tables variant of `iptables`
emits. Tools that work out of the box on stormwall:

- **tailscaled** — Tailscale daemon. Probes for the version regex and
  refuses to start if it can't extract `(\d+)\.(\d+)\.(\d+)`. Installs
  rules to `inet ts-input` / `inet ts-forward` / `mangle ts-postrouting`.
- **dockerd / libnetwork** — bridge driver shells out to `iptables` /
  `iptables-save` / `ip6tables`. Hard-codes `/usr/sbin/iptables` and
  `/sbin/iptables` rather than `PATH` lookup, so install the symlinks.
- **firewalld** — backend probe + rule ops. Zone/service/rich-rule
  operations all route through nft.
- **NetworkManager** — connectivity-check and shared-connection NAT.
- **kubelet** / **kube-proxy** — iptables-mode service routing.
- **fail2ban** — ban/unban via the nftables backend.
- **ufw** — uncomplicated-firewall consumer compat.

For dockerd specifically, see also the per-package documentation in
[`packages/extra/docker/`](https://github.com/stormj-UH/jonerix/tree/main/packages/extra/docker)
in the jonerix monorepo — its `pre_install` hook handles the kernel
modules and hard-coded `/usr/sbin/iptables` / `/sbin/iptables` paths
that libnetwork uses instead of `PATH`.

---

## Multi-mode dispatch / symlink layout

```
/bin/stormwall                    real binary (~1.3 MB ELF, static-musl)
/bin/iptables           → stormwall   (filter/nat/mangle/raw/security; ip family)
/bin/iptables-save      → stormwall   (dump current ruleset in iptables-save form)
/bin/iptables-restore   → stormwall   (atomic restore from a save dump)
/bin/ip6tables          → stormwall   (same as iptables but ip6 family)
/bin/ip6tables-save     → stormwall
/bin/ip6tables-restore  → stormwall
/bin/nft                → stormwall   (nft(1) command surface)
/bin/pfctl              → stormwall   (FreeBSD pfctl(8) shim)
/sbin/iptables          → /bin/stormwall   (libnetwork hard-coded path)
/sbin/iptables-save     → /bin/stormwall
/sbin/iptables-restore  → /bin/stormwall
/sbin/ip6tables         → /bin/stormwall
/sbin/ip6tables-save    → /bin/stormwall
/sbin/ip6tables-restore → /bin/stormwall
/sbin/nft               → /bin/stormwall
/usr/sbin/iptables      → /bin/stormwall   (tailscaled hard-coded path)
/usr/sbin/iptables-save → /bin/stormwall
... (full set under /usr/sbin too)
```

The argv[0] dispatch happens at the top of `main()`. The first thing
stormwall does is inspect its own basename and route into the matching
dialect's parser / lowerer. **There is no sub-process spawn** —
everything stays in the same process, so a single rule batch is a
single set of netlink syscalls regardless of which dialect was invoked.

---

## Common patterns

### Open SSH on port 22 and accept established conntrack

```sh
sudo iptables -A INPUT -i lo -j ACCEPT
sudo iptables -A INPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
sudo iptables -A INPUT -p tcp --dport 22 -j ACCEPT
sudo iptables -P INPUT DROP
```

### Port-forward 80 → 8080 (Docker-style)

```sh
sudo iptables -t nat -A PREROUTING -p tcp --dport 80 -j REDIRECT --to-ports 8080
sudo iptables -t nat -A OUTPUT -d 127.0.0.1 -p tcp --dport 80 -j REDIRECT --to-ports 8080
```

### Connmark slicing (tailscale-style)

```sh
# Restore from ct mark on prerouting
sudo iptables -t mangle -A PREROUTING \
    -m conntrack --ctstate ESTABLISHED,RELATED \
    -j CONNMARK --restore-mark --nfmask 0xff0000 --ctmask 0xff0000

# Save to ct mark on output (for new connections we marked ourselves)
sudo iptables -t mangle -A OUTPUT \
    -m conntrack --ctstate NEW -m mark --mark 0x10000 \
    -j CONNMARK --save-mark --nfmask 0xff0000 --ctmask 0xff0000
```

### Rate-limit SSH brute-force (per-source)

```sh
# Allow only 4 connection attempts per minute, per source IP
sudo iptables -A INPUT -p tcp --dport 22 \
    -m recent --rcheck --seconds 60 --hitcount 4 --name SSHBRUTE -j DROP
sudo iptables -A INPUT -p tcp --dport 22 \
    -m recent --set --name SSHBRUTE -j ACCEPT
```

### Hashlimit per-IP rate cap

```sh
# 10 connections per second per source IP, burst 20
sudo iptables -A INPUT -p tcp --syn \
    -m hashlimit --hashlimit-upto 10/sec --hashlimit-burst 20 \
    --hashlimit-mode srcip --hashlimit-name http_rate \
    -j ACCEPT
```

### MASQUERADE behind a NAT box

```sh
sudo iptables -t nat -A POSTROUTING -o eth0 -j MASQUERADE
sudo sysctl -w net.ipv4.ip_forward=1
```

### Time-of-day match (business hours only)

```sh
sudo iptables -A INPUT -p tcp --dport 8080 \
    -m time --timestart 09:00 --timestop 17:00 --weekdays Mon,Tue,Wed,Thu,Fri \
    -j ACCEPT
```

### IPv6 link-local DROP + reverse-path filter

```sh
sudo ip6tables -A INPUT -s fe80::/10 -j DROP
sudo ip6tables -A INPUT -m rpfilter --invert -j DROP
```

### TCP MSS clamp on outgoing forwards (tunnel use case)

```sh
sudo iptables -t mangle -A FORWARD -p tcp --tcp-flags SYN,RST SYN \
    -j TCPMSS --clamp-mss-to-pmtu
```

### Logging dropped packets at a low rate

```sh
sudo iptables -A INPUT -m limit --limit 5/min --limit-burst 10 \
    -j LOG --log-prefix "DROP: " --log-level 4
sudo iptables -A INPUT -j DROP
```

### IPSec policy match (only allow ESP-protected traffic)

```sh
sudo iptables -A FORWARD -m policy --dir in --pol ipsec --proto esp \
    -j ACCEPT
sudo iptables -A FORWARD -m policy --dir in --pol none -j DROP
```

---

## Debugging

### Verify a rule installed

```sh
# Direct check — exit 0 if a matching rule exists in the chain
sudo iptables -C INPUT -p tcp --dport 22 -j ACCEPT && echo found

# What's actually in the kernel?
sudo nft list table ip filter
```

### Dump the netlink wire bytes

```sh
# Append every NLM_F_REQUEST batch as a hex dump to /tmp/nl.log;
# the actual sendto(2) is short-circuited so this works without root
# and without a working nf_tables module.
STORMWALL_NLDUMP=/tmp/nl.log iptables -A INPUT -p tcp --dport 22 -j ACCEPT
xxd -r -p < /tmp/nl.log | hexdump -C | head      # quick visual decode
```

This is the deepest level of debug — useful for reverse-engineering what
a specific iptables flag set actually sends, or for byte-comparing
against upstream nft's output (`nft --debug=mnl` for the upstream side).
The `tests/pf-parity.sh` harness does exactly this comparison, offline.

### Common errors

| Error                                                                | Cause                                                                                | Fix                                                              |
|----------------------------------------------------------------------|--------------------------------------------------------------------------------------|------------------------------------------------------------------|
| `iptables: No such file or directory (os error 2)`                   | A required `nft_*` kernel module isn't loaded                                        | `modprobe nft_reject nft_reject_ipv4` (or whichever)             |
| `iptables: No such file or directory` on `-A FORWARD -j X`           | Jumping to a user chain that doesn't exist                                            | Either `-N X` first, or rely on the auto-create (1.1.2+)         |
| `iptables: Value too large for data type (os error 75)`              | A `--reject-with` kind that the kernel doesn't accept on this family                  | Try a different kind, or drop `--reject-with`                    |
| `iptables: Invalid argument (os error 22)`                           | Family/table mismatch (e.g. nat-only target on filter), bad mask, unsupported expr   | Check the `nft list ruleset` output for what was actually applied |
| `internal nft synthesis failed`                                      | The lower-text round-trip parser doesn't recognise something stormwall just emitted   | File a bug — paste the rule + the error                          |
| `cannot open netlink: Permission denied`                             | Running unprivileged. Netlink + NETFILTER requires `CAP_NET_ADMIN`                   | Run via `sudo` or grant the capability                           |
| `Chain '<X>' already exists (built-in)`                              | `-N` against `INPUT` / `FORWARD` / etc.                                              | Built-ins; don't try to `-N` them                                |
| `cannot allocate memory: Cannot allocate memory (os error 12)`       | Set / map exceeds kernel `nft_set_size` limit                                        | Split the set, or raise the limit at module load                 |
| `failed to extract iptables version from [stormwall iptables ...]`   | Consumer (tailscaled, firewalld) parsing version output of a binary not invoked as `iptables` | Make sure the symlink layout above is in place                |

### Live trace what dockerd / tailscaled is doing

```sh
# Safe iptables tracer (no fork bomb risk: exec -a with absolute target)
sudo tee /usr/local/bin/iptables-trace.sh >/dev/null <<'EOF'
#!/bin/sh
printf "[%s] %s\n" "$(date +%H:%M:%S.%N)" "$*" >> /var/log/iptables-trace.log
exec -a iptables /bin/stormwall "$@"
EOF
sudo chmod +x /usr/local/bin/iptables-trace.sh
sudo touch /var/log/iptables-trace.log

# Point /usr/sbin/iptables at the tracer (keep the original symlink as backup)
sudo cp -d /usr/sbin/iptables /usr/sbin/iptables.bak
sudo ln -sf /usr/local/bin/iptables-trace.sh /usr/sbin/iptables

# Restart the daemon you want to observe; tail the log
sudo rc-service docker restart
sudo tail -f /var/log/iptables-trace.log

# Restore when done
sudo mv /usr/sbin/iptables.bak /usr/sbin/iptables
```

The `exec -a` form preserves `argv[0]` so stormwall picks the iptables
dialect, and there's no sub-process / fork — it's a true exec
replacement.

### Live monitor netlink notifications

```sh
# Tail every nft event (rule add/del, set update, chain change)
sudo nft monitor

# Just rule events
sudo nft monitor rules

# JSON for tooling
sudo nft monitor --json
```

`nft monitor` listens on the `nfnetlink_log` group and prints kernel-side
events as they happen — invaluable for watching what a daemon installs at
startup, or what's flushing your rules underneath you.

---

## nft compatibility

### Supported operations

| Command                                                                       | Status     |
|-------------------------------------------------------------------------------|------------|
| `add/create/delete/destroy table`                                             | Working    |
| `add/create/delete chain` (base + regular)                                    | Working    |
| `add/insert/delete rule`                                                      | Working    |
| `list table/tables/chain/chains/rules/ruleset`                                | Working    |
| `list sets/maps/counters/quotas/limits/objects/flowtables/meters/synproxys`   | Working    |
| `flush table/chain/ruleset`                                                   | Working    |
| `rename chain`                                                                | Working    |
| `add/delete set`, `add/delete element`                                        | Working    |
| `reset rules/counters/quotas`                                                 | Working    |
| Named objects: counter, quota, limit, ct helper/timeout/expectation, synproxy, secmark, tunnel | Working |
| `-f` file input, `-i` interactive REPL                                        | Working    |
| `-a` show handles, `-s` stateless, `-c` check                                 | Working    |
| `-j` JSON output + `-j -f file.json` input                                    | Working (20/20) |
| `-nn`/`-y`/`-p` numeric output                                                | Working (6/6)   |
| `define`/`redefine`/`undefine` (incl. hyphenated names)                       | Working    |

### Rule expressions

| Expression                                                          | Status   |
|---------------------------------------------------------------------|----------|
| `ip saddr/daddr`, `ip6 saddr/daddr`                                 | Working  |
| `tcp/udp sport/dport`, `tcp flags` (incl. masked `&`)               | Working  |
| `icmp type/code`, `icmpv6 type/code`                                | Working  |
| `ether saddr/daddr/type`, `ip dscp`                                 | Working  |
| `meta iifname/oifname/mark/l4proto/skuid/length/day/hour`           | Working  |
| `ct state/mark/status/zone`, `ct helper`, `ct count`                | Working  |
| `counter`, `log`, `limit rate`                                      | Working  |
| Named counters, quotas, limits                                      | Working  |
| `accept/drop/return/jump/goto/continue`                             | Working  |
| Anonymous chain bindings (`jump { ... }`, `goto { ... }`)           | Working  |
| `masquerade/snat/dnat`                                              | Working  |
| `notrack`, `reject`, `dup`, `fwd`, `synproxy`                       | Working  |
| Anonymous and named sets, interval sets, maps, vmaps                | Working  |
| Dynamic sets (timeout, `flags dynamic`)                             | Working  |
| `lookup` (set/map reference)                                        | Working  |
| `fib`, `jhash`, `numgen` (incl. `<,<=,>,>=`), `socket`, `rt`         | Working  |
| `flowtable`                                                         | Working  |
| `define` variable substitution                                      | Working  |
| CIDR matching, bitwise mask/xor                                     | Working  |

### Consumer compatibility

Wrappers and management tools that call `nft` under the hood work
transparently when stormwall is symlinked as `/usr/sbin/nft`:

- **firewalld** — zone/service/rich-rule operations (9/9 smoke tests)
- **iptables-translate** — 10/10 common rule shapes
- **fail2ban** — jail ban/unban via nftables backend
- **ufw** — tested via consumer-matrix harness

---

## iptables compatibility

Highlights as of 1.1.11. The forward path (rule → kernel) is fully
covered for everything in the tables below; the reverse path
(kernel → `iptables-save` text) is partially implemented (chain
headers + `COMMIT`; the per-rule reverse-renderer is the largest
remaining gap).

### Operations (-A/-I/-D/-R/-L/-S/-F/-Z/-N/-X/-P/-E/-C/-h/-V)

All covered. `-D <spec>` and `-C` are stubbed pending the
nft → iptables-save inverse renderer (which provides the spec→handle
matching they need); use `-D <chain> <rulenum>` for now.

### Targets (~30)

`ACCEPT`, `DROP`, `RETURN`, `REJECT` (with every documented
`--reject-with` kind), `LOG`, `MASQUERADE`, `SNAT`, `DNAT`, `REDIRECT`,
`MARK`, `CONNMARK` (incl. masked `--save-mark`/`--restore-mark` with
`--nfmask`/`--ctmask`), `TPROXY`, `NFLOG`, `TRACE`, `NOTRACK`, `NFQUEUE`
(incl. `--queue-num`, `--queue-balance`, `--queue-bypass`,
`--queue-cpu-fanout`), `TCPMSS` (incl. `--clamp-mss-to-pmtu`), `CT`
(incl. `--notrack`, `--helper`, `--zone`), `TOS`, `DSCP`, `TTL`, `HL`,
`CLASSIFY`, `CHECKSUM`, `NETMAP`, `SET`, plus user-chain jumps and
`--goto`. NAT targets accept `--random-fully` (modern NAT randomisation).
`LOG` accepts `--log-tcp-sequence`, `--log-tcp-options`,
`--log-ip-options`, `--log-uid`.

### Match modules (~30)

`addrtype`, `comment`, `conntrack`/`state` (incl. all states + `--ctstatus`
+ `--ctdir` + `--ctexpire`), `multiport`, `mark`, `connmark`, `physdev`,
`owner`, `set`, `mac`, `string` (stub), `limit`, `tcp`/`udp`, `iprange`,
`length`, `pkttype`, `tcpmss`, `tos`, `dscp`, `ttl`, `hl`, `statistic`
(`--mode random` and `--mode nth`), `connlimit`, `helper`, `time`
(`--timestart`/`--timestop`/`--weekdays`), `rpfilter`, `recent`
(`--set`/`--rcheck`/`--update`/`--remove` with `--seconds`/`--hitcount`/
`--name`/`--rsource`/`--rdest`), `connbytes`, `hashlimit` (full flag
suite incl. `--hashlimit-htable-*` and `--hashlimit-rate-*`), `policy`
(IPsec match), `ah`, `esp`, `frag`, `hbh`, `mh`, `rt`, `dst`,
`ipv6header`.

---

## pf.conf translation

stormwall's `--pf` mode parses OpenBSD `pf.conf` syntax and emits
equivalent nftables rules. A `pf.conf` carried over from a BSD box
runs unmodified on Linux.

**21/21 pf-parity tests pass** — expression-level byte equivalence
against upstream `nft --debug=netlink`. The test harness
(`tests/pf-parity.sh`) runs entirely offline (no root, no kernel) by
comparing bracketed netlink IR.

### pf features covered

| Feature                                            | Tests | Notes                              |
|----------------------------------------------------|:-----:|------------------------------------|
| scrub (no-df, random-id, min-ttl, max-mss)         | 5     | Packet normalization               |
| Source tracking (max-src-conn, rate, states, overload) | 4 | Connection limiting                |
| antispoof                                          | 1     | Interface-based spoofing prevention|
| set skip on lo                                     | 1     | Loopback bypass                    |
| Time-based rules                                   | 1     | Schedule matching                  |
| block return-rst                                   | 1     | TCP reset on block                 |
| log                                                | 1     | pflog integration                  |
| Routing actions (route-to, reply-to, dup-to)       | 1     | Policy routing                     |
| NAT (rdr-to, nat-to)                               | 1     | DNAT + SNAT                        |
| Tables (persist)                                   | 1     | Named address tables               |
| Anchors                                            | 1     | Nested rulesets                    |
| State flags (keep, modulate, synproxy)             | 1     | Stateful inspection                |
| Queue (ALTQ via TC)                                | 1     | Traffic shaping                    |

### pfctl shim

`pfctl` (`src/pfctl_main.rs`, ~1100 lines) provides a FreeBSD-compatible
pfctl(8) CLI. pfSense's PHP web UI calls it directly for all firewall
operations.

| Operation     | Flags                                                           |
|---------------|-----------------------------------------------------------------|
| Rule loading  | `-f file`, `-n` (check-only), `-O` (optimise)                   |
| Daemon mode   | `-D` (stay resident, SIGHUP reload, SIGTERM exit, PID file)     |
| Show          | `-sr`, `-sn`, `-ss`, `-si`, `-sa`, `-sT`, `-st`, `-sm`, `-sI`, `-s queue` |
| Flush         | `-F states`, `-F Sources`, `-F all`                             |
| Tables        | `-t NAME -T show/add/delete/flush/kill`                         |
| Kill states   | `-k host`, `-k gw -k ip` via `conntrack(8)` + `/proc` fallback  |
| Enable/disable | `-e` / `-d`                                                    |
| TC queues     | HTB / HFSC / PRIO via `src/tc.rs`                               |

---

## pfSense Docker container

`docker/pfsense/` runs the pfSense CE web UI natively on Linux.
stormwall's pfctl binary replaces FreeBSD's, and a PHP compatibility
layer translates BSD system calls to Linux equivalents.

```sh
docker build -f docker/pfsense/Dockerfile -t stormwall-pfsense .
docker run -d --name pfsense --cap-add NET_ADMIN -p 8443:443 stormwall-pfsense
# Login: admin / stormwall
```

**20/20 page tests pass** (login, dashboard, all firewall/status/system pages):

| Category                                          | Pages tested | Status |
|---------------------------------------------------|:------------:|:------:|
| Dashboard + login                                 | 1            | Pass   |
| Firewall (rules, NAT, aliases, schedules)         | 5            | Pass   |
| Status (dashboard, filter reload)                 | 2            | Pass   |
| Diagnostics (pfInfo, states, ARP, DNS, command)   | 5            | Pass   |
| Interfaces (WAN, assignments)                     | 2            | Pass   |
| Services (DHCP)                                   | 1            | Pass   |
| System (general, users, admin, certs)             | 4            | Pass   |

Architecture: nginx → PHP-FPM 8.2 → pfSense PHP (Apache 2.0) →
pfctl/stormwall → kernel nf_tables.

---

## Architecture

```
src/
  parser.rs       7700+ lines   Tokenizer + nft command parser
  ops.rs          5100+ lines   nftables CRUD via netlink batches
  output.rs       4400+ lines   Format kernel data back to nft syntax
  pf_parser.rs    2700+ lines   pf.conf parser + nft lowering
  iptables/       8200+ lines   iptables family: cmd / parser / lower / save / tests
  netlink.rs      1600+ lines   Raw netlink socket, message builder
  pfctl_main.rs   1100+ lines   pfctl(8) shim for pfSense
  tc.rs            970+ lines   TC queue setup (HTB/HFSC/PRIO)
  templates.rs     700+ lines   Compiled rule templates
  main.rs          500+ lines   CLI entry point, argv[0] dispatch
  json.rs          200+ lines   JSON output formatter
                  ─────────
                  ~33,000 lines total
```

Single dependency: `libc` (socket syscalls). Everything else is
implemented from scratch against the nf_tables netlink protocol.

### Unsafe code & security audit

Both binaries enforce `#![deny(warnings)]`. Every unsafe block carries a
`// SAFETY:` comment explaining its invariant.

| File             | Unsafe blocks | Purpose                                                                                              |
|------------------|:-------------:|------------------------------------------------------------------------------------------------------|
| `netlink.rs`     | 13            | socket, setsockopt×4, bind, getsockname, send, recv, poll, close, mem::zeroed×2                     |
| `parser.rs`      | 3             | if_nametoindex, localtime_r + time, tzset (test-only)                                                |
| `output.rs`      | 1             | if_indextoname                                                                                       |
| `pf_parser.rs`   | 1             | if_nametoindex                                                                                       |
| `main.rs`        | 1             | isatty(0)                                                                                            |
| `pfctl_main.rs`  | 4             | getpid×2, signal, pause                                                                              |

**Security hardening:**

- `u16` truncation guard on all netlink attribute length fields (4 sites)
- Reachable `unreachable!()` replaced with error return in pf_parser
- Verdicts ↔ elements length invariant enforced via debug_assert + checked indexing
- done_flags ↔ n_reqs invariant enforced in multi-dump response handler
- `parse_attrs` returns borrowed `&[u8]` slices — zero per-attribute allocation
- 68 unnecessary `.to_string()` comparisons eliminated in `ops.rs`
- Dead clones removed; interval sort done in-place

**Miri (1.0.5, nightly 1.97.0-nightly, x86_64-unknown-linux-gnu):**
137 passed · 0 failed · 29 ignored. All pure-Rust code (parser,
pf_parser, output, tc, templates) passes Miri with
`-Zmiri-disable-isolation`. The 29 ignored tests are kernel-call
integration tests (`#[ignore]`) and four time-zone tests
(`#[cfg_attr(miri, ignore)]`) that call `localtime_r(3)` / `tzset(3)` —
FFI calls Miri cannot shim.

See `INVARIANTS.md` for the full module-level invariant ledger.

---

## Test suites

### How to run

```sh
# Unit tests (parser + output, no root needed)
cargo test --offline

# pf-parity (offline, no root)
./tests/pf-parity.sh ./target/release/stormwall

# Integration tests (root + Linux nf_tables kernel module)
cargo test -- --ignored
sudo bash tests/stormwall_test.sh

# Behavioral soak harness (~80 iptables scenarios on a live kernel)
sudo IPTABLES=/usr/sbin/iptables ./tests/iptables-soak.sh
```

### nft test results

| Harness                          | What it tests                                                              | Result                |
|----------------------------------|----------------------------------------------------------------------------|-----------------------|
| `cargo test`                     | Unit tests (parser, output, lower, iptables)                               | **261 / 261**         |
| `stormwall_test.sh`              | Core nft compatibility (file, JSON, round-trip, ct, meta, NAT)             | **25 / 25**           |
| `monitor-diff.sh`                | 45 rulesets: monitor event parity vs upstream nft                          | **45 / 45**           |
| `traffic-parity.sh`              | 16 ruleset shapes, real veth pair, accept/drop per rule                    | **16 / 16**           |
| `traffic-parity-ipv6.sh`         | IPv6 rules, real packets through kernel                                    | **18 / 18**           |
| `traffic-parity-bridge.sh`       | Bridge/L2 rules, 3-netns setup                                             | **9 / 12** (3 env)    |
| `traffic-nat.sh`                 | masquerade, snat, dnat, negative case                                      | **4 / 4**             |
| `fuzz-parity.sh`                 | 50 random rulesets, apply+list compare vs nft                              | **50 / 50**           |
| `iperf3-throughput.sh`           | Throughput vs nft under load (6 scenarios)                                 | **6 / 6** (0.81–1.03×)|
| `nft-check-parity.sh`            | `-c` flag validation (35-case corpus)                                      | **34 / 35**           |
| `json-parity.sh`                 | `-j list ruleset` JSON shape (20-case corpus)                              | **20 / 20**           |
| `iptables-translate-parity.sh`   | iptables-translate wrapper (10 rule shapes)                                | **10 / 10**           |
| `firewalld-smoke.sh`             | firewalld / fail2ban / ufw consumer compat                                 | **9 / 9**             |
| `concurrent-writers.sh`          | 8-worker kernel contention, EBUSY/EAGAIN                                   | **4 / 4**             |
| `numeric-output-parity.sh`       | `-nn` / `-y` / `-p` numeric output (6 rulesets × 3 modes)                  | **6 / 6**             |
| `upstream-nft-suite.sh`          | Upstream nftables shell suite (22 categories, 493 tests)                    | **487 / 493** (99 %)  |
| `iptables-soak.sh`               | Behavioral iptables harness (~80 scenarios on live kernel)                 | **78 / 81**           |
| Dump parse coverage              | Parser `-c` accepts upstream nft dump files                                | **142 / 142** (100 %) |

#### Upstream nftables shell suite breakdown

Tested on Raspberry Pi 5 (aarch64, kernel 6.18, Debian trixie).

| Category          | Pass | Skip | Fail | Total |
|-------------------|-----:|-----:|-----:|------:|
| sets              | 108  | 0    | 0    | 108   |
| chains            | 53   | 0    | 0    | 53    |
| transactions      | 48   | 4    | 0    | 52    |
| maps              | 49   | 0    | 0    | 49    |
| nft-f             | 35   | 1    | 0    | 36    |
| packetpath        | 31   | 0    | 0    | 31    |
| listing           | 25   | 0    | 0    | 25    |
| optimizations     | 24   | 0    | 0    | 24    |
| include           | 22   | 0    | 0    | 22    |
| bitwise           | 17   | 0    | 0    | 17    |
| flowtable         | 16   | 0    | 0    | 16    |
| rule_management   | 12   | 0    | 0    | 12    |
| cache             | 11   | 0    | 0    | 11    |
| optionals         | 11   | 0    | 0    | 11    |
| json              | 10   | 0    | 0    | 10    |
| parsing           | 4    | 1    | 0    | 5     |
| netns             | 3    | 0    | 0    | 3     |
| nft-i             | 3    | 0    | 0    | 3     |
| owner             | 2    | 0    | 0    | 2     |
| comments          | 1    | 0    | 0    | 1     |
| bogons            | 1    | 0    | 0    | 1     |
| trace             | 1    | 0    | 0    | 1     |
| **Total**         | **487** | **6** | **0** | **493** |

The 6 skipped tests are stress / performance tests (large rulesets,
30-second stress loops) that upstream nft also skips by default.

### pf test results

| Harness          | What it tests                                                | Result    |
|------------------|--------------------------------------------------------------|-----------|
| `pf-parity.sh`   | 21 pf → nft expression-level parity vs `nft --debug=netlink` | **21 / 21** |
| pfSense Docker   | 20 page tests (login, dashboard, firewall, status, system)   | **20 / 20** |

### iptables behavioral harness

The behavioral harness in [`tests/iptables-soak.sh`](tests/iptables-soak.sh)
runs ~80 iptables scenarios against a live kernel and validates the
resulting `nft list ruleset`:

```sh
sudo IPTABLES=/usr/sbin/iptables ./tests/iptables-soak.sh
```

Output:

```
=== 1. Built-in chains, simple targets ===
=== 2. Negation (modern + legacy) ===
... (14 sections, ~80 scenarios)

soak result: 78/81 passed
failed: 10.01-add-then-delete-by-spec 11.01-check-existing-rule 14.01-save-restore-roundtrip
```

The few remaining failures are all the listing-renderer / `-D <spec>`
stubs documented under [iptables compatibility](#iptables-compatibility) —
none affect rule installation.

---

## Performance

| Scenario             | stormwall | nft       | Ratio  |
|----------------------|----------:|----------:|-------:|
| Throughput, no rules | 1960 Mbps | 1900 Mbps | 1.03×  |
| Throughput, 100 rules| 1073 Mbps | 1320 Mbps | 0.81×  |
| Single op latency    | 80 ms     | 88 ms     | 0.9×   |

The ct-soak 52 % stateful gap is a thermal artefact from sequential
5-minute arms on the RPi5 test host. Alternating 10-second measurements
show 0.97–1.12×. See `tests/ct-state-perf-investigation.md`.

---

## Build from source

```sh
# Get the source tarball from the GitHub release mirror
curl -fsSL -o stormwall-1.1.11.tar.gz \
    https://github.com/stormj-UH/jonerix/releases/download/source-stormwall-v1.1.11/stormwall-1.1.11.tar.gz
tar xzf stormwall-1.1.11.tar.gz
cd stormwall-1.1.11

# Build (vendored deps; no network needed)
cargo build --release --bin stormwall --bin pfctl --offline --frozen

# Cross-compile for aarch64
RUSTFLAGS="-C linker=rust-lld" cargo build --release \
    --target aarch64-unknown-linux-musl

# Build a .jpkg package (jonerix native format)
./mkjpkg.sh x86_64    # or aarch64

# Or via jpkg-local (recipe in this repo)
cd /path/to/jonerix
jpkg-local build packages/core/stormwall --build-jpkg --output .
```

The recipe sets `RUSTFLAGS=-C strip=symbols -C target-feature=+crt-static`
so the resulting binary is fully static-musl-linked.

The build produces two binaries:

| Binary       | Purpose                                                      |
|--------------|--------------------------------------------------------------|
| `stormwall`  | Multi-dialect dispatch — selects nft / iptables / pfctl by `argv[0]` |
| `pfctl`      | FreeBSD-compatible pfctl(8) shim for pfSense on Linux        |

---

## Known gaps

- **iptables-save reverse renderer:** the per-rule renderer that turns
  arbitrary nft expressions back into iptables-save text is partial —
  chain headers and `COMMIT` are emitted, but the per-rule lines aren't.
  `iptables -A` (forward path) is fully covered. Tracked in the soak
  harness as `14.01-save-restore-roundtrip`.
- **`iptables -D <spec>` and `-C`:** depend on the inverse renderer for
  spec → handle matching. `iptables -D <chain> <rulenum>` works.
- **`-o` optimiser:** rule optimisation pass not implemented (`-o` is
  accepted and silently ignored).
- **CONNLIMIT inline form:** install currently fails on jonerix kernel
  configurations that require a stateful object for connlimit. The
  inline form (`-m connlimit --connlimit-above N`) is parsed and lowered
  but the kernel rejects it. Use named connlimit objects via nft instead.

---

## License

MIT — see [LICENSE](LICENSE).
