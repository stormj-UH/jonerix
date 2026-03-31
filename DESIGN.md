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

jonerix is bootstrapped from Alpine Linux using jpkg (the custom package manager) and per-package `recipe.toml` build recipes. Alpine is used only as a build host — nothing from it enters the final image.

```
Alpine build host              jpkg packages              Final rootfs
┌──────────────────┐         ┌───────────────────┐      ┌───────────────────┐
│ Alpine + clang   │──build──▶│ .jpkg archives    │──install──▶│ Pure permissive  │
│ jpkg, build deps │         │ both arches, signed│      │ system. No GPL.  │
└──────────────────┘         └───────────────────┘      └───────────────────┘
       GPL is OK                 from-source builds           no GPL at all
```

### Build Pipeline

1. **jpkg** is built from C source in an Alpine container (the only GPL build-time dependency)
2. **Packages** are built from source using `bootstrap/build-all.sh` and per-package `packages/bootstrap/*/recipe.toml` recipes inside a jonerix-develop container
3. **Images** are assembled using Dockerfiles that install packages via `jpkg install`

### From-Source Builds

All 40+ packages build from source on jonerix itself (both x86_64 and aarch64):

- **C/C++**: musl, toybox, OpenSSL, curl, dropbear, OpenRC, ncurses, etc.
- **LLVM/Clang/LLD**: Full compiler toolchain from source
- **Go chain**: C → Go 1.4 → 1.17 → 1.20 → 1.22 → 1.24 → 1.26 (bootstrapped from C)
- **Go packages**: containerd, runc, nerdctl, CNI plugins (CGO_ENABLED=0)
- **Rust packages**: uutils, gitoxide, ripgrep (via system LLVM + bootstrap rustc)
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
| Coreutils | toybox | 0BSD | BusyBox (GPL), GNU coreutils (GPL) |
| Shell | mksh | MirOS (ISC-like) | bash (GPL) |
| Init system | OpenRC | BSD-2-Clause | systemd (LGPL) |
| Privilege escalation | doas | ISC | sudo |
| TLS library | OpenSSL | Apache-2.0 | — |
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
| Text editor | micro | MIT | vim |
| grep/sed/awk | toybox builtins | 0BSD | GNU versions (GPL) |

### Notable Absences and Why

**GNU make**: Replaced by `samurai` (ninja-compatible) for most builds. For Makefile-based projects, BSD `make` (bmake) is MIT-licensed.

**bash**: zsh is bash-compatible and supports most bashisms via compatibility mode. Shell scripts in jonerix target POSIX `sh`.

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

Packages and the INDEX manifest are signed with Ed25519 using `tweetnacl` (public domain) or OpenSSL. The distribution's public key is embedded in jpkg at compile time.

### Implementation

Written in C, linked statically against musl. Dependencies: OpenSSL (for HTTPS fetches), zstd (decompression), tweetnacl (signature verification).

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
│   ├── ssl/          ← TLS certificates
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
  → User logs in → zsh (develop) / sh (minimal)
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
ENTRYPOINT ["/bin/sh"]
```

The OCI image includes: musl, toybox, curl, OpenSSL, ca-certificates, jpkg. Everything needed to `jpkg install` additional packages.

Target sizes:
- **Minimal rootfs**: ~8 MB (toybox + musl + jpkg)
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

CI builds packages from source using `bootstrap/build-all.sh` inside a jonerix-develop container, then uploads `.jpkg` archives to GitHub Releases for both x86_64 and aarch64.

---

## 10. Repository Structure

```
jonerix/
├── DESIGN.md                ← this document
├── LICENSE                  ← MIT
├── Dockerfile               ← full image (jpkg-based assembly)
├── Dockerfile.minimal       ← minimal runtime (shell, init, SSH)
├── Dockerfile.develop       ← development environment (compilers, languages)
│
├── bootstrap/
│   ├── build-all.sh         ← build all packages from source in dependency order
│   └── JONERIX-BUILD-ENVIRONMENT.md
│
├── packages/
│   ├── core/
│   │   ├── musl/
│   │   │   └── recipe.toml  ← package metadata + deps
│   │   ├── toybox/
│   │   ├── mksh/
│   │   ├── openrc/
│   │   ├── llvm/
│   │   ├── openssl/
│   │   ├── ... (40+ packages)
│   │   └── ca-certificates/
│   ├── bootstrap/
│   │   ├── musl/
│   │   │   └── recipe.toml  ← from-source build recipe
│   │   ├── toybox/
│   │   ├── llvm/
│   │   ├── go/
│   │   ├── ... (40+ packages)
│   │   └── cni-plugins/
│   └── jpkg/                ← the package manager source code
│       ├── *.c / *.h
│       └── Makefile
│
├── config/
│   ├── openrc/
│   │   └── init.d/          ← service scripts (sshd, socklog, etc.)
│   └── defaults/
│       └── etc/             ← hostname, passwd, group, profile, etc.
│
├── scripts/
│   └── license-audit.sh     ← verify all components are permissive
│
├── .github/
│   └── workflows/
│       ├── publish-packages.yml  ← CI: build + upload jpkg packages
│       └── bootstrap-packages.yml
│
└── docs/
    ├── bootstrapping.md
    ├── packaging.md
    └── contributing.md
```

---

## 11. Build Recipe Format

Each package has a `recipe.toml` file. Core recipes (`packages/core/`) hold metadata and dependencies. Bootstrap recipes (`packages/bootstrap/`) contain from-source build instructions.

```toml
# packages/bootstrap/toybox/recipe.toml
[package]
name = "toybox"
version = "0.8.11"
license = "0BSD"
description = "BSD-licensed replacement for BusyBox"

[source]
url = "https://github.com/landley/toybox/archive/refs/tags/0.8.11.tar.gz"
sha256 = "..."

[build]
system = "custom"
configure = "..."
build = "CC=clang make -j$(nproc)"
install = "make PREFIX=$DESTDIR install"

[depends]
runtime = ["musl"]
build = ["clang"]
```

`bootstrap/build-all.sh` processes recipes in dependency order, building each package inside a jonerix-develop container with `CC=clang`, `LD=ld.lld`, and hardening flags.

---

## 12. Open Questions & Future Work

### v1 Scope (server-focused)

- [x] Write `jpkg` — custom package manager (C, MIT)
- [ ] Write `jnft` — minimal netlink-based firewall CLI (~2-3K lines of C)
- [ ] Write `cloud-init-lite` — metadata-driven instance setup (~500 lines of sh)
- [x] Full from-source bootstrap (40+ packages, both arches)
- [x] Self-hosting (jonerix rebuilds itself from source)
- [x] Publish OCI base images (minimal, develop)
- [x] CI pipeline on GitHub Actions
- [x] aarch64 support (all packages build on both x86_64 and aarch64)

### Future

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
| OpenSSL | Apache-2.0 (since v3.0) | Fully permissive. |

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
