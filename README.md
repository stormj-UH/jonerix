# jonerix

```
   _                       _
  (_) ___  _ __   ___ _ __(_)_  __
  | |/ _ \| '_ \ / _ \ '__| \ \/ /
  | | (_) | | | |  __/ |  | |>  <
 _/ |\___/|_| |_|\___|_|  |_/_/\_\
|__/
  ========= permissive + linux =========
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

The jpkg-only develop image (`Dockerfile.develop`) installs every tool from jpkg packages with no Alpine overlay. It compiles C, Go, and Rust programs out of the box.

## Quick Start

```sh
# Minimal runtime (shell, init, network, SSH)
docker build -f Dockerfile.minimal --tag jonerix-minimal:latest .
docker run -it jonerix-minimal:latest

# Development environment (compilers, languages, build tools)
docker build -f Dockerfile.develop --tag jonerix-develop:latest .
docker run -it jonerix-develop:latest

# Full image (runtime + all dev tools)
docker build --tag jonerix:latest .
docker run -it jonerix:latest
```

### Building Packages from Source

```sh
# Run inside jonerix-develop container
docker run --rm -v "$PWD:/workspace" -w /workspace jonerix-develop:latest \
  sh bootstrap/build-all.sh --output /workspace/.build/pkgs
```

## What's Inside

### Core System

| Component | License | Role |
|-----------|---------|------|
| musl | MIT | C standard library |
| toybox | 0BSD | Coreutils (ls, cp, cat, grep, ...) |
| uutils | MIT | Extended coreutils (sort, wc, tr, ...) |
| mksh | MirOS | Shell |
| jpkg | MIT | Package manager |
| OpenRC | BSD-2-Clause | Init system |
| dropbear | MIT | SSH server/client |

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
| OpenSSL | Apache-2.0 | TLS library |
| unbound | BSD-3-Clause | DNS resolver |
| dhcpcd | BSD-2-Clause | DHCP client |
| ifupdown-ng | ISC | Network configuration |

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

The system bootstraps through a layered approach:

1. **jpkg** is built from C source in an Alpine container (the only GPL build-time dependency)
2. **All packages** are installed from the jpkg repository into a clean rootfs
3. **Final image** is assembled FROM scratch with zero GPL runtime components

For from-source builds, `bootstrap/build-all.sh` builds packages in dependency order inside a jonerix-develop container, using `packages/bootstrap/*/recipe.toml` recipes.

### Licensing Rule

Every package must carry a permissive license:

| Allowed | Not Allowed |
|---------|-------------|
| MIT, BSD-2-Clause, BSD-3-Clause | GPL, LGPL, AGPL |
| Apache-2.0, ISC, 0BSD | SSPL, EUPL |
| Zlib, PSF-2.0, Artistic-2.0 | CC-BY-SA |
| Public Domain, MirOS | Any copyleft |

bash (GPL-3.0) is used only as a build-time tool inside Alpine and never ships in the final image.

## fastfetch

```
   _                       _
  (_) ___  _ __   ___ _ __(_)_  __       root@jonerix
  | |/ _ \| '_ \ / _ \ '__| \ \/ /      -----------------
  | | (_) | | | |  __/ |  | |>  <       OS -> jonerix 1.0 aarch64
 _/ |\___/|_| |_|\___|_|  |_/_/\_\      Kernel -> Linux 6.18.5
|__/                                     Shell -> zsh (develop) / toybox sh (minimal)
  ========= permissive + linux =========  CPU -> Virtualized Apple Silicon (4)
                                         Memory -> 102 MiB / 1.01 GiB (10%)
                                         Disk (/) -> 726 MiB / 504 GiB - ext4
```

## License

All original jonerix code is released under the **MIT License**.

The Linux kernel (GPLv2) is the sole GPL exception. It ships as a single blob under `/boot` and is the only non-permissive component on a running jonerix system.
