# jonerix package inventory

Generated from tracked `packages/**/recipe.toml` -- **98 recipes**. All jonerix-built userland packages are permissively licensed (MIT / BSD / Apache-2.0 / ISC / 0BSD / Zlib / PSF-2.0 / MirOS). The sole exception is `linux` (GPL-2.0-only), which is explicitly blocked by jpkg's license gate and built out-of-band via `scripts/build-kernel.sh`.

## Folders

- **`packages/core/`** -- runtime-critical userland that every jonerix system needs.
- **`packages/develop/`** -- toolchain and compilers: clang/LLVM, Rust, Go, Python, Perl, Node.js, jmake, cmake.
- **`packages/extra/`** -- everything else: networking daemons, container runtime, shells, editors, router tooling, and optional packages. `jcarp` lives here only.
- **`packages/core/jpkg/`** -- the Rust jonerix package manager, including `jpkg`, `jpkg-local`, and `jpkg-conform`. CI and Docker bootstrap builds now use the vendored Cargo dependency tree with `cargo --frozen`.

## Build targets

| Target | File | Role |
|---|---|---|
| **`pi5-install`** | `install/pi5-install.sh` | User-triggered USB install script for a running Pi 5 |
| **`pi5-image`** | `image/pi5/build-image.py` + `.github/workflows/publish-pi5-image.yml` | Pre-baked Pi 5 SD/USB image shipped as a release asset |
| **`docker:minimal`** | `Dockerfile.minimal` | Minimal jonerix image: musl, toybox, dropbear, openrc, curl, libressl, zlib, zstd |
| **`docker:core`** | `Dockerfile.core` | Slim runtime base; parent of builder and router images |
| **`docker:full`** | `Dockerfile` | Traditional all-in-one jonerix container with runtime, compilers, tools, and editors |
| **`docker:builder`** | `Dockerfile.builder` | Core plus toolchain used by package-build CI |
| **`docker:router`** | `Dockerfile.router` | Core plus extra/router packages: `jcarp`, `nloxide`, `hostapd`, `wpa_supplicant`, and `stormwall` |

## Full inventory

| Package | Folder | Version | License | Arch | Runtime deps | Build deps | Description |
|---|---|---|---|---|---|---|---|
| **`anvil`** | `core` | 0.2.1-r1 | MIT | any | `musl` | `rust` | Clean-room MIT Rust ext2/3/4 userland (mkfs.ext4, e2fsck, tune2fs, debugfs, ...) |
| **`brash`** | `core` | 1.0.10 | MIT | any | `musl` | `rust` | Clean-room Rust reimplementation of bash 5.3 (no GNU runtime) |
| **`bsdtar`** | `core` | 3.8.6-r7 | Apache-2.0 | any | `libarchive` | - | Compatibility package providing /bin/tar via libarchive bsdtar |
| **`curl`** | `core` | 8.11.1-r6 | MIT | any | `musl`, `libressl`, `zlib`, `libnghttp2` | `clang`, `cmake`, `samurai`, `libressl`, `zlib`, `libnghttp2` | Command-line tool and library for transferring data with URLs |
| **`dhcpcd`** | `core` | 10.1.0-r8 | BSD-2-Clause | any | `musl`, `openrc` | `clang`, `make`, `jonerix-headers` | RFC2131-compliant DHCP client daemon |
| **`dropbear`** | `core` | 2024.86-r4 | MIT | any | `musl`, `libressl` | `clang`, `make`, `jonerix-headers` | Lightweight SSH server and client |
| **`expat`** | `core` | 2.6.4 | MIT | any | `musl` | `clang`, `make` | Stream-oriented XML parser library (libexpat) |
| **`exproxide`** | `core` | 0.1.0 | MIT | any | `musl` | `rust` | Clean-room Rust implementation of expr for jonerix |
| **`fastfetch`** | `core` | 2.36.1-r1 | MIT | any | `musl`, `libcxx` | `clang`, `cmake`, `samurai` | Feature-rich system information tool |
| **`gitoxide`** | `core` | 0.52.0-r5 | MIT OR Apache-2.0 | any | `musl` | `rust` | Pure Rust implementation of git plumbing (gix + ein) |
| **`gitredoxide`** | `core` | 1.0.0 | MIT OR Apache-2.0 | any | `musl` | `rust` | Drop-in /bin/git replacement (43 commands, built on gitoxide gix-* crates) |
| **`gnu-compat-symlinks`** | `core` | 1.0.0 | MIT | any | `llvm`, `libcxx` | - | Compatibility symlinks for GNU-built binaries (libgcc_s → libunwind, libstdc++ → libc++) |
| **`ifupdown-ng`** | `core` | 0.12.1-r2 | ISC | any | `musl` | `clang`, `make`, `jonerix-headers` | Next-generation network interface configuration tool |
| **`iproute-go`** | `core` | 0.16.0 | BSD-3-Clause | any | - | `go` | u-root's ip command — Go reimplementation of iproute2's ip(8) |
| **`jpkg`** | `core` | 2.0.2-r2 | 0BSD | any | `musl`, `mksh` | `rust` | jonerix package manager (Rust 2.0 — supersedes the C jpkg 1.1.5) |
| **`jq`** | `core` | 1.8.1 | MIT | any | `musl` | `clang`, `make`, `exproxide`, `jonerix-headers` | Lightweight and flexible command-line JSON processor |
| **`libarchive`** | `core` | 3.8.6-r5 | Apache-2.0 | any | `musl`, `zlib`, `xz`, `zstd`, `lz4`, `libressl` | `clang`, `cmake`, `samurai`, `libressl`, `zlib`, `xz`, `zstd`, `lz4` | Multi-format archive and compression library with bsdtar |
| **`libffi`** | `core` | 3.4.6 | MIT | any | `musl` | `clang`, `make`, `exproxide`, `jonerix-headers` | Foreign Function Interface library — dispatches to C ABI from dynamic callers |
| **`libressl`** | `core` | 4.0.0 | ISC | any | `musl` | `clang`, `cmake`, `samurai` | Free TLS/crypto stack from OpenBSD (provides libssl, libcrypto, libtls) |
| **`lz4`** | `core` | 1.10.0 | BSD-2-Clause | any | `musl` | `clang`, `cmake`, `samurai` | Extremely fast lossless compression library and tool |
| **`mandoc`** | `core` | 1.14.6 | ISC | any | `musl` | `clang`, `make` | UNIX manpage compiler and viewer |
| **`mksh`** | `core` | R59c-r3 | MirOS | any | `musl`, `toybox` | `clang` | MirBSD Korn Shell — POSIX shell for /bin/sh |
| **`musl`** | `core` | 1.2.6 | MIT | any | - | `clang`, `make` | Lightweight, fast, and standards-conformant C library |
| **`ncurses`** | `core` | 6.5-r3 | MIT | any | - | `clang`, `make` | Terminal handling library |
| **`onetrueawk`** | `core` | 20240728-r2 | MIT | any | `musl` | `clang`, `byacc` | Brian Kernighan's one true awk |
| **`openntpd`** | `core` | 6.8p1-r4 | ISC | any | `musl`, `libressl` | `clang`, `jmake`, `exproxide` | OpenBSD NTP daemon — lightweight, secure time synchronization |
| **`openrc`** | `core` | 0.54-r6 | BSD-2-Clause | any | `musl`, `mksh` | `clang`, `meson`, `samurai`, `jonerix-headers` | Dependency-based init system for Unix-like systems |
| **`pigz`** | `core` | 2.8-r1 | Zlib | any | `musl`, `zlib` | `clang`, `make` | Parallel implementation of gzip |
| **`readlineoxide`** | `core` | 0.1.9-r0 | MIT | any | `musl` | `rust` | Clean-room Rust libreadline/libhistory compatibility layer for jonerix |
| **`ripgrep`** | `core` | 15.1.0 | MIT | any | `musl` | `rust` | Fast line-oriented search tool (grep replacement) |
| **`shadow`** | `core` | 4.19.4-r5 | BSD-3-Clause | any | `musl`, `toybox` | `clang`, `make`, `jonerix-headers`, `pkgconf` | shadow-utils (login, passwd, user/group management) — REPLACES toybox's passwd / login / useradd / userdel / usermod / groupadd / groupdel / groupmod and jonerix-util's chsh. Originals snapshotted under /etc/jpkg/shadow/toybox-prev/ and restored on remove. |
| **`snooze`** | `core` | 0.5-r1 | CC0 | any | `musl` | `clang`, `make` | Lightweight cron alternative for running a command at a specific time |
| **`stormwall`** | `core` | 1.0.4 | MIT | any | `musl` | `rust` | Drop-in firewall front-end — accepts both Linux nftables (nft) and OpenBSD pf (pfctl) DSLs against the same in-kernel netfilter backend |
| **`sudo`** | `core` | 1.9.17p2-r2 | ISC | any | `musl`, `libressl` | `clang`, `make`, `exproxide`, `jonerix-headers` | Privilege escalation utility |
| **`toybox`** | `core` | 0.8.11-r4 | 0BSD | any | `musl` | `clang`, `brash`, `jonerix-headers` | BSD-licensed coreutils replacement |
| **`tzdata`** | `core` | 2026a | Public-Domain | any | `musl` | `clang` | IANA time zone database (zoneinfo data plus zic/zdump tools) |
| **`unbound`** | `core` | 1.22.0-r4 | BSD-3-Clause | any | `musl`, `libressl`, `expat` | `clang`, `make`, `jonerix-headers`, `libressl`, `expat` | Validating, recursive, and caching DNS resolver |
| **`uutils`** | `core` | 0.7.0-r1 | MIT | any | `musl` | `rust`, `jmake` | Rust rewrite of GNU coreutils (tr, expr, sort, wc, cut, and 70+ more) |
| **`xz`** | `core` | 5.8.2-r2 | 0BSD | any | `musl` | `clang`, `cmake`, `samurai` | XZ compression utilities and liblzma (with development headers) |
| **`zlib`** | `core` | 1.3.2-r1 | Zlib | any | `musl` | `clang`, `make` | General-purpose compression library |
| **`zstd`** | `core` | 1.5.6 | BSD-3-Clause | any | `musl` | `clang`, `cmake`, `samurai` | Zstandard compression library and tool |
| **`bc`** | `develop` | 7.0.3 | BSD-2-Clause | any | `musl` | `clang`, `make` | Implementation of the bc calculator language |
| **`byacc`** | `develop` | 20241231-r1 | public domain | any | `musl` | `clang`, `make`, `exproxide` | Berkeley Yacc parser generator |
| **`cmake`** | `develop` | 4.1.0 | BSD-3-Clause | any | `musl` | `clang`, `python3`, `samurai` | Cross-platform build system generator |
| **`flex`** | `develop` | 2.6.4-r4 | BSD-2-Clause | any | `musl` | `clang`, `make`, `m4oxide` | Fast lexical analyzer generator |
| **`go`** | `develop` | 1.26.1 | BSD-3-Clause | any | `musl`, `gnu-compat-symlinks` | `python3` | Go programming language compiler and tools |
| **`jmake`** | `develop` | 1.1.14 | MIT | any | `musl` | `rust` | Clean-room drop-in replacement for GNU Make, written in Rust |
| **`jonerix-headers`** | `develop` | 4.19.88-r3 | 0BSD AND BSD-3-Clause | any | - | - | Linux UAPI kernel headers for jonerix package builds + BSD sys/queue.h compat |
| **`libcxx`** | `develop` | 21.1.2-r1 | Apache-2.0 | any | `musl` | `clang`, `cmake`, `samurai`, `python3` | LLVM libc++, libc++abi, and libunwind runtime with corrected libunwind SONAME |
| **`lldb`** | `develop` | 21.1.2 | Apache-2.0 | any | `musl`, `llvm`, `libcxx`, `xz`, `zstd`, `zlib` | `llvm-all` | LLVM debugger — carved out of llvm-all (no separate compile) |
| **`llvm`** | `develop` | 21.1.2-r2 | Apache-2.0 | any | `musl`, `libcxx`, `zstd`, `zlib` | `clang`, `cmake`, `samurai`, `python3`, `libcxx` | Slim LLVM toolchain (toolchain-only: clang, lld, lldb, llvm-ar/nm/ranlib/strip/objcopy/objdump/readelf, opt, llc). See llvm-all for the full 80+ tool set. |
| **`m4oxide`** | `develop` | 0.1.0-r0 | MIT | any | `musl` | `rust` | Clean-room Rust implementation of m4 for jonerix |
| **`nodejs`** | `develop` | 24.15.0-r3 | MIT | any | `musl`, `zlib`, `libcxx` | `clang`, `python3`, `samurai`, `zlib`, `libcxx`, `jonerix-headers` | JavaScript runtime built on V8 (libc++ / compiler-rt / small-icu / zero GNU) |
| **`perl`** | `develop` | 5.40.0 | Artistic-2.0 | any | `musl` | `clang`, `jmake` | Practical Extraction and Report Language |
| **`python3`** | `develop` | 3.14.3-r10 | PSF-2.0 | any | `musl`, `zlib`, `zstd`, `ncurses`, `libressl`, `xz`, `libffi`, `sqlite`, `bzip2` | `clang`, `libffi`, `sqlite`, `bzip2`, `xz`, `pkgconf` | Python programming language interpreter (with _bz2) |
| **`rust`** | `develop` | 1.94.1-r3 | MIT | any | `musl`, `libcxx`, `llvm` | - | Systems programming language (jonerix-linux-musl triple, no GNU runtime) |
| **`samurai`** | `develop` | 1.2 | Apache-2.0 | any | `musl` | `clang`, `make` | ninja-compatible build tool written in C |
| **`strace`** | `develop` | 4.25-r2 | BSD-3-Clause | any | `musl` | `clang`, `make`, `exproxide`, `jonerix-headers` | ptrace-based syscall tracer (last BSD-3-Clause release) |
| **`bsdsed`** | `extra` | 0.99.2-r1 | BSD-2-Clause | any | `musl` | `clang`, `make` | FreeBSD sed made portable — POSIX stream editor |
| **`btop`** | `extra` | 1.4.5-r3 | Apache-2.0 | any | `musl`, `libcxx` | `clang`, `cmake`, `samurai` | Terminal resource monitor with CPU, memory, disk, network, and process views |
| **`bzip2`** | `extra` | 1.0.8-r1 | bzip2-1.0.6 | any | `musl` | `clang`, `make` | Block-sorting file compressor (bzip2 + libbz2, clang/musl build, no GNU) |
| **`ca-certificates`** | `extra` | 20260211-r2 | MPL-2.0 | any | - | - | Mozilla CA certificate bundle for TLS verification (sourced from curl.se) |
| **`chimerautils`** | `extra` | 15.0.3-r1 | BSD-3-Clause | any | `musl`, `ncurses`, `libressl`, `zlib`, `xz`, `bzip2`, `zstd` | `clang`, `samurai`, `meson`, `pkgconf`, `ncurses`, `libressl`, `zlib`, `xz`, `bzip2`, `zstd` | Chimera Linux's FreeBSD-derived BSD coreutils — ls, cat, cp, mv, sed, grep, awk, find, tar, ed, ee, jot, nc, gzip, m4, patch, ... — installed under /share/chimerautils/ so it coexists with toybox + uutils + bsdsed + onetrueawk without clobbering /bin paths |
| **`cni-plugins`** | `extra` | 1.9.1 | Apache-2.0 | any | `musl` | `go` | CNI network plugins for container networking |
| **`containerd`** | `extra` | 2.2.2-r1 | Apache-2.0 | any | `musl` | `go` | Industry-standard container runtime |
| **`derper`** | `extra` | 1.96.5 | BSD-3-Clause | any | `musl` | `go` | Tailscale DERP relay server |
| **`headscale`** | `extra` | 0.28.0 | BSD-3-Clause | any | `musl` | `go` | Open-source self-hosted Tailscale control server |
| **`hostapd`** | `extra` | 2.11-r1 | BSD-3-Clause | any | `musl`, `libressl`, `nloxide` | `clang`, `jmake`, `jonerix-headers`, `libressl`, `nloxide` | IEEE 802.11 AP, IEEE 802.1X/WPA/WPA2/EAP/RADIUS Authenticator |
| **`jcarp`** | `extra` | 0.1.0-r1 | BSD-2-Clause | any | `musl`, `openrc`, `stormwall` | `rust`, `mksh` | Rust OpenBSD-CARP-compatible failover daemon for jonerix |
| **`jfsck`** | `extra` | 0.1.0-r1 | BSD-2-Clause | any | - | `rust` | Clean-room fsck for ext4 + FAT32 (Raspberry Pi scope) derived from Ghidra binary analysis of e2fsprogs and dosfstools |
| **`jonerix-ext4-rescue`** | `extra` | 0.1.0-r1 | 0BSD | any | - | `rust` | Reset a corrupted ext4 inode's extent header so the file can be rm'd |
| **`jonerix-ntp-http-bootstrap`** | `extra` | 1.0.1-r1 | 0BSD | any | `mksh`, `openrc`, `curl` | - | HTTP Date-header clock bootstrap for RTC-less hosts (ships ntp-set-http + ntp-bootstrap OpenRC service) |
| **`jonerix-raspi5-fixups`** | `extra` | 1.6.15 | 0BSD | aarch64 | `musl`, `openrc`, `python3`, `shadow`, `toybox` | `rust` | Hardware fixups for jonerix on Raspberry Pi 5 (EEE, fan, onboard WiFi, OpenRC-backed reboot, cold-reboot, wake-on-power, RTC coin-cell monitor) + adduser safety + legacy bootstrap cleanup + fstab rescue + errors=remount-ro |
| **`jonerix-util`** | `extra` | 0.1.0-r5 | 0BSD | any | `musl` | `clang`, `rust` | Clean-room permissive-licensed replacement for parts of util-linux (lscpu, hwclock, ionice, nsenter, chsh) |
| **`libevent`** | `extra` | 2.1.12-stable | BSD-3-Clause | any | `musl`, `libressl` | `clang`, `make`, `exproxide`, `jonerix-headers`, `libressl` | Event notification library (prerequisite for tmux) |
| **`libnghttp2`** | `extra` | 1.69.0-r1 | MIT | any | `musl` | `clang`, `make`, `pkgconf` | HTTP/2 C library and tools (nghttp2) |
| **`libnghttp3`** | `extra` | 1.13.0-r1 | MIT | any | `musl` | `clang`, `make`, `pkgconf` | HTTP/3 C library (nghttp3) — implements RFC 9114 framing and QPACK |
| **`libngtcp2`** | `extra` | 1.18.0-r1 | MIT | any | `musl`, `libressl` | `clang`, `make`, `pkgconf`, `libressl` | QUIC C library (ngtcp2) — implements RFC 9000 / 9001 |
| **`limine`** | `extra` | 11.2.1 | BSD-2-Clause | any | `musl` | `clang`, `jmake` | Modern, portable bootloader supporting UEFI and legacy BIOS (BSD-2-Clause) |
| **`linux`** | `extra` | 6.14.2 | GPL-2.0-only | any | - | - | Linux kernel — the sole GPL exception in jonerix. Provides vmlinuz, kernel modules, and kernel headers. |
| **`llvm-all`** | `extra` | 21.1.2-r1 | Apache-2.0 | any | `musl`, `libcxx`, `zstd`, `zlib` | `clang`, `cmake`, `samurai`, `python3`, `libcxx` | Full LLVM toolchain with all 80+ clang/llvm/lldb tools — pairs with slim llvm |
| **`lsusb-rs`** | `extra` | 0.1.0-r0 | MIT | any | `musl` | `rust` | Permissive-license lsusb drop-in (pure Rust, sysfs backend) |
| **`lua`** | `extra` | 5.4.7 | MIT | any | `musl` | `clang`, `make` | Lua programming language interpreter, compiler, and library |
| **`nerdctl`** | `extra` | 2.2.1-r1 | Apache-2.0 | any | `musl`, `containerd`, `runc`, `cni-plugins` | `go` | Docker-compatible CLI for containerd |
| **`nginx`** | `extra` | 1.28.3-r3 | BSD-2-Clause | any | `musl`, `libressl`, `zlib`, `pcre2` | `clang`, `make`, `mksh`, `libressl`, `zlib`, `pcre2` | High-performance HTTP server and reverse proxy |
| **`nloxide`** | `extra` | 1.2.1 | BSD-2-Clause | any | `musl` | `clang`, `rust`, `jonerix-headers` | Clean-room netlink library for jonerix hostapd/wpa_supplicant |
| **`openrsync`** | `extra` | 0.5.0-git20250127 | ISC | any | `musl` | `clang`, `llvm` | BSD-licensed clean-room implementation of rsync (drop-in replacement, protocol 27 compatible) |
| **`pcre2`** | `extra` | 10.47 | BSD-3-Clause | any | `musl` | `clang`, `cmake`, `samurai` | Perl Compatible Regular Expressions library (v2) |
| **`pico`** | `extra` | 2.26 | Apache-2.0 | any | `ncurses`, `musl` | `clang`, `make`, `exproxide`, `python3`, `ncurses`, `jonerix-headers` | Stand-alone pico text editor (from alpine-2.26) |
| **`pkgconf`** | `extra` | 2.5.1-r1 | ISC | any | `musl` | `clang`, `make` | Drop-in replacement for pkg-config (canonical pkg-config implementation since freedesktop.org adopted it) |
| **`raspi-config`** | `extra` | 20260326 | MIT | any | `mksh`, `toybox` | - | Raspberry Pi configuration tool (MIT, upstream RPi-Distro/raspi-config pinned to 08a52319) |
| **`ruby`** | `extra` | 3.4.3-r1 | BSD-2-Clause AND Ruby | any | `musl`, `libressl`, `zlib` | `clang`, `jmake`, `onetrueawk`, `mksh` | Ruby programming language interpreter |
| **`runc`** | `extra` | 1.4.1-r3 | Apache-2.0 | any | `musl` | `go` | OCI container runtime |
| **`sqlite`** | `extra` | 3.51.3-r1 | Public-Domain | any | `musl` | `clang`, `make` | Self-contained SQL database engine (with sqlite3.pc) |
| **`tmux`** | `extra` | 3.6a-r1 | ISC | any | `musl`, `ncurses`, `libevent` | `clang`, `make`, `exproxide`, `libevent`, `ncurses`, `jonerix-headers`, `byacc` | Terminal multiplexer |
| **`unzip`** | `extra` | 0.1.0 | Apache-2.0 | any | `libarchive` | - | Compatibility package: /bin/unzip → /bin/bsdunzip (libarchive) |
| **`wpa_supplicant`** | `extra` | 2.11-r2 | BSD-3-Clause | any | `musl`, `libressl`, `nloxide` | `clang`, `jmake`, `jonerix-headers`, `libressl`, `nloxide` | WPA/WPA2/WPA3 supplicant for wireless network authentication |
| **`zsh`** | `extra` | 5.9-r15 | MIT | any | `musl`, `ncurses` | `clang`, `make` | Z shell — feature-rich interactive shell |
