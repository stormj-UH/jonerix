# Bootstrapping jonerix

This document explains how to build jonerix from source. The build system uses jpkg (the custom package manager) and per-package `recipe.toml` build recipes to produce a complete, self-contained Linux distribution where every userland component is permissively licensed.

## Prerequisites

### Hardware Requirements

- x86_64 or aarch64 system (or VM)
- At least 4 GB RAM (8 GB recommended for LLVM builds)
- At least 20 GB free disk space
- Internet connection (for downloading source tarballs)

### Software Requirements

- **Docker** (recommended):
  ```sh
  docker pull alpine:latest
  ```

## Overview

```
Alpine build host              jpkg packages              Final rootfs
┌──────────────────┐         ┌───────────────────┐      ┌───────────────────┐
│ Alpine + clang   │──build──▶│ .jpkg archives    │──install──▶│ Pure permissive  │
│ jpkg, build deps │         │ both arches, signed│      │ system. No GPL.  │
└──────────────────┘         └───────────────────┘      └───────────────────┘
       GPL is OK                 from-source builds           no GPL at all
```

**Key principle**: GPL tools (Alpine's apk, BusyBox) are used only as build-time scaffolding. They never appear in the final image.

## Building Images

### Minimal Image (shell, init, network, SSH)

```sh
docker build -f Dockerfile.minimal --tag jonerix-minimal:latest .
docker run -it jonerix-minimal:latest
```

### Development Image (compilers, languages, build tools)

```sh
docker build -f Dockerfile.develop --tag jonerix-develop:latest .
docker run -it jonerix-develop:latest
```

### Full Image

```sh
docker build --tag jonerix:latest .
docker run -it jonerix:latest
```

## Building Packages from Source

Packages are built from the source recipes in `packages/{core,develop,extra}/*/recipe.toml`. The build script processes them in dependency order:

```sh
# Inside a jonerix-develop container
docker run --rm -v "$PWD:/workspace" -w /workspace jonerix-develop:latest \
  sh scripts/build-all.sh --output /workspace/.build/pkgs
```

### Build Order

Dependencies are built first. Key tiers:

| Tier | Packages | Notes |
|------|----------|-------|
| 1 | musl | C library — everything links against it |
| 2 | zstd, lz4, xz, zlib | Compression libraries |
| 3 | OpenSSL, ncurses | TLS + terminal |
| 4 | toybox, bsdtar | Core userland |
| 5 | samurai, cmake, byacc, flex, bc | Build tools |
| 6 | LLVM/Clang/LLD | Compiler toolchain (~45 min) |
| 7 | Go (bootstrap chain: C → 1.4 → 1.17 → ... → 1.26) | Go language |
| 8 | containerd, runc, nerdctl, CNI plugins | Container runtime |
| 9 | Rust, uutils, gitoxide, ripgrep | Rust ecosystem |
| 10 | Python 3, Node.js, Perl | Scripting languages |

### Compiler Flags

All C/C++ packages are compiled with hardening flags:

```sh
CC=clang
LD=ld.lld
CFLAGS="-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2"
LDFLAGS="-Wl,-z,relro,-z,now -pie"
```

### Recipe Format

Each package has a `recipe.toml` with source URL, SHA256, build commands, and dependencies:

```toml
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
build = "CC=clang make -j$(nproc)"
install = "make PREFIX=$DESTDIR install"

[depends]
runtime = ["musl"]
build = ["clang"]
```

## How Images Are Assembled

1. **jpkg** is built from C source in an Alpine container (the only GPL build-time dependency)
2. **Packages** are downloaded from GitHub Releases via `jpkg install`
3. **Rootfs** is assembled FROM scratch with merged `/usr` layout (`/usr → /`)
4. **Final image** contains zero GPL runtime components

## Self-Hosting

jonerix can rebuild itself from source using only its own tools. The jonerix-develop image contains every compiler and build tool needed — no Alpine overlay required.

## Troubleshooting

### LLVM Build Fails with OOM

LLVM requires significant memory. Solutions:

```sh
# Add swap
dd if=/dev/zero of=/swapfile bs=1M count=4096
mkswap /swapfile && swapon /swapfile

# Or reduce parallelism
export MAKEFLAGS="-j2"
```

### License Audit

```sh
# Inside a jonerix container
license-audit
```

If violations are found:
1. Check if the package is build-time only (OK if it doesn't ship)
2. Check if there's a permissive alternative
3. Document any essential exceptions in DESIGN.md
