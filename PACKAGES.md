# jonerix package inventory

Generated from tracked `packages/**/recipe.toml` — **90 recipes**. All jonerix-built packages are permissively licensed (MIT / BSD / Apache-2.0 / ISC / 0BSD / Zlib / PSF-2.0 / MirOS). The sole exception, `linux` (GPL-2.0), is explicitly blocked by jpkg's license gate and built out-of-band via `scripts/build-kernel.sh`.

## Folders

- **`packages/core/`** — runtime-critical userland that every jonerix system needs.
- **`packages/develop/`** — toolchain + compilers (clang/LLVM, rust, go, python3, perl, nodejs, jmake, cmake).
- **`packages/extra/`** — everything else: networking daemons, container runtime, shells, editors, vendored tooling.
- **`packages/jpkg/`** + **`packages/jpkg-local/`** — the jonerix package manager itself, built out-of-tree by the CI scripts; the parallel recipe under `core/jpkg/` is what ships the runtime binary.

## Build targets that consume these packages

| Target | File | Role |
|---|---|---|
| **`pi5-install`** | `install/pi5-install.sh` `DEFAULT_PACKAGES` | User-triggered USB install script for a running Pi 5 |
| **`pi5-image`**   | `image/pi5/build-image.py` + `.github/workflows/publish-pi5-image.yml` | Pre-baked Pi 5 SD/USB image shipped as a release asset |
| **`docker:minimal`** | `Dockerfile.minimal`   | Minimal ~30 MB jonerix image — musl, toybox, dropbear, openrc, curl, libressl, zlib, zstd |
| **`docker:core`** | `Dockerfile.core`      | Slim runtime base (Stage 1); parent of builder + router |
| **`docker:full`** | `Dockerfile`           | The traditional all-in-one jonerix container (includes compilers, tools, editors) |
| **`docker:builder`** | `Dockerfile.builder` | Core + toolchain used by every publish-packages CI run |
| **`docker:router`** | `Dockerfile.router`   | Core + nloxide/hostapd/wpa_supplicant for BR-routers |

### Packages shared across build targets

A package's "Used in" column lists every build that installs it. Spot-check of high-sharing packages:
- **`musl`** → `pi5-install`, `pi5-image`, `docker:minimal`, `docker:core`, `docker:full`
- **`toybox`** → `pi5-install`, `pi5-image`, `docker:minimal`, `docker:core`, `docker:full`
- **`openrc`** → `pi5-install`, `pi5-image`, `docker:minimal`, `docker:core`, `docker:full`
- **`dropbear`** → `pi5-install`, `pi5-image`, `docker:minimal`, `docker:core`, `docker:full`
- **`dhcpcd`** → `pi5-install`, `pi5-image`, `docker:core`, `docker:full`
- **`bsdtar`** → `pi5-install`, `pi5-image`, `docker:core`, `docker:full`
- **`libressl`** → `docker:minimal`, `docker:core`, `docker:full`
- **`curl`** → `docker:minimal`, `docker:core`, `docker:full`
- **`zstd`** → `docker:minimal`, `docker:core`, `docker:full`

## Full inventory

| Package | Folder | Version | License | Arch | Runtime deps | Build deps | Used in | Description |
|---|---|---|---|---|---|---|---|---|
| **`jpkg`** | `(top)` | 1.0.10 | MIT | any | `musl` | `clang`, `llvm`, `samurai`, `libressl`, `zstd` | — | jonerix package manager |
| **`jpkg-local`** | `(top)` | 1.0.10 | MIT | any | `jpkg`, `zstd` | `clang`, `llvm`, `zstd` | — | jpkg subcommand for local .jpkg install and recipe builds |
| **`anvil`** | `core` | 0.2.1-r1 | MIT | any | `musl` | `rust` | `pi5-install`, `pi5-image` | Clean-room MIT Rust ext2/3/4 userland (mkfs.ext4, e2fsck, tune2fs, debugfs, ...) |
| **`bsdtar`** | `core` | 3.8.6-r6 | Apache-2.0 | any | `libarchive` | — | `pi5-install`, `pi5-image`, `docker:core`, `docker:full` | Compatibility package providing /bin/tar via libarchive bsdtar |
| **`curl`** | `core` | 8.11.1-r1 | MIT | any | `musl`, `libressl`, `zlib` | `clang`, `cmake`, `samurai`, `libressl`, `zlib` | `docker:minimal`, `docker:core`, `docker:full` | Command-line tool and library for transferring data with URLs |
| **`dhcpcd`** | `core` | 10.1.0-r4 | BSD-2-Clause | any | `musl`, `openrc` | `clang`, `make`, `jonerix-headers` | `pi5-install`, `pi5-image`, `docker:core`, `docker:full` | RFC2131-compliant DHCP client daemon |
| **`doas`** | `core` | 6.8.2 | ISC | any | `musl` | `clang`, `make` | `docker:core`, `docker:full` | Lightweight privilege escalation tool (OpenDoas) |
| **`dropbear`** | `core` | 2024.86-r2 | MIT | any | `musl`, `libressl` | `clang`, `make`, `jonerix-headers` | `pi5-install`, `pi5-image`, `docker:minimal`, `docker:core`, `docker:full` | Lightweight SSH server and client |
| **`exproxide`** | `core` | 0.1.0 | MIT | any | `musl` | `rust` | `docker:builder` | Clean-room Rust implementation of expr for jonerix |
| **`fastfetch`** | `core` | 2.36.1 | MIT | any | `musl`, `libcxx` | `clang`, `cmake`, `samurai` | `docker:core`, `docker:full` | Feature-rich system information tool |
| **`gitoxide`** | `core` | 0.52.0-r5 | MIT OR Apache-2.0 | any | `musl` | `rust` | `docker:core` | Pure Rust implementation of git (gix + ein) |
| **`gnu-compat-symlinks`** | `core` | 1.0.0 | MIT | any | `llvm`, `libcxx` | — | — | Compatibility symlinks for GNU-built binaries (libgcc_s → libunwind, libstdc++ → libc++) |
| **`ifupdown-ng`** | `core` | 0.12.1 | ISC | any | `musl` | `clang`, `make`, `jonerix-headers` | `pi5-image`, `docker:core`, `docker:full` | Next-generation network interface configuration tool |
| **`iproute-go`** | `core` | 0.16.0 | BSD-3-Clause | any | — | `go` | `docker:core` | u-root's ip command — Go reimplementation of iproute2's ip(8) |
| **`jpkg`** | `core` | 1.0.10 | MIT | any | `musl`, `libressl`, `zstd` | `clang`, `llvm`, `samurai`, `libressl`, `zstd` | — | jonerix package manager |
| **`jq`** | `core` | 1.8.1 | MIT | any | `musl` | `clang`, `make`, `jonerix-headers` | — | Lightweight and flexible command-line JSON processor |
| **`libarchive`** | `core` | 3.8.6-r5 | Apache-2.0 | any | `musl`, `zlib`, `xz`, `zstd`, `lz4`, `libressl` | `clang`, `cmake`, `samurai`, `libressl`, `zlib`, `xz`, `zstd`, `lz4` | `docker:core` | Multi-format archive and compression library with bsdtar |
| **`libffi`** | `core` | 3.4.6 | MIT | any | `musl` | `clang`, `make`, `jonerix-headers` | — | Foreign Function Interface library — dispatches to C ABI from dynamic callers |
| **`libressl`** | `core` | 4.0.0 | ISC | any | `musl` | `clang`, `cmake`, `samurai` | `docker:minimal`, `docker:core`, `docker:full` | Free TLS/crypto stack from OpenBSD (provides libssl, libcrypto, libtls) |
| **`lz4`** | `core` | 1.10.0 | BSD-2-Clause | any | `musl` | `clang`, `cmake`, `samurai` | `docker:core`, `docker:full` | Extremely fast lossless compression library and tool |
| **`mandoc`** | `core` | 1.14.6 | ISC | any | `musl` | `clang`, `make` | `docker:core`, `docker:full` | UNIX manpage compiler and viewer |
| **`micro`** | `core` | 2.0.15 | MIT | any | — | `go` | `docker:core`, `docker:full` | Modern terminal text editor (intuitive keybindings, syntax highlighting) |
| **`mksh`** | `core` | R59c-r2 | MirOS | any | `musl` | `clang` | `pi5-install`, `pi5-image` | MirBSD Korn Shell — POSIX shell for /bin/sh |
| **`musl`** | `core` | 1.2.6 | MIT | any | — | `clang`, `make` | `pi5-install`, `pi5-image`, `docker:minimal`, `docker:core`, `docker:full` | Lightweight, fast, and standards-conformant C library |
| **`ncurses`** | `core` | 6.5-r1 | MIT | any | — | `clang`, `make` | `docker:core`, `docker:full` | Terminal handling library |
| **`onetrueawk`** | `core` | 20240728 | MIT | any | `musl` | `clang`, `byacc` | `docker:core` | Brian Kernighan's one true awk |
| **`openntpd`** | `core` | 6.8p1-r3 | ISC | any | `musl`, `libressl` | `clang`, `jmake` | `pi5-install`, `pi5-image` | OpenBSD NTP daemon — lightweight, secure time synchronization |
| **`openrc`** | `core` | 0.54-r6 | BSD-2-Clause | any | `musl`, `mksh` | `clang`, `meson`, `samurai`, `jonerix-headers` | `pi5-install`, `pi5-image`, `docker:minimal`, `docker:core`, `docker:full` | Dependency-based init system for Unix-like systems |
| **`pigz`** | `core` | 2.8-r1 | Zlib | any | `musl`, `zlib` | `clang`, `make` | `docker:core`, `docker:full` | Parallel implementation of gzip |
| **`ripgrep`** | `core` | 15.1.0 | MIT | any | `musl` | `rust` | `docker:core` | Fast line-oriented search tool (grep replacement) |
| **`snooze`** | `core` | 0.5-r1 | CC0 | any | `musl` | `clang`, `make` | `docker:core`, `docker:full` | Lightweight cron alternative for running a command at a specific time |
| **`sudo`** | `core` | 1.9.17p2 | ISC | any | `musl`, `libressl` | `clang`, `make`, `jonerix-headers` | `pi5-install`, `pi5-image` | Privilege escalation utility |
| **`toybox`** | `core` | 0.8.11-r2 | 0BSD | any | `musl` | `clang` | `pi5-install`, `pi5-image`, `docker:minimal`, `docker:core`, `docker:full` | BSD-licensed coreutils replacement |
| **`tzdata`** | `core` | 2026a | Public-Domain | any | `musl` | `clang` | `docker:core` | IANA time zone database (zoneinfo data plus zic/zdump tools) |
| **`unbound`** | `core` | 1.22.0 | BSD-3-Clause | any | `musl`, `libressl` | `clang`, `make`, `jonerix-headers` | `docker:core`, `docker:full` | Validating, recursive, and caching DNS resolver |
| **`uutils`** | `core` | 0.7.0-r1 | MIT | any | `musl` | `rust`, `jmake` | — | Rust rewrite of GNU coreutils (tr, expr, sort, wc, cut, and 70+ more) |
| **`xz`** | `core` | 5.8.2-r2 | 0BSD | any | `musl` | `clang`, `cmake`, `samurai` | `docker:core`, `docker:full` | XZ compression utilities and liblzma |
| **`zlib`** | `core` | 1.3.2-r1 | Zlib | any | `musl` | `clang`, `make` | `docker:minimal`, `docker:core`, `docker:full` | General-purpose compression library |
| **`zstd`** | `core` | 1.5.6 | BSD-3-Clause | any | `musl` | `clang`, `cmake`, `samurai` | `docker:minimal`, `docker:core`, `docker:full` | Zstandard compression library and tool |
| **`bc`** | `develop` | 7.0.3 | BSD-2-Clause | any | `musl` | `clang`, `make` | `docker:full`, `docker:builder` | Implementation of the bc calculator language |
| **`byacc`** | `develop` | 20241231 | public domain | any | `musl` | `clang`, `make` | `docker:full`, `docker:builder` | Berkeley Yacc parser generator |
| **`cmake`** | `develop` | 4.1.0 | BSD-3-Clause | any | `musl` | `clang`, `python3`, `samurai` | `docker:full`, `docker:builder` | Cross-platform build system generator |
| **`flex`** | `develop` | 2.6.4 | BSD-2-Clause | any | `musl` | `clang`, `make` | `docker:full`, `docker:builder` | Fast lexical analyzer generator |
| **`go`** | `develop` | 1.26.1 | BSD-3-Clause | any | `musl`, `gnu-compat-symlinks` | `python3` | `docker:builder` | Go programming language compiler and tools |
| **`jmake`** | `develop` | 1.1.1 | MIT | any | `musl` | `rust` | `docker:full`, `docker:builder` | Clean-room drop-in replacement for GNU Make, written in Rust |
| **`jonerix-headers`** | `develop` | 4.19.88-r2 | MIT | any | — | — | — | Linux UAPI kernel headers for jonerix package builds |
| **`libcxx`** | `develop` | 21.1.2 | Apache-2.0 | any | `musl` | `clang`, `cmake`, `samurai`, `python3` | `docker:core`, `docker:builder` | LLVM libc++, libc++abi, and libunwind runtime with corrected libunwind SONAME |
| **`lldb`** | `develop` | 21.1.2 | Apache-2.0 | any | `musl`, `llvm`, `libcxx`, `xz`, `zstd`, `zlib` | `llvm-all` | — | LLVM debugger — carved out of llvm-all (no separate compile) |
| **`llvm`** | `develop` | 21.1.2-r1 | Apache-2.0 | any | `musl`, `libcxx`, `zstd`, `zlib` | `clang`, `cmake`, `samurai`, `python3`, `libcxx` | `docker:full`, `docker:builder` | Slim LLVM toolchain (toolchain-only: clang, lld, llvm-ar/nm/ranlib/strip/objcopy/objdump/readelf, opt, llc)… |
| **`m4oxide`** | `develop` | 0.1.0-r0 | MIT | any | `musl` | `rust` | — | Clean-room Rust implementation of m4 for jonerix |
| **`nodejs`** | `develop` | 24.15.0-r3 | MIT | any | `musl`, `zlib`, `libcxx` | `clang`, `python3`, `samurai`, `zlib`, `libcxx`, `jonerix-headers` | `docker:full`, `docker:builder` | JavaScript runtime built on V8 (libc++ / compiler-rt / small-icu / zero GNU) |
| **`perl`** | `develop` | 5.40.0 | Artistic-2.0 | any | `musl` | `clang`, `jmake` | `docker:full`, `docker:builder` | Practical Extraction and Report Language |
| **`python3`** | `develop` | 3.14.3-r9 | PSF-2.0 | any | `musl`, `zlib`, `zstd`, `ncurses`, `libressl`, `xz`, `libffi`, `sqlite` (+1) | `clang`, `libffi`, `sqlite`, `bzip2` | `pi5-install`, `pi5-image`, `docker:full`, `docker:builder` | Python programming language interpreter (with _bz2) |
| **`rust`** | `develop` | 1.94.1-r2 | MIT | any | `musl`, `libcxx`, `llvm` | — | `docker:builder` | Systems programming language (jonerix-linux-musl triple, no GNU runtime) |
| **`samurai`** | `develop` | 1.2 | Apache-2.0 | any | `musl` | `clang`, `make` | `docker:full`, `docker:builder` | ninja-compatible build tool written in C |
| **`strace`** | `develop` | 4.25 | BSD-3-Clause | any | `musl` | `clang`, `make`, `jonerix-headers` | `docker:full`, `docker:builder` | ptrace-based syscall tracer (last BSD-3-Clause release) |
| **`bsdsed`** | `extra` | 0.99.2 | BSD-2-Clause | any | `musl` | `clang`, `make` | — | FreeBSD sed made portable — POSIX stream editor |
| **`btop`** | `extra` | 1.4.5-r1 | Apache-2.0 | any | `musl`, `libcxx` | `clang`, `cmake`, `samurai` | `docker:router` | Terminal resource monitor with CPU, memory, disk, network, and process views |
| **`bzip2`** | `extra` | 1.0.8-r1 | bzip2-1.0.6 | any | `musl` | `clang`, `make` | — | Block-sorting file compressor (bzip2 + libbz2, clang/musl build, no GNU) |
| **`ca-certificates`** | `extra` | 20260211-r1 | MPL-2.0 | any | — | — | `docker:full` | Mozilla CA certificate bundle for TLS verification |
| **`cni-plugins`** | `extra` | 1.9.1 | Apache-2.0 | any | `musl` | `go` | — | CNI network plugins for container networking |
| **`containerd`** | `extra` | 2.2.2 | Apache-2.0 | any | `musl` | `go` | — | Industry-standard container runtime |
| **`derper`** | `extra` | 1.96.5 | BSD-3-Clause | any | `musl` | `go` | — | Tailscale DERP relay server |
| **`headscale`** | `extra` | 0.28.0 | BSD-3-Clause | any | `musl` | `go` | — | Open-source self-hosted Tailscale control server |
| **`hostapd`** | `extra` | 2.11-r1 | BSD-3-Clause | any | `musl`, `libressl`, `nloxide` | `clang`, `jmake`, `jonerix-headers`, `libressl`, `nloxide` | `docker:router` | IEEE 802.11 AP, IEEE 802.1X/WPA/WPA2/EAP/RADIUS Authenticator |
| **`jfsck`** | `extra` | 0.1.0-r0 | BSD-2-Clause | any | — | `rust` | — | Clean-room fsck for ext4 + FAT32 (Raspberry Pi scope) derived from Ghidra binary analysis of e2fsprogs and … |
| **`jonerix-ext4-rescue`** | `extra` | 0.1.0 | MIT | any | — | `rust` | — | Reset a corrupted ext4 inode's extent header so the file can be rm'd |
| **`jonerix-ntp-http-bootstrap`** | `extra` | 1.0.0 | MIT | any | `mksh`, `openrc`, `curl` | — | `pi5-install` | HTTP Date-header clock bootstrap for RTC-less hosts (ships ntp-set-http + ntp-bootstrap OpenRC service) |
| **`jonerix-raspi5-fixups`** | `extra` | 1.6.1 | MIT | aarch64 | `musl`, `openrc`, `python3` | `rust` | `pi5-install`, `pi5-image` | Hardware fixups for jonerix on Raspberry Pi 5 (EEE, fan, onboard WiFi, OpenRC-backed reboot, cold-reboot, wake-on-power, RTC coin-cell monitor, tty1 HDMI console) + adduser safety + legacy bootstrap cleanup + fstab rescue + errors=remount-ro |
| **`jonerix-util`** | `extra` | 0.1.0-r4 | MIT | any | `musl` | `clang`, `rust` | — | Clean-room permissive-licensed replacement for parts of util-linux (lscpu, hwclock, ionice, nsenter, chsh) |
| **`libevent`** | `extra` | 2.1.12-stable | BSD-3-Clause | any | `musl`, `libressl` | `clang`, `make`, `jonerix-headers`, `libressl` | — | Event notification library (prerequisite for tmux) |
| **`limine`** | `extra` | 11.2.1 | BSD-2-Clause | any | `musl` | — | — | Modern, portable bootloader supporting UEFI and legacy BIOS (BSD-2-Clause) |
| **`linux`** | `extra` | 6.14.2 | GPL-2.0-only | any | — | — | — | Linux kernel — the sole GPL exception in jonerix. Provides vmlinuz, kernel modules, and kernel headers. |
| **`llvm-all`** | `extra` | 21.1.2 | Apache-2.0 | any | `musl`, `libcxx`, `zstd`, `zlib` | `clang`, `cmake`, `samurai`, `python3`, `libcxx` | — | Full LLVM toolchain with all 80+ clang/llvm tools — pairs with slim llvm |
| **`lsusb-rs`** | `extra` | 0.1.0-r0 | MIT | any | `musl` | `rust` | — | Permissive-license lsusb drop-in (pure Rust, sysfs backend) |
| **`lua`** | `extra` | 5.4.7 | MIT | any | `musl` | `clang`, `make` | — | Lua programming language interpreter, compiler, and library |
| **`nerdctl`** | `extra` | 2.2.1 | Apache-2.0 | any | `musl`, `containerd`, `runc`, `cni-plugins` | `go` | — | Docker-compatible CLI for containerd |
| **`nginx`** | `extra` | 1.28.3-r1 | BSD-2-Clause | any | `musl`, `libressl`, `zlib`, `pcre2` | `clang`, `make`, `mksh`, `libressl`, `zlib`, `pcre2` | — | High-performance HTTP server and reverse proxy |
| **`nloxide`** | `extra` | 0.1.0-r3 | BSD-2-Clause | any | `musl` | `clang`, `rust`, `jonerix-headers` | `docker:router` | Clean-room netlink library (libnl-3 / libnl-genl-3 drop-in) written in Rust |
| **`openrsync`** | `extra` | 0.5.0-git20250127 | ISC | any | `musl` | `clang`, `llvm` | — | BSD-licensed clean-room implementation of rsync (drop-in replacement, protocol 27 compatible) |
| **`pcre2`** | `extra` | 10.47 | BSD-3-Clause | any | `musl` | `clang`, `cmake`, `samurai` | — | Perl Compatible Regular Expressions library (v2) |
| **`pico`** | `extra` | 2.26 | Apache-2.0 | any | `ncurses`, `musl` | `clang`, `make`, `python3`, `ncurses`, `jonerix-headers` | — | Stand-alone pico text editor (from alpine-2.26) |
| **`raspi-config`** | `extra` | 20260326 | MIT | any | `mksh`, `toybox` | — | `pi5-install`, `pi5-image` | Raspberry Pi configuration tool (MIT, upstream RPi-Distro/raspi-config pinned to 08a52319) |
| **`ruby`** | `extra` | 3.4.3-r1 | BSD-2-Clause AND Ruby | any | `musl`, `libressl`, `zlib` | `clang`, `jmake`, `onetrueawk`, `mksh` | — | Ruby programming language interpreter |
| **`runc`** | `extra` | 1.4.1 | Apache-2.0 | any | `musl` | `go` | — | OCI container runtime |
| **`sqlite`** | `extra` | 3.51.3-r1 | Public-Domain | any | `musl` | `clang`, `make` | — | Self-contained SQL database engine (with sqlite3.pc) |
| **`tmux`** | `extra` | 3.6a | ISC | any | `musl`, `ncurses`, `libevent` | `clang`, `make`, `libevent`, `ncurses`, `jonerix-headers`, `byacc` | — | Terminal multiplexer |
| **`unzip`** | `extra` | 0.1.0 | Apache-2.0 | any | `libarchive` | — | — | Compatibility package: /bin/unzip → /bin/bsdunzip (libarchive) |
| **`wpa_supplicant`** | `extra` | 2.11-r2 | BSD-3-Clause | any | `musl`, `libressl`, `nloxide` | `clang`, `jmake`, `jonerix-headers`, `libressl`, `nloxide` | `docker:router` | WPA/WPA2/WPA3 supplicant for wireless network authentication |
| **`zsh`** | `extra` | 5.9-r1 | MIT | any | `musl`, `ncurses` | `clang`, `make` | `docker:full` | Z shell — feature-rich interactive shell |

## Special-purpose / not-built-by-CI

- **`linux`** (`packages/extra/linux`) — Linux kernel 6.14.2, GPL-2.0-only. Deliberately blocked by jpkg's license gate (`cmd_build.c`). Built out-of-band via `scripts/build-kernel.sh` as the sole GPL exception in the project.
- **`jpkg`** (`packages/jpkg`) — The package manager itself. CI builds this from source in each runner (`scripts/ci-build-*.sh`) rather than pulling a prebuilt one; the parallel `packages/core/jpkg/recipe.toml` is what publishes the runtime binary into INDEX.
- **`jpkg-local`** (`packages/jpkg-local`) — Ad-hoc `.jpkg`-file installer and recipe builder. Shares jpkg's util/toml/pkg/db modules so it tracks jpkg's version in lockstep.
- **`jonerix-raspi5-fixups`** — `arch = "aarch64"`; skipped by ci-build on x86_64 runners since its inline asm is Pi 5-only.
