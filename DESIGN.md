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
| **Build time** | GPL tools (GCC, GNU make, apk-tools, BusyBox) are permitted during bootstrap. They are scaffolding — they never appear in the final image. |
| **Self-hosting** | The final system must be able to rebuild itself from source using only its own (permissive) tools. This is the ultimate proof that no GPL leaked in. |

### Why not just use a BSD?

FreeBSD/OpenBSD are excellent. jonerix targets a different niche:

- **Linux kernel**: vastly wider hardware support, mature cgroups/namespaces for containers, better cloud driver ecosystem.
- **musl + toybox**: smaller than BSD base, competitive with Alpine's footprint.
- **Familiarity**: Linux syscall ABI, `/proc`, `/sys`, OCI compatibility — no porting friction for server workloads.

---

## 2. Multi-Stage Bootstrap

jonerix is bootstrapped from Alpine Linux. Alpine already uses musl and OpenRC, making it the ideal host for cross-compiling a permissive userland. The bootstrap has four stages:

```
Stage 0 (Alpine host)        Stage 1 (cross-build)        Stage 2 (jonerix rootfs)
┌──────────────────┐         ┌───────────────────┐        ┌───────────────────┐
│ Alpine minimal   │──build──▶│ clang/musl sysroot│──pack──▶│ Pure permissive  │
│ apk, busybox,    │         │ toybox, mksh,      │        │ system. No GPL.  │
│ gcc, abuild      │         │ OpenRC, LibreSSL...│        │ Self-hosting.    │
└──────────────────┘         └───────────────────┘        └───────────────────┘
       GPL is OK                  mixed toolchain               no GPL at all
```

### Stage 0 — Alpine Build Host

Pull `alpine:latest` (Docker or raw rootfs). Install build dependencies:

```sh
apk add clang lld llvm-dev musl-dev cmake samurai git curl patch
```

This stage is throwaway. Nothing from it enters the final image.

### Stage 1 — Cross-Compile the Permissive World

Using Alpine's tools, build every jonerix component from source into a staging sysroot (`/jonerix-sysroot`). Build order matters — dependencies first:

```
1. musl              (C library — everything links against this)
2. zstd, lz4         (compression — needed by jpkg and kernel)
3. zlib              (compression — needed by Python, Node.js, pigz, etc.)
4. LibreSSL          (TLS — needed by curl, dropbear, Python, Node.js)
5. toybox            (coreutils — ls, cp, cat, grep, sed, awk, tar, ...)
6. mksh              (shell)
7. samurai           (ninja-compatible build tool, Apache-2.0)
8. LLVM/Clang/lld/libc++/libc++abi (compiler + linker + C++ stdlib)
9. OpenRC            (init system)
10. dropbear         (SSH)
11. curl             (HTTP client)
12. dhcpcd           (DHCP)
13. unbound          (DNS resolver)
14. doas             (privilege escalation)
15. socklog          (logging)
16. snooze           (cron)
17. mandoc           (man pages)
18. ifupdown-ng      (network config)
19. jpkg             (package manager)
20. pigz             (parallel gzip, zlib license)
21. nvi              (text editor)
22. Python 3         (interpreter + build dep for Node.js)
23. Node.js          (JavaScript runtime — needs Python 3, libc++)
```

All components are compiled with:
```sh
CC=clang
LD=ld.lld
CFLAGS="-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2"
LDFLAGS="-Wl,-z,relro,-z,now -pie"
```

### Stage 2 — Assemble the Root Filesystem

Copy Stage 1 artifacts into a clean rootfs. Create directory structure, install configs, set permissions. Generate the jpkg package database. Produce:

- A root filesystem tarball (`jonerix-rootfs-<version>.tar.zst`)
- A bootable disk image (`jonerix-<version>.img`) with EFISTUB
- An OCI container image (`jonerix-<version>-oci.tar`)

No Alpine bits carry over. Verification: `find /jonerix-rootfs -type f -exec file {} \;` should show only musl-linked ELF binaries and plain text configs.

### Stage 3 — Self-Hosting Verification

Boot the Stage 2 image. From within jonerix, rebuild the entire system from source:

```sh
jpkg build-world
```

If the output is bit-for-bit identical to the Stage 2 rootfs (reproducible build), the bootstrap is proven and the system is fully self-hosting with zero GPL runtime dependencies.

---

## 3. Core Component Stack

| Role | Component | License | Replaces |
|------|-----------|---------|----------|
| Kernel | Linux | GPLv2 (sole exception) | — |
| C library | musl | MIT | glibc (LGPL) |
| Compiler | LLVM/Clang + lld | Apache-2.0 w/ LLVM exception | GCC (GPL) |
| Coreutils | toybox | 0BSD | BusyBox (GPL), GNU coreutils (GPL) |
| Shell | mksh | MirOS (ISC-like) | bash (GPL) |
| Init system | OpenRC | BSD-2-Clause | systemd (LGPL) |
| Privilege escalation | doas | ISC | sudo |
| TLS library | LibreSSL | ISC + OpenSSL legacy | — |
| SSH server | dropbear | MIT | — |
| HTTP client | curl | curl license (MIT-like) | wget (GPL) |
| DNS resolver | unbound | BSD-3-Clause | — |
| DHCP client | dhcpcd | BSD-2-Clause | — |
| Logging | socklog | BSD-3-Clause | syslog-ng (GPL) |
| Cron | snooze | CC0 (public domain) | cronie (GPL) |
| Man pages | mandoc | ISC | groff (GPL) |
| Network config | ifupdown-ng | ISC | — |
| Bootloader | EFISTUB (kernel feature) | n/a | GRUB (GPL) |
| Package manager | jpkg (custom) | MIT | apk-tools (GPL) |
| Compression | zstd (BSD), lz4 (BSD), zlib (zlib), pigz (zlib) | BSD / zlib | gzip (GPL) |
| C++ standard library | libc++ / libc++abi | Apache-2.0 w/ LLVM exception | libstdc++ (GPL) |
| Scripting / build tool | Python 3 | PSF-2.0 | — |
| JavaScript runtime | Node.js | MIT | — |
| Build tool | samurai | Apache-2.0 | GNU make (GPL) |
| Text editor | nvi | BSD | vim |
| grep/sed/awk | toybox builtins | 0BSD | GNU versions (GPL) |

### Notable Absences and Why

**GNU make**: Replaced by `samurai` (ninja-compatible) for most builds. For Makefile-based projects, BSD `make` (bmake) is MIT-licensed.

**bash**: mksh is POSIX-compliant and supports most bashisms via compatibility mode. Shell scripts in jonerix target POSIX `sh`.

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
│ ────────────────────────
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
build = ["clang", "samurai"]

[files]
sha256 = "abc123..."
size = 245760
```

### Repository Layout

A repository is a static HTTPS directory:

```
https://pkg.jonerix.org/v1/x86_64/
├── INDEX.zst          ← signed manifest of all packages + versions + hashes
├── INDEX.zst.sig      ← Ed25519 signature
├── toybox-0.8.11.jpkg
├── mksh-59c.jpkg
├── openrc-0.54.jpkg
└── ...
```

No database server required. A static file host (S3, GitHub Releases, nginx) is sufficient.

### Commands

```sh
jpkg update                  # fetch INDEX from mirrors
jpkg install <pkg>           # install package + deps
jpkg remove <pkg>            # remove package
jpkg upgrade                 # upgrade all installed packages
jpkg search <query>          # search package names/descriptions
jpkg info <pkg>              # show package metadata
jpkg build <recipe-dir>      # build package from source recipe
jpkg build-world             # rebuild entire system from source
jpkg verify                  # check installed files against manifests
jpkg license-audit           # verify all installed packages are permissive
```

### Signing

Packages and the INDEX manifest are signed with Ed25519 using `tweetnacl` (public domain) or LibreSSL. The distribution's public key is embedded in jpkg at compile time.

### Implementation

Written in C, linked statically against musl. Dependencies: LibreSSL (for HTTPS fetches), zstd (decompression), tweetnacl (signature verification). Total binary size target: < 500KB.

---

## 5. Filesystem Layout

Merged `/usr` — all binaries live in `/bin`, all libraries in `/lib`. Symlinks for compatibility:

```
/
├── bin/              ← all executables (merged /usr/bin + /sbin)
├── lib/              ← all libraries (merged /usr/lib)
├── etc/              ← system configuration
│   ├── init.d/       ← OpenRC service scripts
│   ├── conf.d/       ← OpenRC service config
│   ├── ssl/          ← LibreSSL certificates
│   ├── network/      ← ifupdown-ng interfaces
│   └── jpkg/         ← package manager config + keys
├── var/
│   ├── log/          ← socklog output
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
      - socklog (logging)
      - snooze (cron jobs)
  → agetty spawns on tty1
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
| DNS resolver | unbound (recursive) or resolv.conf (stub) | BSD-3-Clause |
| HTTP/HTTPS client | curl | curl (MIT-like) |
| SSH | dropbear | MIT |
| Firewall | *see below* | — |

### The Firewall Gap

The `nft` CLI (nftables userspace) is GPL-2.0+. The kernel's netfilter subsystem is part of the kernel (GPLv2, accepted). Options:

1. **jnft** — Write a minimal MIT-licensed CLI that communicates with the kernel via the nftables netlink API (`libnftnl` is GPL, so we'd talk netlink directly). This is ~2-3K lines of C for basic rule management. Ship this in jonerix v1.
2. **BPF-based filtering** — Use eBPF programs for packet filtering. Tools like `bpfilter` aim to provide a permissive userspace. This is the long-term direction.
3. **Cloud-only** — For cloud VMs, security groups replace host firewalls. Document this as a valid v1 deployment model.

**jonerix v1 recommendation**: Ship `jnft` (option 1) for basic stateful firewall rules, plus document option 3 for cloud deployments.

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
| `-fvisibility=hidden` | Minimize exported symbols |
| LLVM CFI | Control-flow integrity (when supported) |

### Runtime Defaults

- **No root SSH**: `dropbear` configured with `DROPBEAR_OPTS="-w"` (disable root login).
- **doas**: Minimal config. Default: users in `wheel` group can elevate. No `NOPASSWD` by default.
- **Kernel hardening**: `kernel.kptr_restrict=2`, `kernel.dmesg_restrict=1`, `kernel.unprivileged_bpf_disabled=1`.
- **Minimal kernel config**: Only enable subsystems needed for the target platform. No USB, no sound, no graphics for server builds.
- **Reproducible builds**: Deterministic compilation flags, fixed timestamps, sorted file lists.

---

## 9. Container & Cloud

### OCI Base Image

```dockerfile
# Built by image/oci.sh — NOT a Dockerfile (no Docker needed at build time)
# Resulting image: ~8-15 MB
FROM scratch
ADD jonerix-rootfs.tar.zst /
ENTRYPOINT ["/bin/mksh"]
```

The OCI image includes: musl, toybox, mksh, curl, LibreSSL certs, jpkg. Everything needed to `jpkg install` additional packages.

Target sizes:
- **Minimal rootfs**: ~8 MB (toybox + mksh + musl + jpkg)
- **Server image**: ~80 MB (adds dropbear, curl, OpenRC, socklog, Python 3, Node.js)
- **Full development**: ~600 MB (adds LLVM/Clang, libc++)

### Cloud Images

| Platform | Format | Init Integration |
|----------|--------|------------------|
| AWS | AMI (raw → EBS snapshot) | tiny-ec2-bootstrap (MIT) |
| GCP | .tar.gz (raw disk image) | GCP guest agent (Apache-2.0) |
| Generic | .qcow2 / .img | OpenRC + cloud-init-lite (custom, MIT) |

`cloud-init-lite`: A ~500-line shell script that reads metadata from the platform's metadata service (169.254.169.254), sets hostname, injects SSH keys, and runs user-data scripts. No Python dependency.

### GitHub Actions CI

```yaml
# .github/workflows/build.yml
name: Build jonerix
on: [push, pull_request]
jobs:
  build:
    runs-on: ubuntu-latest
    container: alpine:latest
    steps:
      - uses: actions/checkout@v4
      - run: sh bootstrap/stage0.sh
      - run: sh bootstrap/stage1.sh
      - run: sh bootstrap/stage2.sh
      - uses: actions/upload-artifact@v4
        with:
          name: jonerix-rootfs
          path: output/jonerix-rootfs-*.tar.zst
```

---

## 10. Repository Structure

```
jonerix/
├── DESIGN.md                ← this document
├── LICENSE                  ← MIT
├── Makefile                 ← top-level: make bootstrap, make image, make oci
│
├── bootstrap/
│   ├── stage0.sh            ← install Alpine build deps
│   ├── stage1.sh            ← cross-compile all components
│   ├── stage2.sh            ← assemble clean rootfs
│   ├── stage3-verify.sh     ← self-hosting rebuild check
│   └── config.sh            ← shared vars: versions, hashes, flags
│
├── packages/
│   ├── rules.mk             ← shared build rules (fetch, extract, patch)
│   ├── core/
│   │   ├── musl/
│   │   │   ├── Makefile      ← build recipe
│   │   │   └── patches/      ← any needed patches
│   │   ├── toybox/
│   │   │   ├── Makefile
│   │   │   └── toybox.config ← enabled applets
│   │   ├── mksh/
│   │   ├── openrc/
│   │   ├── llvm/
│   │   ├── libressl/
│   │   ├── dropbear/
│   │   ├── curl/
│   │   ├── dhcpcd/
│   │   ├── unbound/
│   │   ├── doas/
│   │   ├── socklog/
│   │   ├── snooze/
│   │   ├── mandoc/
│   │   ├── ifupdown-ng/
│   │   ├── pigz/
│   │   ├── zstd/
│   │   ├── zlib/
│   │   ├── lz4/
│   │   ├── samurai/
│   │   ├── nvi/
│   │   ├── python3/
│   │   ├── nodejs/
│   │   └── linux/            ← kernel config + build
│   └── jpkg/                 ← the package manager source code
│       ├── src/
│       ├── Makefile
│       └── tests/
│
├── config/
│   ├── kernel/
│   │   ├── x86_64.config     ← minimal server kernel config
│   │   └── aarch64.config
│   ├── openrc/
│   │   ├── inittab
│   │   └── init.d/           ← service scripts (sshd, socklog, etc.)
│   └── defaults/
│       ├── etc/
│       │   ├── hostname
│       │   ├── resolv.conf
│       │   ├── passwd
│       │   ├── group
│       │   ├── shadow
│       │   ├── shells
│       │   ├── profile       ← mksh login profile
│       │   ├── doas.conf
│       │   └── ssl/
│       │       └── cert.pem  ← CA bundle (Mozilla, MPL — data not code)
│       └── var/
│           └── db/jpkg/
│
├── image/
│   ├── mkimage.sh            ← bootable disk image (GPT + ESP + root)
│   ├── oci.sh                ← OCI container image
│   └── cloud/
│       ├── aws-ami.sh
│       ├── gcp-image.sh
│       └── cloud-init-lite.sh
│
├── scripts/
│   ├── license-audit.sh      ← verify all components are permissive
│   └── size-report.sh        ← measure rootfs size breakdown
│
├── .github/
│   └── workflows/
│       ├── build.yml          ← CI: full bootstrap
│       └── license-check.yml  ← CI: automated license audit
│
└── docs/
    ├── bootstrapping.md
    ├── packaging.md
    └── contributing.md
```

---

## 11. Build Recipe Format

Each package under `packages/core/` has a `Makefile` that follows a standard pattern:

```makefile
# packages/core/toybox/Makefile
PKG_NAME     = toybox
PKG_VERSION  = 0.8.11
PKG_LICENSE  = 0BSD
PKG_SOURCE   = https://github.com/landley/toybox/archive/$(PKG_VERSION).tar.gz
PKG_SHA256   = <sha256-of-tarball>

include ../../rules.mk

configure:
	cp $(PKG_DIR)/toybox.config $(SRC_DIR)/.config

build:
	$(MAKE) -C $(SRC_DIR) CC="$(CC)" CFLAGS="$(CFLAGS)" LDFLAGS="$(LDFLAGS)"

install:
	$(MAKE) -C $(SRC_DIR) PREFIX=$(DESTDIR) install
```

`rules.mk` provides:
- `fetch` — download + verify SHA256
- `extract` — unpack tarball
- `patch` — apply patches from `patches/` directory
- `clean` — remove build artifacts
- Variables: `$(CC)`, `$(CFLAGS)`, `$(LDFLAGS)`, `$(DESTDIR)`, `$(SRC_DIR)`

### License Gate

`rules.mk` includes an automatic license check. If `PKG_LICENSE` contains `GPL`, `LGPL`, or `AGPL`, the build aborts:

```makefile
# rules.mk (excerpt)
FORBIDDEN_LICENSES = GPL LGPL AGPL
$(foreach lic,$(FORBIDDEN_LICENSES),\
  $(if $(findstring $(lic),$(PKG_LICENSE)),\
    $(error BLOCKED: $(PKG_NAME) is $(PKG_LICENSE) — not permitted in jonerix)))
```

---

## 12. Open Questions & Future Work

### v1 Scope (server-focused)

- [ ] Write `jpkg` — the custom package manager (~3-5K lines of C)
- [ ] Write `jnft` — minimal netlink-based firewall CLI (~2-3K lines of C)
- [ ] Write `cloud-init-lite` — metadata-driven instance setup (~500 lines of sh)
- [ ] Produce working Stage 0→2 bootstrap scripts
- [ ] Achieve self-hosting (Stage 3)
- [ ] Publish OCI base image
- [ ] CI pipeline on GitHub Actions

### Future

- **aarch64 support** — LLVM cross-compilation makes this straightforward.
- **Desktop variant** — Wayland (MIT) + wlroots (MIT) + foot terminal (MIT) + Sway (MIT). All permissive.
- **Rust/Zig ecosystem** — Many modern CLI tools (ripgrep, fd, bat, tokei) are MIT-licensed. Could offer a `jonerix-extras` package set.
- **Secure Boot** — Sign the EFISTUB kernel with a custom MOK key. No GPL bootloader needed.
- **Reproducible builds** — Deterministic timestamps, sorted tar entries, fixed locale. Goal: bit-for-bit identical output from any build host.

### Known Compromises

| Item | Status | Notes |
|------|--------|-------|
| Linux kernel | GPLv2 — accepted | No permissive OS kernel with equivalent hardware/container support exists |
| CA certificates | Mozilla bundle, MPL-2.0 | This is *data*, not *code*. Widely considered acceptable. |
| BIOS bootloader | syslinux (GPL) on boot media only | Not installed to rootfs. UEFI EFISTUB avoids this entirely. |
| LibreSSL legacy code | OpenSSL/SSLeay license (permissive but quirky) | New code is ISC. The legacy license is functionally permissive. |

---

## License

This document and all original jonerix code are released under the MIT License.

```
MIT License

Copyright (c) 2026 jonerix contributors

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
