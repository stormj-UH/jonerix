# jonerix

**A Linux distribution with a strictly permissive userland.**

Every component running on a jonerix system is licensed under MIT, BSD, ISC, Apache-2.0, or public domain. The Linux kernel is the sole GPL exception — there is no viable permissive alternative. GPL tools may be used during the build process but never ship in the final image.

---

## Table of Contents

1. [Philosophy](#1-philosophy--license-policy)
2. [Core Stack](#2-core-component-stack)
3. [Package Manager (jpkg)](#3-package-manager-jpkg)
4. [Filesystem Layout](#4-filesystem-layout)
5. [Boot Sequence](#5-boot-sequence)
6. [Networking](#6-networking)
7. [Security Hardening](#7-security-hardening)
8. [Container & Cloud](#8-container--cloud)
9. [Repository Structure](#9-repository-structure)
10. [Build Recipes](#10-build-recipe-format)
11. [Open Questions](#11-open-questions--future-work)

---

## 1. Philosophy & License Policy

jonerix exists because permissive licensing matters for infrastructure. Operators should be able to inspect, modify, and redistribute their OS without any copyleft obligations. We do not ship the Linux Kernel. The rules are simple:

| Rule | Detail |
|------|--------|
| **Runtime** | Every binary, library, config, and script on the running system must be permissive (MIT, BSD, ISC, Apache-2.0, 0BSD, CC0, public domain). |
| **Self-hosting** | The final system must be able to rebuild itself from source using only its own (permissive) tools. The `jonerix:builder` image proves this. |

### Why not just use a BSD?

FreeBSD/OpenBSD are excellent. jonerix targets a different niche:

- **Linux kernel**: vastly wider hardware support, mature cgroups/namespaces for containers, better cloud driver ecosystem.
- **musl + toybox**: smaller than BSD base, competitive with Alpine's footprint.
- **Familiarity**: Linux syscall ABI, `/proc`, `/sys`, OCI compatibility — no porting friction for server workloads.

---

### Image Chain

jonerix produces a layered set of Docker images, each built from the previous:

```
minimal -> core -> builder   (compilers + dev tools)
                -> router    (networking appliance)
```

| Image | Base | Contents |
|-------|------|----------|
| `minimal` | scratch | musl, toybox, dropbear, curl, libressl, openrc, jpkg |
| `core` | minimal | mksh, uutils, pico, fastfetch, ripgrep, gitredoxide, ncurses, networking |
| `builder` | core | clang/llvm, rust, rustdoc, rustfmt, rustup, go, nodejs, python3, cmake, jmake, samurai, perl |
| `router` | core | jcarp, hostapd, wpa_supplicant, btop, unbound DNS config, sysctl hardening |

### Build Pipeline

1. **jpkg** is built from C source in an jonerix:builder container
2. **Packages** are built from source using `scripts/build-all.sh` and per-package `recipe.toml` recipes in `packages/{core,develop,extra}/`
3. **Images** are assembled using Dockerfiles that install packages via `jpkg install`
4. **CI** builds and publishes all images and packages for both x86_64 and aarch64

### From-Source Builds

All packages build from source on jonerix itself (both x86_64 and aarch64):

- **C/C++**: musl, toybox, LibreSSL, curl, dropbear, OpenRC, ncurses, etc.
- **LLVM/Clang/LLD**: Full compiler toolchain from source
- **Go chain**: C → Go 1.4 → 1.17 → 1.20 → 1.22 → 1.24 → 1.26 (bootstrapped from C)
- **Go packages**: containerd, runc, nerdctl, CNI plugins, headscale, derper
- **Rust packages**: uutils, gitredoxide, ripgrep, btop (via system LLVM + bootstrap rustc)
- **Scripting**: Python 3, Node.js, Perl (from source with Clang/musl)

All components are compiled with:
```sh
CC=clang
LD=ld.lld
CFLAGS="-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2"
LDFLAGS="-Wl,-z,relro,-z,now -pie"
```

Packages are uploaded to GitHub Releases and installed via jpkg into clean rootfs images.

---

## 2. Core Component Stack

| Role | Component | License | Replaces |
|------|-----------|---------|----------|
| Kernel | Linux | GPLv2 (sole exception) | — |
| C library | musl | MIT | glibc (LGPL) |
| Compiler | LLVM/Clang + lld | Apache-2.0 w/ LLVM exception | GCC (GPL) |
| Coreutils | toybox + uutils | 0BSD / MIT | BusyBox (GPL), GNU coreutils (GPL) |
| Shell | zsh | BSD | bash (GPL) |
| Init system | OpenRC | BSD-2-Clause | systemd (LGPL) |
| Privilege escalation | sudo | ISC | — |
| TLS library | LibreSSL | ISC | OpenSSL (Apache-2.0) |
| SSH server | dropbear | MIT | OpenSSH (BSD, but depends on GPL OpenSSL historically) |
| HTTP client | curl | curl license (MIT-like) | wget (GPL) |
| DNS resolver | unbound | BSD-3-Clause | — |
| DHCP client | dhcpcd | BSD-2-Clause | — |
| Logging | toybox syslogd | 0BSD | syslog-ng (GPL) |
| Cron | snooze | CC0 (public domain) | cronie (GPL) |
| Man pages | mandoc | ISC | groff (GPL) |
| Network config | ifupdown-ng | ISC | — |
| WiFi AP | hostapd | BSD-3-Clause | — |
| WiFi client | wpa_supplicant | BSD-3-Clause | — |
| Bootloader | EFISTUB (kernel feature) | n/a | GRUB (GPL) |
| Package manager | jpkg (custom) | MIT | apk-tools (GPL) |
| Compression | zstd (BSD), lz4 (BSD), zlib (zlib), pigz (zlib) | BSD / zlib | gzip (GPL) |
| C++ standard library | libc++ / libc++abi | Apache-2.0 w/ LLVM exception | libstdc++ (GPL) |
| Scripting / build tool | Python 3 | PSF-2.0 | — |
| JavaScript runtime | Node.js | MIT | — |
| Build tool (ninja) | samurai | Apache-2.0 | GNU make (GPL) |
| Build tool (make) | jmake | MIT | GNU make (GPL) |
| Text editor | pico | Apache-2.0 | vim |
| Grep | ripgrep | MIT | GNU grep (GPL) |
| Git | gitredoxide | MIT/Apache-2.0 | git (GPL) |
| System info | fastfetch | MIT | neofetch |
| Process monitor | btop | Apache-2.0 | htop |
| awk | onetrueawk | MIT | gawk (GPL) |

### Notable Absences and Why

**GNU make**: Replaced by `jmake` (MIT, clean-room Rust implementation) and `samurai` (Apache-2.0, ninja-compatible) for cmake projects. jmake is a drop-in replacement that handles all GNU make features.

**bash**: zsh is used as the default shell in all non-minimal builds. Brash is a drop-in replacement and is available.

**gzip**: The gzip *format* is open. `pigz` (zlib license) handles `.gz` files. `zstd` is the preferred compression.

**systemd**: OpenRC is simpler, BSD-licensed, and proven in Alpine. The service file format is plain shell.

### POSIX-First Code Discipline

Every shell script, init script, build recipe, and in-tree tool jonerix ships should run under a strict POSIX interpretation. Concretely: **no bashisms, no GNUisms.** This is a practical consequence of the runtime — `/bin/sh` is mksh (not bash), coreutils is toybox (not GNU coreutils), sed/awk/grep are toybox (or BSD equivalents), and `patch(1)` is toybox. A script that quietly depends on `[[ ... ]]`, `local`, `echo -e`, `sed -i` with a backup argument, `grep -P`, `readlink -f`, `mktemp --tmpdir`, or `getent` works on the author's Debian laptop and fails in CI or on a fresh Pi install. We have already paid for each of those lessons.

Concrete rules:

- **Shebangs.** Prefer `#!/bin/sh` for scripts we author. Only switch to `#!/bin/mksh` or `#!/usr/bin/env python3` when you actually need features POSIX sh doesn't provide. Never `#!/bin/bash` in anything that ships — bash is not on the runtime image.
- **Shell features.** Stick to POSIX.1-2017: `$()` (not backticks), `[ ... ]` (not `[[ ... ]]`), `"$var"` (quoting is not optional), `getopts` (not GNU getopt), `case` statements (not bash regex), arithmetic via `$(( ))`. Avoid `local`, arrays, `${var^^}`/`${var,,}`, process substitution `<(...)`, `function foo()`, `read -p`, `echo -e` / `echo -n` — use `printf` instead.
- **Coreutils flags.** Use only the flags documented in POSIX or SUS. `sed -i` is GNU-only — emit to stdout and redirect, or `mv` afterwards. `cp -a` and `install -D` are GNU-ish but supported by toybox; they are OK. `readlink -f` is GNU — use the POSIX `readlink` in a loop, or shell out to python for path resolution. `grep -P` is GNU — rewrite using POSIX ERE (`grep -E`). `xargs -r` is GNU — check for empty input explicitly.
- **Patches.** Unified diffs must apply under strict `patch(1)` (toybox 0.8.11). No fuzz, no re-indenting context lines, no preamble tricks. If a patch won't hold cleanly, use a Python or awk text-substitution pre-step instead (see `packages/extra/btop/cpuname-patch.py` for the pattern).
- **New C / Rust tools.** Prefer POSIX APIs over `_GNU_SOURCE` extensions. No `asprintf`, no `getline` without a fallback, no `pipe2` (use `pipe` + `fcntl(F_SETFD, FD_CLOEXEC)`), no `ppoll`/`epoll_pwait` unless the feature actively matters. In Rust, avoid crates that assume glibc (`rustix` is fine, `nix` usually fine; watch for crates that gate behind `target_env = "gnu"`).
- **Testing.** Run new scripts through `mksh -n` (syntax check) and ideally `shellcheck --shell=sh --severity=style` before landing. Tool smoke tests should run under both the x86_64 and aarch64 jonerix builder images, which are the closest thing we have to the real target.

When portability and pragmatism collide, portability wins unless there's a written-down reason (e.g., a recipe's comment block) explaining why the GNU/bash extension is load-bearing for this specific case.

---

## 3. Package Manager: jpkg

`jpkg` is a custom, 0BSD-licensed package manager purpose-built for jonerix. It is intentionally minimal.

### Package Format

A `.jpkg` file is a zstd-compressed tarball with a prepended metadata header:

```
┌────────────────────────┐
│ PKG magic (8 bytes)    │  "JPKG\x00\x01\x00\x00"
│ Header length (4 bytes)│
│ PKG metadata (TOML)    │
│────────────────────────|
│ zstd-compressed tar    │  (the actual files)
└────────────────────────┘
```

### Metadata (PKG)

```toml
[package]
name = "toybox"
version = "0.8.11"
license = "0BSD"
description = "BSD-licensed replacement for BusyBox"
arch = "x86_64"

[depends]
runtime = ["musl"]
build = ["clang"]

[files]
sha256 = "abc123..."
size = 245760
```

### Repository Layout

Packages are hosted on GitHub Releases as a flat set of `.jpkg` files with a signed index:

```
github.com/stormj-UH/jonerix/releases/download/packages/
├── INDEX.zst          ← signed manifest of all packages + versions + hashes
├── INDEX.zst.sig      ← Ed25519 signature
├── toybox-0.8.11-aarch64.jpkg
├── toybox-0.8.11-x86_64.jpkg
├── mksh-R59c-aarch64.jpkg
└── ...
```

No database server required. GitHub Releases (or any static file host) is sufficient. Packages include architecture in the filename; the INDEX contains `[name-arch]` sections for multi-arch support.

### Commands

```sh
jpkg update                  # fetch INDEX from mirrors
jpkg install <pkg>           # install package + deps
jpkg remove <pkg>            # remove package
jpkg upgrade                 # upgrade all installed packages
jpkg search <query>          # search package names/descriptions
jpkg info <pkg>              # show package metadata
jpkg list                    # list installed packages
jpkg build <recipe-dir>      # build package from source recipe
jpkg audit                   # verify all installed packages are permissive
jpkg sign <file> --key <key> # sign a file with Ed25519
```

### License Enforcement

jpkg enforces permissive licensing at build and install time. The allowlist includes: MIT, BSD-2-Clause, BSD-3-Clause, ISC, Apache-2.0, 0BSD, CC0, Unlicense, MirOS, OpenSSL, zlib, PSF-2.0, Artistic-2.0, Ruby, MPL-2.0, and public domain variants. SPDX compound expressions (`AND`/`OR`) are parsed recursively — `AND` requires all components permissive, `OR` requires at least one.

### Signing

Packages and the INDEX manifest are signed with Ed25519. The distribution's public key is shipped in `/etc/jpkg/keys/`. Signature verification uses tweetnacl (public domain).

### Implementation

Written in C (~5000 lines), built with `samu` (ninja). Linked statically against musl. Dependencies: LibreSSL (HTTPS), zstd (decompression), tweetnacl (Ed25519).

---

## 4. Filesystem Layout

Merged `/usr` — all binaries live in `/bin`, all libraries in `/lib`. It's 2026. What are we doing? Symlinks for compatibility:

```
/
├── bin/              ← all executables (merged /usr/bin + /sbin)
├── lib/              ← all libraries (merged /usr/lib)
├── include/          ← all headers (merged /usr/include)
├── etc/              ← system configuration
│   ├── init.d/       ← OpenRC service scripts
│   ├── conf.d/       ← OpenRC service config
│   ├── ssl/          ← TLS certificates
│   ├── network/      ← ifupdown-ng interfaces
│   ├── jpkg/         ← package manager config + keys
│   ├── unbound/      ← DNS resolver config (router)
│   ├── hostapd/      ← WiFi AP config (router)
│   ├── fastfetch/    ← system info display config
│   └── securetty     ← allowed TTYs for root login
├── var/
│   ├── log/          ← syslogd output
│   ├── cache/jpkg/   ← downloaded packages
│   └── db/jpkg/      ← installed package database
├── home/
├── root/
├── boot/             ← vmlinuz (EFISTUB-capable)
├── dev/              ← devtmpfs
├── proc/             ← procfs
├── sys/              ← sysfs
├── run/              ← tmpfs (runtime state)
├── tmp/              ← tmpfs
└── usr -> /          ← symlink for compatibility
```

---

## 5. Boot Sequence

### UEFI (preferred, no GPL bootloader needed)

```
UEFI firmware
  → ESP partition: /EFI/jonerix/vmlinuz.efi  (kernel with EFISTUB)
  → Kernel command line embedded or via UEFI vars:
      root=/dev/sda2 rootfstype=ext4 init=/bin/openrc-init quiet
  → Linux kernel boots, mounts root
  → /bin/openrc-init (PID 1)
  → OpenRC sysinit runlevel:
      - mount /proc, /sys, /dev, /run
      - load kernel modules
      - set hostname
      - seed RNG from /var/lib/urandom/seed
  → OpenRC default runlevel:
      - networking (ifupdown-ng: configure interfaces)
      - sshd (dropbear)
      - syslogd (logging via toybox)
      - snooze (cron jobs)
  → getty spawns on tty1-tty3
  → User logs in → mksh
```
---

## 6. Networking

| Function | Tool | License |
|----------|------|---------|
| Interface config | ifupdown-ng | ISC |
| DHCP client | dhcpcd | BSD-2-Clause |
| DNS resolver | unbound (recursive, DNSSEC) | BSD-3-Clause |
| HTTP/HTTPS client | curl | curl (MIT-like) |
| SSH | dropbear | MIT |
| WiFi AP | hostapd | BSD-3-Clause |
| WiFi client | wpa_supplicant | BSD-3-Clause |
| HA failover | jcarp | BSD-2-Clause |
| VPN relay | derper (Tailscale DERP) | BSD-3-Clause |
| Firewall | *see below* | — |

### Router Image

The `jonerix:router` image extends core with networking packages and ships default configs:

- **Unbound**: recursive DNS with DNSSEC, listens on LAN, rebind protection
- **nloxied** Rust-based replacement for libnl
- **jcarp**: OpenBSD-CARP-compatible virtual IP failover
- **Hostapd**: WiFi access point (WPA2, disabled by default — user must configure SSID/passphrase)
- **Network interfaces**: WAN (eth0, DHCP) + LAN (eth1, static 192.168.1.1/24)
- **Sysctl hardening**: IP forwarding enabled, SYN cookies, ICMP redirect rejection, reverse path filtering
- **Stormwall**: A drop-in replacement for nftables. 
---

## 7. Security Hardening

### Compile-Time Defaults

All packages are built with:

| Flag | Purpose |
|------|---------|
| `-fstack-protector-strong` | Stack buffer overflow detection |
| `-D_FORTIFY_SOURCE=2` | Runtime buffer overflow checks |
| `-fPIE` + `-pie` | Position-independent executables (ASLR) |
| `-Wl,-z,relro,-z,now` | Full RELRO — GOT is read-only after load |

### Runtime Defaults

- **Multi-user**: getty on tty1-tty3, `/etc/securetty` restricts root login to console + tty1-3 + ttyS0.
- **SUID bits**: Only `su`, `passwd`, and `login` have setuid (chmod 4755).
- **System accounts**: daemon, bin, sys with nologin. Separate groups for tty, disk access.
- **Kernel hardening**: `kernel.kptr_restrict=2`, `kernel.dmesg_restrict=1`, `kernel.unprivileged_bpf_disabled=1`.
- **Router sysctl**: SYN cookies, ICMP redirect rejection, reverse path filtering, no source routing.

### Rust

New code where feasible is written in Rust. All uses of "unsafe" are audited.

---

## 8. Container & Cloud

### Image Chain

```sh
# Pull from GHCR
docker pull ghcr.io/stormj-uh/jonerix:minimal   # base: toybox, dropbear, openrc
docker pull ghcr.io/stormj-uh/jonerix:core       # runtime: mksh, pico, ripgrep, networking
docker pull ghcr.io/stormj-uh/jonerix:builder    # dev: core + clang, rust, rustdoc, rustfmt, rustup, go, python3
docker pull ghcr.io/stormj-uh/jonerix:router     # networking: core + hostapd, unbound config

# Per-arch tags: -amd64 and -arm64 are also available
```

All images are multi-arch (x86_64 + aarch64), built in CI via `publish-images.yml`, and published to `ghcr.io/stormj-uh/jonerix`.

### Local Builds

```sh
./scripts/build-local.sh                           # build minimal + core + builder
./scripts/build-local.sh minimal core              # specific targets
./scripts/build-local.sh router                    # networking appliance
./scripts/build-local.sh packages                  # build all recipes from source
PKG_INPUT=ruby ./scripts/build-local.sh packages   # single recipe
```

### Building From Source

The builder image is self-sufficient — it can rebuild itself and all other images:

```sh
docker run --rm -v "$PWD:/workspace" -v "$PWD/.build/pkgs:/output" \
  jonerix:builder -c 'sh /workspace/scripts/build-from-source.sh'
```

### Cloud Images

| Platform | Format | Init Integration |
|----------|--------|------------------|
| AWS | AMI (raw → EBS snapshot) | tiny-ec2-bootstrap (MIT) |
| GCP | .tar.gz (raw disk image) | GCP guest agent (Apache-2.0) |
| Generic | .qcow2 / .img | OpenRC + cloud-init-lite (custom, MIT) |

### GitHub Actions CI

The CI pipeline (`publish-images.yml`) builds and publishes all images:

```
check-sources → minimal (arm64 + amd64) → core → builder → smoke tests
                                               → router  →
```

Package builds (`publish-packages.yml`) compile recipes inside `jonerix:builder` containers and upload `.jpkg` files to the rolling `packages` GitHub Release. The rolling INDEX is regenerated and Ed25519-signed after each build.

Versioned package releases are controlled by `package-release-state.yml`. Opening a release writes the active tag to the internal `package-release-state` release; while that marker exists, every package publish also mirrors matching updated `.jpkg` assets into the open tag and rebuilds that tag's signed INDEX. Closing the release deletes the marker, so later package publishes only update the rolling `packages` release.

---

## 9. Repository Structure

```
jonerix/
├── DESIGN.md                ← this document
├── README.md                ← quick start and overview
├── LICENSE                  ← MIT
│
├── Dockerfile.minimal       ← base rootfs (FROM scratch)
├── Dockerfile.core          ← core runtime (FROM minimal)
├── Dockerfile.builder       ← compilers + dev tools (FROM core)
├── Dockerfile.router        ← networking appliance (FROM core)
├── Dockerfile               ← full image (legacy, jpkg-based)
├── Dockerfile.develop       ← development environment (jpkg-only)
├── Dockerfile.build         ← build environment (FROM develop)
│
├── scripts/
│   ├── build-all.sh         ← build all packages from source in dependency order
│   ├── build-order.txt      ← 10-tier dependency ordering (60+ packages)
│   └── ...                  ← utility scripts
│
├── docs/
│   ├── JONERIX-BUILD-ENVIRONMENT.md  ← build environment reference
│   └── ...
│
├── packages/
│   ├── core/                ← runtime packages (minimal + core images)
│   │   ├── musl/            ← each has recipe.toml
│   │   ├── toybox/
│   │   ├── mksh/
│   │   └── ... (29 packages)
│   ├── develop/             ← compilers + build tools (builder image)
│   │   ├── llvm/
│   │   ├── rust/
│   │   ├── go/
│   │   └── ... (17 packages)
│   ├── extra/               ← apps, router packages, container tools
│   │   ├── headscale/
│   │   ├── hostapd/
│   │   ├── containerd/
│   │   └── ... (18 packages)
│   └── jpkg/                ← the package manager source code
│       ├── src/*.c, src/*.h
│       └── build.ninja      ← built with samu (ninja-compatible)
│
├── config/
│   ├── openrc/
│   │   ├── init.d/          ← service scripts
│   │   └── inittab          ← getty on tty1-tty3
│   ├── router/
│   │   └── etc/             ← unbound, hostapd, interfaces, sysctl
│   └── defaults/
│       └── etc/             ← hostname, passwd, group, shadow, profile,
│                               securetty, os-release, fastfetch/
│
├── scripts/
│   ├── build-local.sh       ← build images locally (mirrors CI chain)
│   ├── build-from-source.sh ← build all recipes inside builder container
│   ├── ci-build-aarch64.sh  ← CI package build script (arm64)
│   ├── ci-build-x86_64.sh   ← CI package build script (x86_64)
│   ├── build-llvm-libcxx.sh ← LLVM from-source build helper
│   ├── license-audit.sh     ← verify all components are permissive
│   └── size-report.sh       ← image size analysis
│
├── sources/                 ← vendored source tarballs
│
├── .github/
│   └── workflows/
│       ├── publish-images.yml    ← CI: build + push Docker images
│       ├── publish-packages.yml  ← CI: build + upload jpkg packages
│       ├── full-bootstrap.yml    ← CI: full from-source bootstrap test
│       ├── license-check.yml
│       └── wsl-rootfs.yml
│
└── docs/
    ├── packaging.md
    └── contributing.md
```

---

## 10. Build Recipe Format

Each package has a `recipe.toml` in `packages/{core,develop,extra}/`. Recipes contain metadata, source URL, build instructions, and dependencies.

```toml
# packages/core/toybox/recipe.toml
[package]
name = "toybox"
version = "0.8.11"
license = "0BSD"
description = "BSD-licensed coreutils replacement"

[source]
url = "http://landley.net/toybox/downloads/toybox-0.8.11.tar.gz"
sha256 = "15aa3f832f4ec1874db761b9950617f99e1e38144c22da39a71311093bfe67dc"

[build]
system = "custom"
build = """set -e
bash scripts/genconfig.sh defconfig
for opt in SH GETTY LOGIN PASSWD SU; do
  sed -i "s/# CONFIG_${opt} is not set/CONFIG_${opt}=y/" .config
done
CC=clang CFLAGS="-Os -fomit-frame-pointer" bash scripts/make.sh
"""
install = """set -e
mkdir -p $DESTDIR/bin
install -m755 generated/unstripped/toybox $DESTDIR/bin/toybox
llvm-strip --strip-all $DESTDIR/bin/toybox 2>/dev/null || true
# Create applet symlinks
for applet in $(./generated/unstripped/toybox --long 2>/dev/null); do
  ln -sf toybox "$DESTDIR/bin/$applet" 2>/dev/null || true
done"""

[depends]
runtime = ["musl"]
build = ["clang"]
```

`scripts/build-all.sh` processes recipes in dependency order from `scripts/build-order.txt`, searching across `packages/{core,develop,extra}/` for each recipe. jpkg sets `CC=clang`, `LD=ld.lld`, hardening flags, and an isolated `DESTDIR` automatically.

---

## 11. Open Questions & Future Work

### Completed

- [x] Write `jpkg` — custom package manager (C, MIT, ~5000 lines)
- [x] Full from-source bootstrap (60+ packages, both arches)
- [x] Self-hosting (`jonerix:builder` rebuilds everything from source)
- [x] Publish OCI images (minimal, core, builder, router)
- [x] CI pipeline on GitHub Actions (images + packages + smoke tests)
- [x] aarch64 + x86_64 support (multi-arch manifests)
- [x] Multi-user mode (getty, system accounts, SUID, securetty)
- [x] License enforcement in jpkg (allowlist + SPDX AND/OR parsing)
- [x] Router image with unbound DNS, hostapd, sysctl hardening
- [x] Raspi support

### Roadmap (deferred — not blocking any current release)

- [ ] **Cross-compile between x86_64 and aarch64** — make the LLVM family
  multi-target (`LLVM_TARGETS_TO_BUILD="X86;AArch64"`) plus ship per-arch
  sysroot jpkgs (`musl-cross-*`, `libcxx-cross-*`, `compiler-rt-builtins-cross-*`)
  so either host arch can produce jpkgs for either target arch. Eliminates
  the castle x86_64 / cloud-arm64 round-trip. Two arches only — no plans
  for a third. Full plan in [docs/cross-compile-design.md](docs/cross-compile-design.md).
- [ ] **`gitredoxide` upload-pack `filter` capability** — server-side support
  for partial-clone filter advertising (`git clone --filter=blob:none` against
  a gitredoxide-served upstream). Client-side `git backfill` already lands
  blobs from a promisor remote that DOES support filter; the missing piece
  is the gitredoxide helper-mode upload-pack advertising the capability and
  honouring the v2 fetch `filter <spec>` line.  Needed for self-hosted
  partial-clone scenarios on tormenta/jonerix-served Forgejo mirrors. No
  current dependency. ([gitredoxide repo](https://castle.great-morpho.ts.net:3000/jonerik/gitredoxide))
- [ ] **Vendored Rust source-tarball trimming** — current gitredoxide source
  tarball is 130 MB compressed because `cargo vendor` copies the full source
  of every transitive crate (windows-*, aws-lc-sys, sqlite-wasm-rs, web-sys,
  etc.) regardless of whether it compiles for jonerix-musl targets. A first
  attempt (1.0.9 vendor-prune.sh) worked locally but broke
  `cargo build --frozen --offline` in CI because `--frozen` validates the
  whole `Cargo.lock` against the vendor dir. A target-aware lockfile
  regeneration is the cleanest fix; cargo's stable surface doesn't expose it
  yet — research item, see `gitredoxide/scripts/VENDOR-TRIM-RESEARCH.md`
  (when written).

### Known Compromises

| Item | Status | Notes |
|------|--------|-------|
| Linux kernel | GPLv2 — accepted | No permissive OS kernel with equivalent hardware/container support exists |
| CA certificates | Mozilla bundle, MPL-2.0 | This is *data*, not *code*. MPL-2.0 now in jpkg allowlist. |
| BIOS bootloader | syslinux (GPL) on boot media only | Not installed to rootfs. UEFI EFISTUB avoids this entirely. |
| GNU make at build time | GPL, Alpine container only | Ruby, hostapd, wpa_supplicant upstream Makefiles require it. Never shipped. |

---

## License

This document and all original jonerix code are released under the
**BSD Zero Clause License** ([0BSD](https://opensource.org/licenses/0BSD)).
The full text is in [`LICENSE`](LICENSE).

```
BSD Zero Clause License

Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```
