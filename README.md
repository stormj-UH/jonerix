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

jonerix is a Linux distribution built around a simple rule: every userland component must use a permissive license such as MIT, BSD, ISC, Apache-2.0, or public domain. The Linux kernel is the one exception.

46 packages build from source on jonerix itself. The system compiles its own compiler (Clang/LLVM), its own languages (Go from C, Rust from a bootstrap binary), and its own container runtime. No GNU toolchain, no GCC, no GPL coreutils.

The point of jonerix is not moral instruction. It is not a sermon against copyleft, and it does not require anyone to agree with its premises. It is a distribution for people and organizations who want the lowest possible licensing friction in userland. If that use case does not matter to you, then jonerix is probably not for you.

### Self-Hosting

jonerix can rebuild itself from source using only the tools it ships:

- **C/C++**: Clang/LLVM/LLD built from source on jonerix
- **Go**: Full bootstrap chain from C source (C &rarr; Go 1.4 &rarr; 1.17 &rarr; 1.20 &rarr; 1.22 &rarr; 1.24 &rarr; 1.26)
- **Rust**: Built from source using system LLVM and a bootstrap rustc
- **Python 3 + Node.js**: Built from source with Clang/musl
- **Container runtime**: containerd + runc + nerdctl + CNI plugins, all from source

The `jonerix:builder` image installs every tool from jpkg packages with no Alpine overlay. It compiles C, Go, and Rust programs out of the box.

## Quick Start

```sh
# Pull from GHCR (fastest)
docker pull ghcr.io/stormj-uh/jonerix:minimal   # base: toybox, dropbear, curl, openssl, openrc
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
| `minimal` | scratch | musl, toybox, dropbear, curl, openssl, openrc, jpkg |
| `core` | minimal | mksh, uutils, micro, fastfetch, ripgrep, gitoxide, networking tools |
| `builder` | core | clang/llvm, rust, go, nodejs, python3, cmake, bmake, samurai, perl |

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
| bmake | MIT | BSD make |
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

jonerix uses a merged `/usr` layout where `/usr` is a symlink to `/`. All binaries live in `/bin`, all libraries in `/lib`, all headers in `/include`.

### Bootstrap Process

1. **jpkg** is built from C source in an Alpine container (the only GPL build-time dependency)
2. **All packages** are installed from the jpkg repository into a clean rootfs via `Dockerfile.minimal`
3. **Final image** is assembled `FROM scratch` with zero GPL runtime components
4. **Self-hosting**: `jonerix:builder` contains a full toolchain (Clang 21, Go 1.26, Rust 1.94) capable of rebuilding every package from source. The cycle `jonerix:minimal → jonerix:builder → jonerix:minimal` is proven at v1.0.

Package recipes live in `packages/*/recipe.toml`. Build dependencies are declared explicitly; the package manager resolves and installs them in order.

### Licensing Rule

Every package must carry a permissive license:

| Allowed | Not Allowed |
|---------|-------------|
| MIT, BSD-2-Clause, BSD-3-Clause | GPL, LGPL, AGPL |
| Apache-2.0, ISC, 0BSD | SSPL, EUPL |
| Zlib, PSF-2.0, Artistic-2.0 | CC-BY-SA |
| Public Domain, MirOS | Any copyleft |

bash/GNU tools are used only at build time inside Alpine CI and never ship in the final image. mksh (MirOS) is the runtime shell (/bin/sh). zsh was removed because it deadlocks on musl libc with nested command substitutions.

## fastfetch

```
   _                       _        root@jonerix
  (_) ___  _ __   ___ _ __(_)_  __  -----------------
  | |/ _ \| '_ \ / _ \ '__| \ \/ /  OS -> jonerix 1.0 aarch64
  | | (_) | | | |  __/ |  | |>  <   Kernel -> Linux 6.18.5
 _/ |\___/|_| |_|\___|_|  |_/_/\_\  Shell -> mksh (core/builder) / toybox sh (minimal)
|__/                                CPU -> Virtualized Apple Silicon (4)
======= permissive + linux =======  Memory -> 102 MiB / 1.01 GiB (10%)
                                    Disk (/) -> 726 MiB / 504 GiB - ext4
```

## License

All original jonerix code is released under the **MIT License**.

The Linux kernel (GPLv2) is the sole GPL exception. It ships as a single blob under `/boot` and is the only non-permissive component on a running jonerix system.
