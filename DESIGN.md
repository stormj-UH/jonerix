# jonerix

**A Linux distribution with a strictly permissive userland.**

Every component running on a jonerix system is licensed under MIT, BSD, ISC, Apache-2.0, or public domain. The Linux kernel is the sole GPL exception — there is no viable permissive alternative. GPL tools may be used during the build process but never ship in the final image.

---

## Table of Contents

1. [Philosophy](#1-philosophy--license-policy)
2. [Bootstrap Strategy](#2-multi-stage-bootstrap)
3. [Core Stack](#3-core-component-stack)
4. [Package Manager (jpkg)](#4-package-manager-jpkg)
5. [Filesystem Layout](#5-filesystem-layout)
6. [Boot Sequence](#6-boot-sequence)
7. [Networking](#7-networking)
8. [Security Hardening](#8-security-hardening)
9. [Container & Cloud](#9-container--cloud)
10. [Repository Structure](#10-repository-structure)
11. [Build Recipes](#11-build-recipe-format)
12. [Open Questions](#12-open-questions--future-work)

---

## 1. Philosophy & License Policy

jonerix exists because permissive licensing matters for infrastructure. Operators should be able to inspect, modify, and redistribute every piece of their OS without copyleft obligations. The rules are simple:

| Rule | Detail |
|------|--------|
| **Runtime** | Every binary, library, config, and script on the running system must be permissive (MIT, BSD, ISC, Apache-2.0, 0BSD, CC0, public domain). |
| **Kernel exception** | Linux (GPLv2) is the sole exception. It ships as a single blob under `/boot`. |
| **Build time** | GPL tools (GCC, GNU make, bash) are permitted in Alpine build containers. They are scaffolding — they never appear in the final image. |
| **Self-hosting** | The final system must be able to rebuild itself from source using only its own (permissive) tools. The `jonerix:builder` image proves this. |

### Why not just use a BSD?

FreeBSD/OpenBSD are excellent. jonerix targets a different niche:

- **Linux kernel**: vastly wider hardware support, mature cgroups/namespaces for containers, better cloud driver ecosystem.
- **musl + toybox**: smaller than BSD base, competitive with Alpine's footprint.
- **Familiarity**: Linux syscall ABI, `/proc`, `/sys`, OCI compatibility — no porting friction for server workloads.

---

## 2. Multi-Stage Bootstrap

jonerix is bootstrapped from Alpine Linux using jpkg (the custom package manager) and per-package `recipe.toml` build recipes. Alpine is used only as a build host — nothing from it enters the final image.

```
 Alpine build host               jpkg packages                        Final rootfs
┌──────────────────┐          ┌─────────────────────┐            ┌──────────────────┐
│ Alpine + clang   │──build──▶│ .jpkg archives      │──install──▶│ Pure permissive  │
│ jpkg, build deps │          │ both arches, signed │            │ system. No GPL.  │
└──────────────────┘          └─────────────────────┘            └──────────────────┘
     GPL is OK                  from-source builds                   no GPL at all
```

### Image Chain

jonerix produces a layered set of Docker images, each built from the previous:

```
minimal -> core -> builder   (compilers + dev tools)
                -> router    (networking appliance)
```

| Image | Base | Contents |
|-------|------|----------|
| `minimal` | scratch | musl, toybox, dropbear, curl, libressl, openrc, jpkg |
| `core` | minimal | mksh, uutils, micro, fastfetch, ripgrep, gitoxide, ncurses, networking |
| `builder` | core | clang/llvm, rust, go, nodejs, python3, cmake, bmake, samurai, perl |
| `router` | core | hostapd, wpa_supplicant, btop, unbound DNS config, sysctl hardening |

### Build Pipeline

1. **jpkg** is built from C source in an Alpine container (the only GPL build-time dependency)
2. **Packages** are built from source using `bootstrap/build-all.sh` and per-package `recipe.toml` recipes in `packages/{core,develop,extra}/`
3. **Images** are assembled using Dockerfiles that install packages via `jpkg install`
4. **CI** builds and publishes all images and packages for both x86_64 and aarch64

### From-Source Builds

All 60+ packages build from source on jonerix itself (both x86_64 and aarch64):

- **C/C++**: musl, toybox, LibreSSL, curl, dropbear, OpenRC, ncurses, etc.
- **LLVM/Clang/LLD**: Full compiler toolchain from source
- **Go chain**: C → Go 1.4 → 1.17 → 1.20 → 1.22 → 1.24 → 1.26 (bootstrapped from C)
- **Go packages**: containerd, runc, nerdctl, CNI plugins, headscale, derper
- **Rust packages**: uutils, gitoxide, ripgrep, btop (via system LLVM + bootstrap rustc)
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

## 3. Core Component Stack

| Role | Component | License | Replaces |
|------|-----------|---------|----------|
| Kernel | Linux | GPLv2 (sole exception) | — |
| C library | musl | MIT | glibc (LGPL) |
| Compiler | LLVM/Clang + lld | Apache-2.0 w/ LLVM exception | GCC (GPL) |
| Coreutils | toybox + uutils | 0BSD / MIT | BusyBox (GPL), GNU coreutils (GPL) |
| Shell | mksh | MirOS (ISC-like) | bash (GPL) |
| Init system | OpenRC | BSD-2-Clause | systemd (LGPL) |
| Privilege escalation | doas | ISC | sudo |
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
| Build tool (make) | bmake | MIT | GNU make (GPL) |
| Text editor | micro | MIT | vim |
| Grep | ripgrep | MIT | GNU grep (GPL) |
| Git | gitoxide | MIT/Apache-2.0 | git (GPL) |
| System info | fastfetch | MIT | neofetch |
| Process monitor | btop | Apache-2.0 | htop |
| awk | onetrueawk | MIT | gawk (GPL) |

### Notable Absences and Why

**GNU make**: Replaced by `bmake` (MIT, BSD make) for most builds and `samurai` (Apache-2.0, ninja-compatible) for cmake projects. A few upstream projects (Ruby, hostapd, wpa_supplicant) still require GNU make — these are built in Alpine containers at build time only.

**bash**: mksh (MirOS) is the runtime shell. It is POSIX-compliant and handles all shell scripting needs. Bash is only used at build time inside Alpine containers for projects that require it (e.g., toybox's genconfig.sh).

**gzip**: The gzip *format* is open. `pigz` (zlib license) handles `.gz` files. `zstd` is the preferred compression.

**systemd**: OpenRC is simpler, BSD-licensed, and proven in Alpine. The service file format is plain shell.

---

## 4. Package Manager: jpkg

`jpkg` is a custom, MIT-licensed package manager purpose-built for jonerix. It is intentionally minimal.

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

## 5. Filesystem Layout

Merged `/usr` — all binaries live in `/bin`, all libraries in `/lib`. Symlinks for compatibility:

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
│   ├── doas.conf     ← privilege escalation rules
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

## 6. Boot Sequence

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

### BIOS Legacy (fallback)

For BIOS systems, syslinux (GPL) is used on the **boot media only** — it is not installed to the root filesystem. Once the kernel is loaded, syslinux is no longer running. This is analogous to how the UEFI firmware itself is proprietary but doesn't affect the OS license.

---

## 7. Networking

| Function | Tool | License |
|----------|------|---------|
| Interface config | ifupdown-ng | ISC |
| DHCP client | dhcpcd | BSD-2-Clause |
| DNS resolver | unbound (recursive, DNSSEC) | BSD-3-Clause |
| HTTP/HTTPS client | curl | curl (MIT-like) |
| SSH | dropbear | MIT |
| WiFi AP | hostapd | BSD-3-Clause |
| WiFi client | wpa_supplicant | BSD-3-Clause |
| VPN relay | derper (Tailscale DERP) | BSD-3-Clause |
| Firewall | *see below* | — |

### Router Image

The `jonerix:router` image extends core with networking packages and ships default configs:

- **Unbound**: recursive DNS with DNSSEC, listens on LAN, rebind protection
- **Hostapd**: WiFi access point (WPA2, disabled by default — user must configure SSID/passphrase)
- **Network interfaces**: WAN (eth0, DHCP) + LAN (eth1, static 192.168.1.1/24)
- **Sysctl hardening**: IP forwarding enabled, SYN cookies, ICMP redirect rejection, reverse path filtering
- 
---

## 8. Security Hardening

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
- **doas**: Minimal config. Default: users in `wheel` group can elevate. No `NOPASSWD` by default.
- **System accounts**: daemon, bin, sys with nologin. Separate groups for tty, disk access.
- **Kernel hardening**: `kernel.kptr_restrict=2`, `kernel.dmesg_restrict=1`, `kernel.unprivileged_bpf_disabled=1`.
- **Router sysctl**: SYN cookies, ICMP redirect rejection, reverse path filtering, no source routing.

---

## 9. Container & Cloud

### Image Chain

```sh
# Pull from GHCR
docker pull ghcr.io/stormj-uh/jonerix:minimal   # base: toybox, dropbear, openrc
docker pull ghcr.io/stormj-uh/jonerix:core       # runtime: mksh, micro, ripgrep, networking
docker pull ghcr.io/stormj-uh/jonerix:builder    # dev: core + clang, rust, go, python3
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

Package builds (`publish-packages.yml`) compile recipes inside `jonerix:all` containers and upload `.jpkg` files to GitHub Releases. The INDEX is regenerated and Ed25519-signed after each build.

---

## 10. Repository Structure

```
jonerix/
├── DESIGN.md                ← this document
├── README.md                ← quick start and overview
├── TODO.md                  ← current status and remaining work
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
├── bootstrap/
│   ├── build-all.sh         ← build all packages from source in dependency order
│   ├── build-order.txt      ← 10-tier dependency ordering (60+ packages)
│   └── JONERIX-BUILD-ENVIRONMENT.md  ← build environment reference
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
│                               securetty, doas.conf, os-release, fastfetch/
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
├── tools/
│   └── bsdtar-static-aarch64 ← static bsdtar fallback for CI
│
├── sources/                 ← vendored source tarballs
│
├── .github/
│   └── workflows/
│       ├── publish-images.yml    ← CI: build + push Docker images
│       ├── publish-packages.yml  ← CI: build + upload jpkg packages
│       ├── bootstrap-packages.yml
│       ├── license-check.yml
│       └── wsl-rootfs.yml
│
└── docs/
    ├── bootstrapping.md
    ├── packaging.md
    └── contributing.md
```

---

## 11. Build Recipe Format

Each package has a `recipe.toml` in `packages/{core,develop,extra}/`. Recipes contain metadata, source URL, build instructions, and dependencies.

```toml
# packages/core/toybox/recipe.toml
[package]
name = "toybox"
version = "0.8.11"
license = "0BSD"
pre_bootstrap = false
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

`bootstrap/build-all.sh` processes recipes in dependency order from `bootstrap/build-order.txt`, searching across `packages/{core,develop,extra}/` for each recipe. jpkg sets `CC=clang`, `LD=ld.lld`, hardening flags, and an isolated `DESTDIR` automatically.

---

## 12. Open Questions & Future Work

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

### In Progress

- [ ] Build Ruby 3.4.3 jpkg (license unblocked, needs GNU make in Alpine)
- [ ] Build remaining x86_64 packages (btop, sqlite, npm, libatomic, unzip)
- [ ] Linux kernel recipe + custom build
- [ ] Bootloader for bare metal (EFISTUB + fallback)

### Future

- [ ] Write `cloud-init-lite` — metadata-driven instance setup (~500 lines of sh)
- [ ] **Desktop variant** — Wayland (MIT) + wlroots (MIT) + foot terminal (MIT) + Sway (MIT). All permissive.
- [ ] **Raspberry Pi** support (device tree, kernel config)
- [ ] **Java bootstrap** chain
- [ ] **Secure Boot** — Sign the EFISTUB kernel with a custom MOK key. No GPL bootloader needed.
- [ ] **Reproducible builds** — Deterministic timestamps, sorted tar entries, fixed locale.

### Known Compromises

| Item | Status | Notes |
|------|--------|-------|
| Linux kernel | GPLv2 — accepted | No permissive OS kernel with equivalent hardware/container support exists |
| CA certificates | Mozilla bundle, MPL-2.0 | This is *data*, not *code*. MPL-2.0 now in jpkg allowlist. |
| BIOS bootloader | syslinux (GPL) on boot media only | Not installed to rootfs. UEFI EFISTUB avoids this entirely. |
| GNU make at build time | GPL, Alpine container only | Ruby, hostapd, wpa_supplicant upstream Makefiles require it. Never shipped. |

---

## License

This document and all original jonerix code are released under the MIT License.

```
MIT License

Copyright (c) 2026 Jon-Erik G. Storm, Inc. DBA Lava Goat Software

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
