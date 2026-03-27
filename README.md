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

**A Linux distribution with a strictly permissive userland.**

## Overview

jonerix is a Linux distribution where every userland component runs under a permissive license: MIT, BSD, ISC, Apache-2.0, or public domain. The Linux kernel is the sole GPL exception -- there is no viable permissive alternative with equivalent hardware and container support.

The system ships with a full development toolchain including Python, Node.js, and Clang/LLVM, and is designed to be fully self-hosting: jonerix can build its own kernel and rebuild the entire distribution from source using only its own permissive tools.

## Quick Start

```sh
brew install container
container system start
container build --tag jonerix:latest .
container run --interactive --name jonerix jonerix:latest
```

## What's Inside

| Component | Version | License | Role |
|-----------|---------|---------|------|
| toybox | 0.8.11 | 0BSD | Coreutils |
| mksh | R59c | MirOS/ISC | Shell |
| jpkg | 1.0.0 | MIT | Package manager |
| Python | 3.12 | PSF | Scripting |
| Node.js | v24 | MIT | JavaScript runtime |
| Clang/LLVM | 21 | Apache-2.0 | C/C++ compiler |
| LLD | 21 | Apache-2.0 | Linker |
| Dropbear | latest | MIT | SSH |
| pico | latest | Apache-2.0 | Editor |
| curl | latest | MIT | HTTP client |
| bmake | latest | MIT | BSD make |
| flex | latest | BSD | Lexer generator |
| bc | latest | BSD | Calculator |
| perl | latest | Artistic-2.0 | Scripting |
| fastfetch | latest | MIT | System info |

## Package Manager (jpkg)

jpkg is a custom, MIT-licensed package manager purpose-built for jonerix. Packages are zstd-compressed tarballs signed with Ed25519.

```sh
jpkg update
jpkg search fastfetch
jpkg install fastfetch
fastfetch
```

## Building from Source

jonerix is bootstrapped from Alpine Linux through a multi-stage build process. Alpine is used only as a build host -- nothing from it enters the final image.

```sh
sh bootstrap/stage0.sh    # Install Alpine build dependencies
sh bootstrap/stage1.sh    # Cross-compile all components with Clang/musl
sh bootstrap/stage2.sh    # Assemble clean rootfs (no GPL artifacts)
```

An optional verification stage confirms the system is fully self-hosting:

```sh
sh bootstrap/stage3-verify.sh
```

See `bootstrap/config.sh` for version pins, SHA256 hashes, and compiler flags.

## fastfetch

```
   _                       _
  (_) ___  _ __   ___ _ __(_)_  __       root@jonerix
  | |/ _ \| '_ \ / _ \ '__| \ \/ /      -----------------
  | | (_) | | | |  __/ |  | |>  <       OS -> jonerix 1.0 aarch64
 _/ |\___/|_| |_|\___|_|  |_/_/\_\      Kernel -> Linux 6.18.5
|__/                                     Shell -> mksh
  ========= permissive + linux =========  CPU -> Virtualized Apple Silicon (4)
                                         Memory -> 102 MiB / 1.01 GiB (10%)
                                         Disk (/) -> 726 MiB / 504 GiB - ext4
```

## License

All original jonerix code is released under the **MIT License**.

The Linux kernel (GPLv2) is the sole GPL exception. It ships as a single blob under `/boot` and is the only non-permissive component on a running jonerix system.
