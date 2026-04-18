# jonerix

```
   _                       _
  (_) ___  _ __   ___ _ __(_)_  __
  | |/ _ \| '_ \ / _ \ '__| \ \/ /
  | | (_) | | | |  __/ |  | |>  <
 _/ |\___/|_| |_|\___|_|  |_/_/\_\
|__/
======= permissive + linux =======
```

**A fully self-hosting Linux distribution with zero GPL in userland.**

## Overview

jonerix is a Linux distribution built around a simple rule: every userland component must use a permissive license such as MIT, BSD, ISC, Apache-2.0, or public domain. The Linux kernel is not part of this distribution. This is a "Bring Your Own Kernel" (BYOK) distro. It is designed for use in containers, on WSL, or on Rasbperry Pi, but there are no limits.

100+ packages build from source on jonerix itself. The system compiles its own compiler (Clang/LLVM), its own languages (Go from C, Rust from a bootstrap binary), and its own container runtime. No GNU toolchain, no GCC, no GPL coreutils.

The point of jonerix is not moral instruction. It is not a sermon against copyleft, and it does not require anyone to agree with its premises. It is a distribution for people and organizations who want the lowest possible licensing friction in userland. If that use case does not matter to you, then jonerix is probably not for you.

### Self-Hosting

jonerix can rebuild itself from source using only the tools it ships:

- **C/C++**: Clang/LLVM/LLD built from source on jonerix
- **Go**: Full bootstrap chain from C source (C &rarr; Go 1.4 &rarr; 1.17 &rarr; 1.20 &rarr; 1.22 &rarr; 1.24 &rarr; 1.26)
- **Rust**: Built from source using system LLVM and a bootstrap rustc; targets the custom `aarch64-jonerix-linux-musl` / `x86_64-jonerix-linux-musl` triple with no GCC runtime (uses `llvm-libunwind = "system"` so `-lunwind` replaces `-lgcc_s` in proc-macro link lines)
- **Python 3 + Node.js**: Built from source with Clang/musl
- **Container runtime**: containerd + runc + nerdctl + CNI plugins, all from source

The `jonerix:builder` image installs these tools from jpkg packages. It compiles C, Go, and Rust programs out of the box.

## Quick Start

```sh
# Pull from GHCR (fastest)
docker pull ghcr.io/stormj-uh/jonerix:minimal   # base: toybox, dropbear, curl, libressl, openrc
docker pull ghcr.io/stormj-uh/jonerix:core       # runtime: mksh, uutils, micro, ripgrep, networking
docker pull ghcr.io/stormj-uh/jonerix:builder    # dev: core + clang/llvm, rust, go, nodejs, python3

# Per-arch tags: -amd64 and -arm64 are also available
docker run -it ghcr.io/stormj-uh/jonerix:core
docker run -it ghcr.io/stormj-uh/jonerix:builder
```

Or build locally:

```sh
# Minimal base image
docker build -f Dockerfile.minimal --tag jonerix:minimal .

# Core runtime image (FROM minimal)
docker build -f Dockerfile.core --tag jonerix:core .

# Builder dev image (FROM core)
docker build -f Dockerfile.builder --tag jonerix:builder .
```

## What's Inside

### Image Layers

| Image | Based on | Contents |
|-------|----------|----------|
| `minimal` | scratch | musl, toybox, dropbear, curl, libressl, openrc, jpkg |
| `core` | minimal | mksh, uutils, micro, fastfetch, ripgrep, gitoxide, networking tools |
| `builder` | core | clang/llvm, rust, go, nodejs, python3, cmake, jmake, samurai, perl |

### Core System

| Component | License | Role |
|-----------|---------|------|
| musl | MIT | C standard library |
| toybox | 0BSD | Base coreutils (ls, cp, cat, ...) |
| uutils | MIT | Extended coreutils (sort, wc, tr, ...) |
| mksh | MirOS | Shell (/bin/sh) — POSIX-compliant, musl-safe |
| jpkg | MIT | Package manager |
| OpenRC | BSD-2-Clause | Init system |
| dropbear | MIT | SSH server/client |
| bsdsed | BSD-2-Clause | sed (BSD implementation) |
| onetrueawk | MIT | awk (one true awk) |
| sudo | ISC | Privilege escalation (sudoers-based) |
| tzdata | Public-Domain+BSD-3-Clause | Time zone data |

### Compilers and Languages

| Component | License | Role |
|-----------|---------|------|
| Clang/LLVM/LLD | Apache-2.0 | C/C++ compiler, linker, toolchain |
| Rust | MIT/Apache-2.0 | Systems language (from source) |
| Go | BSD-3-Clause | Go language (bootstrapped from C) |
| Python 3 | PSF-2.0 | Scripting language |
| Node.js | MIT | JavaScript runtime |
| Perl | Artistic-2.0 | Scripting language |

### Build Tools

| Component | License | Role |
|-----------|---------|------|
| cmake | BSD-3-Clause | Build system generator |
| jmake 1.0.1 | MIT | Drop-in GNU make replacement (Rust) |
| samurai | Apache-2.0 | Ninja-compatible build tool |
| meson | Apache-2.0 | Build system (via pip) |
| flex | BSD-2-Clause | Lexer generator |
| byacc | Public Domain | Parser generator |
| bc | BSD-2-Clause | Calculator |

### Networking and Services

| Component | License | Role |
|-----------|---------|------|
| curl | MIT | HTTP client |
| LibreSSL | ISC | TLS library (OpenSSL fork) |
| pcre2 | BSD-3-Clause | Regular expressions library |
| nginx | BSD-2-Clause | HTTP server |
| unbound | BSD-3-Clause | DNS resolver |
| dhcpcd | BSD-2-Clause | DHCP client |
| ifupdown-ng | ISC | Network configuration |
| hostapd | BSD-3-Clause | Wi-Fi access point / WPA supplicant |

### Container Runtime

| Component | License | Role |
|-----------|---------|------|
| containerd | Apache-2.0 | Container runtime |
| runc | Apache-2.0 | OCI runtime |
| nerdctl | Apache-2.0 | Docker-compatible CLI |
| CNI plugins | Apache-2.0 | Container networking |

### Utilities

| Component | License | Role |
|-----------|---------|------|
| micro | MIT | Terminal text editor |
| gitoxide | MIT/Apache-2.0 | Git implementation in Rust |
| ripgrep | MIT | Fast recursive grep |
| mandoc | ISC | Man page tools |
| pigz | Zlib | Parallel gzip |
| bsdtar | BSD-2-Clause | Archive tool (libarchive) |
| doas | ISC | Privilege escalation |
| fastfetch | MIT | System information |
| openrsync | ISC | rsync-protocol-27-compatible drop-in (replaces GPL rsync) |
| jonerix-raspi5-fixups | MIT | Pi 5 hardware fixups (EEE disable, pwm-fan thermal control) |

## Package Manager (jpkg)

jpkg is a custom, MIT-licensed package manager built for jonerix. Packages are zstd-compressed tarballs signed with Ed25519.

```sh
jpkg update                # fetch latest package index
jpkg search fastfetch      # search available packages
jpkg install fastfetch     # install a package
jpkg list                  # list installed packages
```

Packages are hosted on GitHub Releases and built from source in CI for both x86_64 and aarch64.

## Architecture

### Merged /usr Layout

jonerix uses a merged `/usr` layout where `/usr` is a symlink to `/`. All binaries live in `/bin`, all libraries in `/lib`, all headers in `/include`

### Licensing Rule

Every package must carry a permissive license. GPL-3 tools like rsync are replaced by permissive equivalents (e.g. openrsync).

| Allowed | Not Allowed |
|---------|-------------|
| MIT, BSD-2-Clause, BSD-3-Clause | GPL, LGPL, AGPL |
| Apache-2.0, ISC, 0BSD | SSPL, EUPL |
| Zlib, PSF-2.0, Artistic-2.0 | CC-BY-SA |
| Public Domain, MirOS | Any copyleft |

## fastfetch

```
                                    root@jonerix-tormenta
   _                       _        ---------------------
  (_) ___  _ __   ___ _ __(_)_  __  OS -> jonerix 1.1.2 aarch64
  | |/ _ \| '_ \ / _ \ '__| \ \/ /  Kernel -> Linux 6.18.22-v8-16k+
  | | (_) | | | |  __/ |  | |>  <   Uptime -> 16 hours, 28 mins
 _/ |\___/|_| |_|\___|_|  |_/_/\_\  Packages -> 44 (jpkg)
|__/                                 Shell -> mksh
======= permissive + linux =======   Terminal -> dropbear
                                     Editor -> micro
                                     CPU -> BCM2712 (4) @ 2.40 GHz
                                     Memory -> 150.14 MiB / 3.95 GiB (4%)
                                     Disk (/) -> 8.87 GiB / 29.03 GiB (31%) - ext4
                                     Local IP (eth0) -> 10.0.0.8/16
```

## License

All original jonerix code is released under the **MIT License**.
