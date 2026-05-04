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

Current release: **[v1.2.1](https://github.com/stormj-UH/jonerix/releases/tag/v1.2.1)** (`jpkg conform 1.2.1` to pin a host to this tag). The 1.2.x release line ships **jpkg 2.0** — the from-scratch Rust port of the C jpkg 1.1.5 (~11.7K LOC, zero `unsafe`, 160 in-crate tests, byte-equivalent `.jpkg` / INDEX / `/var/db/jpkg/installed/` wire formats). See [PACKAGES.md](PACKAGES.md) for the full package inventory and the README's [Rust drop-in replacements](#rust-drop-in-replacements) section for the port's design.

## Overview

jonerix is a Linux distribution built around a simple rule: every userland component must use a permissive license such as MIT, BSD, ISC, Apache-2.0, or public domain. The Linux kernel is not part of this distribution. This is a "Bring Your Own Kernel" (BYOK) distro. It is designed for use in containers, on WSL, or on Raspberry Pi, but there are no limits.

All packages build from source on jonerix itself. The system compiles its own compiler (Clang/LLVM), its own languages (Go from C, Rust from a bootstrap binary), and its own container runtime. No GNU toolchain, no GCC, no GPL coreutils.

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
docker pull ghcr.io/stormj-uh/jonerix:core       # runtime: mksh (/bin/sh), zsh, uutils, pico, ripgrep, networking
docker pull ghcr.io/stormj-uh/jonerix:builder    # dev: core + clang/llvm, rust, go, nodejs, python3
docker pull ghcr.io/stormj-uh/jonerix:router     # appliance: core + jcarp, hostapd, wpa_supplicant, nloxide, stormwall (nft/pf)

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

# Router appliance image (FROM core)
docker build -f Dockerfile.router --tag jonerix:router .
```

### Raspberry Pi 5

One-liner from any Linux host with `curl` + `sudo`. The bootstrap fetches
the install script from this repo, accepts the GPLv2 + Broadcom Redistributable
licenses for the kernel + firmware blobs (downloaded directly from
`raspberrypi/firmware`, not redistributed by jonerix), and lays down a
permissive-userland Pi 5 with `shadow`-backed login on tty1, the u-root
`ip`, zsh, and the rest.

```sh
# Fresh install onto an attached USB / SD / NVMe device:
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5.sh \
  | sudo sh -s -- -d /dev/sdX

# Pin to a specific jonerix release for a reproducible package set:
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5.sh \
  | sudo sh -s -- -d /dev/sdX --release-tag v1.2.1

# Complete an install on a USB you already dd'd a CI jonerix-pi5.img to
# (the CI image deliberately ships without firmware):
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5.sh \
  | sudo sh -s -- -d /dev/sdX --firmware-only
```

The CI image artifact (`jonerix-pi5.img`, published per release) contains
only the jonerix permissive userland — no Linux kernel and no Broadcom blobs.
Run `--firmware-only` after dd'ing it to fetch the missing pieces directly
from `raspberrypi/firmware` under their own licenses.

See [`install/jonerix-pi5.sh --help`](install/jonerix-pi5.sh) for the full
flag set.

### Windows (WSL2)

CI publishes a ready-to-import WSL rootfs to the rolling [`packages`](https://github.com/stormj-UH/jonerix/releases/tag/packages)
release tag (`jonerix-rootfs-x86_64.tar.gz` and `jonerix-rootfs-aarch64.tar.gz`).
The PowerShell installer downloads the right tarball for your host, runs
`wsl --import`, and registers it as a distribution.

**Prerequisites:** WSL2 enabled. If you've never used WSL on this machine, run
`wsl --install` once from an elevated PowerShell and reboot.

**One-shot install:**

```powershell
# Run from a regular (non-elevated) PowerShell. Auto-detects arch from the host.
iwr -useb https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/wsl/install.ps1 `
  | iex
```

That installs jonerix to `%LOCALAPPDATA%\jonerix\` and registers the
distro as `jonerix`. Launch it any time after with:

```powershell
wsl -d jonerix
```

Or list and pick from your registered distros:

```powershell
wsl -l -v          # list installed distros + their state
wsl -d jonerix     # launch jonerix
wsl --terminate jonerix  # stop the running instance
wsl --unregister jonerix # uninstall (removes the .vhdx)
```

**Custom install location, distro name, or pinned release:**

```powershell
# Save the script first, then call with parameters:
iwr -useb https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/wsl/install.ps1 `
  -OutFile $env:TEMP\jonerix-install.ps1

# Install to D:\WSL\jonerix and register as "jonerix-dev":
& $env:TEMP\jonerix-install.ps1 -InstallDir "D:\WSL\jonerix" -DistroName "jonerix-dev"

# Pin to a specific release tag (defaults to "packages", which rolls):
& $env:TEMP\jonerix-install.ps1 -Release "v1.2.1"

# Install from a local rootfs tarball you've already downloaded:
& $env:TEMP\jonerix-install.ps1 -RootfsUrl "C:\Downloads\jonerix-rootfs-x86_64.tar.gz"
```

See [`install/wsl/install.ps1`](install/wsl/install.ps1) for the full parameter
set, and [`install/wsl/build-rootfs.sh`](install/wsl/build-rootfs.sh) for how
the rootfs is assembled in CI.

## What's Inside

### Image Layers

| Image | Based on | Contents |
|-------|----------|----------|
| `minimal` | scratch | musl, toybox, dropbear, curl, libressl, openrc, jpkg |
| `core` | minimal | mksh (/bin/sh), zsh, uutils, pico, fastfetch, ripgrep, gitoxide, networking tools |
| `builder` | core | clang/llvm, rust, go, nodejs, python3, cmake, jmake, samurai, perl |
| `router` | core | jcarp, hostapd, wpa_supplicant, nloxide (libnl replacement), **stormwall** (single firewall front-end speaking both `nft` and BSD `pf.conf` syntax); home-router / AP / gateway appliance |

### Core System

| Component | License | Role |
|-----------|---------|------|
| musl | MIT | C standard library |
| toybox | 0BSD | Base coreutils (ls, cp, cat, ...) |
| uutils | MIT | Extended coreutils (sort, wc, tr, ...) |
| mksh | MirOS | Shell (/bin/sh) — POSIX-compliant, musl-safe |
| zsh | MIT | Default interactive shell in the larger container images |
| jpkg | 0BSD | Package manager |
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
| jmake | MIT | Drop-in GNU make replacement (Rust). See [Rust drop-in replacements](#rust-drop-in-replacements) |
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
| unbound | BSD-3-Clause | Validating, recursive, caching DNS resolver (takes over /etc/resolv.conf on install) |
| expat | MIT | Stream-oriented XML parser (libexpat) |
| dhcpcd | BSD-2-Clause | DHCP client |
| ifupdown-ng | ISC | Network configuration |
| hostapd | BSD-3-Clause | Wi-Fi access point / WPA supplicant |
| openntpd | BSD-2-Clause | NTP client/server |
| openrsync | ISC | rsync-protocol-27 drop-in (replaces GPL rsync) |
| headscale | BSD-3-Clause | Self-hosted Tailscale coordination server |
| derper | BSD-3-Clause | Tailscale DERP relay |

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
| pico | Apache-2.0 | Terminal text editor (alpine) |
| gitoxide | MIT/Apache-2.0 | Git implementation in Rust |
| ripgrep | MIT | Fast recursive grep |
| mandoc | ISC | Man page tools |
| pigz | Zlib | Parallel gzip |
| bsdtar | BSD-2-Clause | Archive tool (libarchive) |
| doas | ISC | Privilege escalation |
| fastfetch | MIT | System information |
| btop | Apache-2.0 | Terminal resource monitor |
| tmux | ISC | Terminal multiplexer |
| jonerix-raspi5-fixups | 0BSD | Pi 5 hardware fixups (EEE disable, pwm-fan thermal control, DNS takeover opt-out, wake-on-power, cold-reboot) |

## Rust drop-in replacements

jonerix's permissive-license-only rule removes most of the traditional
Linux userland: bash (GPL-3), GNU coreutils (GPL-3), GNU make (GPL-3),
e2fsprogs (LGPL-2 / GPL-2), util-linux (mixed GPL / LGPL), nftables
(GPL-2), libnl (LGPL-2.1), m4 (GPL-3), readline (GPL-3), and so on.
Where a permissive equivalent already exists (toybox for the BSD
coreutils surface, openrsync for rsync, mksh for the POSIX shell) jonerix
uses it. Where none exists, jonerix grows its own — almost always in
Rust, almost always clean-room.

The clean-room rule is taken seriously. Most of these tools are
written from upstream documentation, POSIX text, IETF RFCs, kernel
UAPI headers, and differential test corpora — no GPL or LGPL source
is consulted. A few (`jfsck`, `nloxide`) are explicitly derived from
Ghidra binary analysis of the original tools' compiled artifacts; the
Ghidra session notes are pinned alongside the source so the
provenance is auditable. Each replacement carries its own conformance
suite that diffs its output against the original GPL tool byte-for-byte
on a corpus of real-world inputs (e.g. `jmake`'s `JMAKE_TEST_MODE=1`
gate runs the bash 5.3 test suite and the `musl`/`expat`/`dropbear`/
`toybox` build trees against GNU make 4.4.1).

### In-house

These are written and maintained inside the jonerix project. 

| Replaces | Package | License | Notes |
|----------|---------|---------|-------|
| GNU bash 5.3 | `brash` | MIT | Full bash surface — `[[ ]]`, regex match, indexed and associative arrays, `${var:offset:len}`, here-docs, command/arithmetic/process substitution, traps, history, `printf`/`test`/`read`/`declare`/`mapfile`/`compgen`. Byte-equivalent to bash 5.3 across the upstream test suite plus 1100+ realworld / dash-POSIX / mksh / shellcheck / shfmt corpora. Installs `/bin/brash` and a `/bin/bash` symlink — `/bin/sh` stays mksh. |
| GNU make 4.4.1 | `jmake` | MIT | Recursive-descent parser, second-expansion (`$$`), pattern rules, target-specific variables, grouped targets (`&:`), `.NOTPARALLEL`, `.WAIT`, double-colon, `--shuffle`, parallel scheduler with `.WAIT` ordering. `JMAKE_TEST_MODE=1` flips error-prefix and `--version` output to GNU make's exact bytes for differential testing. Recent fix (1.1.14): MAKEFLAGS now backslash-escapes spaces inside variable values so a child process doesn't tokenise `CFLAGS=-Wall -O2` and reinterpret the trailing `-O2` as `--output-sync`. |
| GNU m4 | `m4oxide` | MIT | Build-time only. Used by autoconf-generated `./configure` scripts (e.g. flex). Implements the GNU m4 surface used by autoconf macros — `divert`, `dnl`, `define`, `pushdef`/`popdef`, `m4exit`, frozen-state files. |
| POSIX `expr(1)` | `exproxide` | MIT | Tiny but load-bearing — autoconf scripts call `expr` on every configure step. POSIX-strict integer / string / regex semantics. |
| GNU libreadline / libhistory | `readlineoxide` | MIT | Drop-in shared library at `/lib/libreadline.so` and `/lib/libhistory.so`. Programs linked against readline (e.g. anvil's `debugfs`) get line editing, history, and Emacs/vi keybindings without pulling in GPL-3. |
| `nft` CLI / nftables userland **and** OpenBSD `pf(8)` (on Linux) | `stormwall` | MIT | The only firewall front-end on jonerix — and the only one anywhere that speaks both Linux's nftables DSL and OpenBSD's `pf` DSL against the same kernel backend. Reads/emits the upstream `nft` ruleset language (covers `dynset`, ct helpers, flowtables, `jhash`/`symhash`/`numgen`, `dup`/`fwd`/`synproxy`, NAT random/persistent, socket/cgroup/cpu/rt classid expressions, and an `nft -i` REPL). Also accepts `pf.conf` syntax — `pass`/`block`/`match`, anchors, tables, `quick`, `keep state`, `nat`, `rdr`, `set skip`, queue/altq remapping — and lowers it to the same in-kernel netfilter rules so a `pf.conf` carried over from a BSD box runs unmodified on a jonerix Linux host. Installs `/bin/stormwall` plus `/bin/nft` and `/bin/pfctl` symlinks (each binary chooses its parser by argv[0]). |
| libnl-3 / libnl-genl-3 | `nloxide` | BSD-2-Clause | Netlink message construction + Generic Netlink for hostapd / wpa_supplicant. Derived from Ghidra binary analysis of the libnl shared objects — no LGPL source consulted. |
| e2fsprogs (mkfs.ext4, e2fsck, tune2fs, debugfs, resize2fs, dumpe2fs, ...) | `anvil` | MIT | Full ext2/3/4 userland in pure Rust. Group descriptors, extent trees, journal replay, htree dirs, large-EA, encrypted-name handling. Replaces toybox's `blkid`, `chattr`, `lsattr` via `replaces = ["toybox"]` so jpkg transfers ownership cleanly. |
| util-linux (lscpu, hwclock, ionice, nsenter, chsh) | `jonerix-util` | 0BSD | Surface-equivalent for the util-linux subset jonerix actually uses. `chsh` only allows shells listed in `/etc/shells`; `nsenter` covers the namespace flags container runtimes need; `hwclock` talks to `/dev/rtc0` directly. |
| e2fsck + fsck.fat (rescue scope) | `jfsck` | BSD-2-Clause | Scoped to Pi 5 boot recovery — ext4 journal replay + FAT32 boot-partition repair. Derived from Ghidra binary analysis of e2fsprogs and dosfstools. |
| lsusb | `lsusb-rs` | MIT | Pure-sysfs lsusb (no libusb dependency). Reads `/sys/bus/usb/devices/*` and the bundled USB IDs database. |
| jpkg (the jonerix package manager itself, 2.0+) | `jpkg` | MIT | Translation of the C jpkg 1.1.5 (~9.5K LOC) to Rust (~11.7K LOC). Byte-equivalent on every wire format the C tool defined: `JPKG\x00\x01\x00\x00` magic + LE32 header + TOML metadata + zstd(tar) payload, the `/var/db/jpkg/installed/<name>/{metadata.toml,files}` layout, the INDEX TOML grammar, the Ed25519 detached `.sig` flow, the merged-/usr layout audit, and the `replaces = […]` ownership-transfer semantics. 158 in-crate unit tests, plus an end-to-end smoke pass (build → install into a tempdir rootfs → info → verify clean → tamper-detect → remove). `#![forbid(unsafe_code)]` at the crate root keeps the safety budget at zero. The 2.0 release supersedes the C 1.1.5 series; `/bin/jpkg`, `/bin/jpkg-local`, and `/bin/jpkg-conform` continue to ship from the same `packages/jpkg/` recipe. |

### Third-party

These are upstream Rust projects we ship as-is from their canonical
sources, vetted for license compatibility:

| Replaces | Project | License | Notes |
|----------|---------|---------|-------|
| GNU coreutils (sort, wc, tr, cut, head, tail, ...70+ tools) | `uutils` | MIT | Replaces toybox multicall symlinks for the commands uutils provides; `replaces = ["toybox"]` lets jpkg flip the `/bin/<cmd>` symlinks and `post_remove` hands them back if uutils is uninstalled. |
| git | `gitoxide` | MIT or Apache-2.0 | `gix` and `ein` binaries. Read-mostly client — fast `clone`, `fetch`, `log`, blame; not a full server-side replacement. |
| grep | `ripgrep` | MIT | Default `/bin/rg`. Faster than GNU grep on the kinds of trees jonerix CI walks (recipe corpus, build logs). |

### Relationship to toybox and mksh

These Rust tools coexist with toybox (BSD-licensed coreutils-of-the-week
multicall) and mksh (the POSIX `/bin/sh`). The split is deliberate:

- **toybox + mksh** are the static-linked, syscall-light, always-present
  base layer — what's in `Dockerfile.minimal`, what survives a damaged
  rootfs, what runs the early-boot OpenRC scripts.
- **The Rust replacements** are larger binaries with richer feature
  coverage. They take over `/bin/<name>` paths via jpkg's `replaces` /
  `post_install` / `post_remove` mechanism so toybox and mksh remain
  available as fallbacks and as the second-pass providers if a Rust
  package is uninstalled.

The same `replaces` ownership flip is what lets brash provide
`/bin/bash` without ever touching `/bin/sh` — bash-isms route through
brash, POSIX scripts route through mksh, and uninstalling brash leaves
a `/bin/bash → mksh` (or `→ toybox`) symlink behind so `#!/bin/bash`
scripts don't fall over with "exec format error".

## Package Manager (jpkg)

jpkg is a custom, 0BSD-licensed package manager built for jonerix. Packages are zstd-compressed tarballs signed with Ed25519.

```sh
jpkg update                      # fetch latest package index
jpkg search fastfetch            # search available packages
jpkg install fastfetch           # install a package
jpkg list                        # list installed packages
jpkg local install ./pkg.jpkg    # install a .jpkg from a local file, URL, or stdin
jpkg local build ./recipe-dir    # build a recipe.toml and either emit a .jpkg or install it
jpkg conform 1.2.1               # pin the host to a specific jonerix release tag
```

`jpkg local` and `jpkg conform` are external subcommands shipped inside the jpkg package (`/bin/jpkg-local`, `/bin/jpkg-conform`); jpkg's main dispatcher falls through to them via PATH.

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

## Raspberry Pi 5

jonerix has first-class support for the Pi 5. Everything below is
installed automatically by the `jonerix-raspi5-fixups` package, which
jpkg pulls in as part of the default rootfs for aarch64 images.

### Defaults out of the box

| setting                   | default        | how to change |
| ------------------------- | -------------- | ------------- |
| Kernel reboot mode        | **cold**       | auto — see "cold reboot" below |
| Auto-boot on power restore| **enabled**    | `sudo pi5-wake-on-power disable` |
| RTC coin-cell charging    | **disabled**   | add `dtparam=rtc_bbat_vchg=3000000` to `/boot/config.txt` (see "RTC battery" below) |
| Energy-Efficient Ethernet | **disabled**   | edit `/etc/init.d/disable-eee` or remove the runlevel symlink |
| PWM fan cooling           | **enabled**    | `rc-update del fan-control boot` |
| Onboard Wi-Fi             | **enabled**    | `rc-update del pi5-wifi boot` |

### Cold reboot

Pi 5's RP1 southbridge cannot be reset by a warm reboot, so `reboot`
hangs the board whenever the kernel is in warm mode. The Pi 5
firmware unconditionally prepends `reboot=w` to the kernel command
line, so jonerix does two things to pin the kernel to **cold** mode:

1. `apply-pi5-cold-reboot` prepends `reboot=c` to
   `/boot/cmdline.txt` at install time. The kernel uses the *last*
   `reboot=` token on the line, so this wins. A one-time backup is
   saved to `/boot/cmdline.txt.pre-pi5-fixups`.
2. The `pi5-cold-reboot` OpenRC service writes `cold` to
   `/sys/kernel/reboot/mode` on every boot as belt-and-suspenders.

The install hook also flips the sysfs knob live, so `reboot` works
immediately after `jpkg install jonerix-raspi5-fixups` without
needing a round-trip reboot first.

### Auto-boot on power restore (wake-on-power)

The Pi 5 EEPROM ships with `WAKE_ON_GPIO=1` and `POWER_OFF_ON_HALT=0`,
which is exactly what you want for a headless box — pulling and
restoring power cold-boots the system automatically. jonerix does
**not** touch the EEPROM by default; factory settings are left alone.

To inspect and manage it:

```sh
sudo pi5-wake-on-power              # show current EEPROM config
sudo pi5-wake-on-power enable       # force WAKE_ON_GPIO=1 / POWER_OFF_ON_HALT=0
sudo pi5-wake-on-power disable      # POWER_OFF_ON_HALT=1 (stay off after halt)
```

`enable` and `disable` both write through the VideoCore mailbox into
EEPROM and persist across reboots. The `disable` preference sticks —
nothing in jonerix will silently re-enable wake-on-power.

The `pi5-cold-reboot` service prints the current wake-on-power state
at every boot so it's auditable via `rc-status` / `dmesg`.

### RTC battery (coin cell)

Pi 5 has an on-board RTC with a `J5` header for a backup coin cell.
Trickle charging is **off by default** — wrong cell chemistry (e.g.
an accidental CR2032 in an ML2032 socket) can vent or ignite a
non-rechargeable cell, so we require an explicit opt-in.

To enable trickle charging at 3.0 V (safe for ML2032 / MS621FE
rechargeables):

```sh
echo 'dtparam=rtc_bbat_vchg=3000000' | sudo tee -a /boot/config.txt
sudo reboot
```

Once charged, the RTC keeps time across full power cycles. Verify
with `hwclock -r` after a power cut.

### Firmware inspection (pi5-fw)

`pi5-fw` is a zero-dependency Rust stand-in for the pieces of
`vcgencmd` and `rpi-eeprom-config` we care about. It talks to the
VideoCore mailbox directly via `/dev/vcio`:

```sh
sudo pi5-fw measure_temp         # SoC temperature
sudo pi5-fw get_throttled        # under-voltage / throttle flags
sudo pi5-fw firmware_version     # VideoCore firmware revision
sudo pi5-fw board_info           # model, revision, serial
sudo pi5-fw clock_rates          # ARM current / max clock
sudo pi5-fw bootloader_config    # boot mode, reset status, cmdline reboot=
sudo pi5-fw reboot_mode          # current kernel reboot mode
sudo pi5-fw reboot_mode cold     # set kernel reboot mode (cold/warm/hard/soft/gpio)
```

### Console login defaults — what jonerix ships and how to change them

The image ships `tty1-console`, an OpenRC service that respawns
`/bin/login` on `/dev/tty1`, plus an unlocked `root` account:

```sh
# /etc/shadow as shipped
root::0:0:99999:7:::
```

The empty second field (the password hash) is what toybox `login`
reads as "no password required". **A fresh jonerix image lets anyone
at the console log in as root without a password.** That's
intentional for first-boot bring-up — no headache pairing a display
with an SSH keyboard just to set up Wi-Fi — but you'll want to lock
it down before exposing the box:

```sh
sudo passwd root      # set a password
# or
sudo passwd -l root   # lock the account entirely (SSH keys still work)
```

The other console quirk people notice: after typing nothing for
~60 seconds at the `login:` prompt the shell clears and respawns a
fresh one. That's toybox `login`'s hard-coded timeout; it's not a
tty1-console setting. The service loops so the new prompt appears
instantly. Replacing `/bin/login` with a different permissive-
licensed implementation (shadow is GPL-2; util-linux is GPL-2; both
are out) is the only real fix — BSD-licensed alternatives exist but
none are packaged in jonerix yet.

### Other Pi 5 fixups in the same package

- **EEE (Energy-Efficient Ethernet) disable** on the BCM54213PE PHY
  — the Pi 5's integrated PHY loses link during LPI transitions.
- **PWM fan driver** auto-load — the Pi 5 Active Cooler header needs
  `pwm-fan.ko` loaded before the cooling device appears in sysfs.
- **Onboard Wi-Fi bring-up** — the brcmfmac stack asks for
  `brcmfmac43455-sdio*.bin`, but the shipped blob is under its
  Cypress name; symlinks are created at install time.
- **fstab rescue** — older Pi images shipped an incomplete fstab
  (no `/dev/pts`, `/sys`, tmpfs `/run` or `/tmp`); missing entries
  are appended and the running system is live-mounted.
- **errors=remount-ro** added to the root ext4 line so SD-card wear
  triggers a fail-safe remount instead of silent corruption.

## Building a Pi 5 image

`install/pi5-install.sh` writes a bootable jonerix image onto a raw block
device — SD card, USB stick, or NVMe. It runs on any POSIX host.

### Prerequisites

| Tool | Notes |
| ---- | ----- |
| `mount` / `umount` | standard on any Linux host |
| `curl` | for firmware download |
| `bsdtar` (preferred) or `tar` | toybox tar mishandles pax long-name headers |
| `mkfs.ext4` / `mkfs.vfat` | only needed if the target disk is unformatted; provided by `jpkg install anvil` |

If `mkfs.ext4` or `mkfs.vfat` are absent and the target partition is not
already formatted, the script bails with an explicit message:

```
error: /dev/sdX2 is not ext4 and mkfs.ext4 is unavailable. Pre-format it.
```

Pre-format the partitions by hand (p1 FAT32, p2 ext4) and re-run, or install
`anvil` first (`jpkg install anvil`) and then re-run.

The target device must already have two partitions (p1, p2). The script will
not repartition from scratch; use `sfdisk` or a similar tool first.

### Quick start — curl-into-sh

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/pi5-install.sh \
  | sudo sh -s -- -y -d /dev/sdX
```

Replace `/dev/sdX` with the actual block device. `-y` / `--yes` accepts all
prompts (device confirmation, license acceptance, firmware download, userland
install) on the operator's behalf.

### Interactive walk-through

Run from a jonerix host (or any POSIX host with `jpkg` on PATH):

```sh
sudo install/pi5-install.sh
```

The script proceeds through these stages in order:

1. **Device selection.** Scans `/sys/block` for removable devices and
   presents a numbered list. Pass `-d /dev/sdX` to skip the prompt.

2. **Confirmation.** Warns that the target will be overwritten. Requires an
   explicit `y`.

3. **Filesystem check.** Probes p1 for FAT32 and p2 for ext4. Offers to
   format them if `mkfs.vfat` / `mkfs.ext4` are available; bails otherwise.

4. **License acceptance.** Before touching the network, prints the full text
   of both covered licenses and requires an explicit `y`:
   - Linux kernel — GPL-2.0 (`kernel_2712.img`, device-tree blobs)
   - Broadcom firmware — proprietary binary (`start4.elf`, `fixup4.dat`,
     `LICENCE.broadcom`); free to redistribute with Raspberry Pi hardware,
     not modifiable

   `--yes` / `-y` accepts both on the operator's behalf. A
   `LICENSES-ACCEPTED.txt` file is written to the boot partition recording
   the timestamp and which licenses were accepted.

5. **Firmware download.** Fetches `raspberrypi/firmware` (stable branch,
   ~500 MiB) from GitHub, extracts only the `boot/` payload onto p1, and
   discards the tarball. The boot partition is cleared first; any existing
   `*.pre-pi5-fixups` backups are preserved.

6. **Userland install.** Runs `jpkg -r <root> install <packages>` to
   populate p2 with the default package set (see table below). Requires
   `jpkg` on the host PATH; the script aborts with a clear message if it is
   not found.

7. **Config.** Writes `/boot/cmdline.txt` (UUID-based root, `reboot=c`
   first so it wins over the firmware's `reboot=w`), `/boot/config.txt`
   (`hdmi_force_hotplug` on both HDMI ports, RTC trickle charging commented
   out), and `/etc/fstab`.

8. **Verify + unmount.** Checks that all boot-critical files are present,
   syncs, and unmounts cleanly.

### Flags

| Flag | Effect |
| ---- | ------ |
| `-y` / `--yes` | Accept all prompts, including license acceptance |
| `-d PATH` / `--device PATH` | Target block device (skip interactive scan) |
| `--no-firmware` | Skip firmware download; reuse whatever is on p1 already |
| `--no-userland` | Skip jpkg install; stop after p1 is populated |
| `--branch NAME` | Pull recipes and helpers from this jonerix branch (default: `main`) |

Unattended scripted form:

```sh
sudo install/pi5-install.sh -y -d /dev/sdX
```

### What ends up on the disk

**p1 — FAT32 (boot)**

| File | Purpose |
| ---- | ------- |
| `kernel_2712.img` | Pi 5 kernel (ARM Cortex-A76 64-bit) |
| `bcm2712-rpi-5-b.dtb` | Device-tree blob |
| `start4.elf` / `fixup4.dat` | Broadcom VideoCore firmware blobs |
| `cmdline.txt` | `reboot=c console=serial0,115200 root=UUID=… rootfstype=ext4 rootwait rw init=/bin/openrc-init` |
| `config.txt` | `arm_64bit=1`, `enable_uart=1`, `gpu_mem=16`, `hdmi_force_hotplug:0/1=1` |
| `LICENCE.broadcom` | Broadcom firmware license (from upstream tarball) |
| `LICENSES-ACCEPTED.txt` | Acceptance record with timestamp |

**p2 — ext4 (root, label `jonerix`)**

Default package set installed by jpkg:

| Package | Role |
| ------- | ---- |
| `musl` | C standard library |
| `toybox` | Base coreutils |
| `mksh` | Shell |
| `openrc` | Init system (`/bin/openrc-init`) |
| `dhcpcd` | DHCP client |
| `dropbear` | SSH server |
| `bsdtar` | Archive tool |
| `python3` | Scripting |
| `sudo` | Privilege escalation |
| `anvil` | mkfs.ext4 / mkfs.vfat / e2fsck / blkid and friends (MIT clean-room) |
| `jonerix-raspi5-fixups` | Cold-reboot service, wake-on-power, EEE disable, PWM fan, Wi-Fi shim, fstab rescue |
| `jonerix-boot-helpers` | Boot-time helper scripts |
| `openntpd` | NTP client |

Tailscale is deliberately not in the default set. Install it after first boot
with `jpkg install tailscale`.

OpenRC services wired into the `boot` runlevel by `jonerix-raspi5-fixups`:
`pi5-cold-reboot`, `pi5-wake-on-power`, `pi5-rtc-battery-check`,
`pi5-wifi`, `disable-eee`, `fan-control`.

### Re-running

The script is safe to re-run. On the first edit, `cmdline.txt` and
`config.txt` are backed up as `cmdline.txt.pre-pi5-fixups` and
`config.txt.pre-pi5-fixups`. Subsequent runs overwrite those files in place
but leave the `*.pre-pi5-fixups` backups untouched.

## fastfetch

```
                                    root@jonerix-tormenta
   _                       _        ---------------------
  (_) ___  _ __   ___ _ __(_)_  __  OS -> jonerix 1.2.1 aarch64
  | |/ _ \| '_ \ / _ \ '__| \ \/ /  Kernel -> Linux 6.18.22-v8-16k+
  | | (_) | | | |  __/ |  | |>  <   Uptime -> 16 hours, 28 mins
 _/ |\___/|_| |_|\___|_|  |_/_/\_\  Packages -> 44 (jpkg)
|__/                                 Shell -> mksh
======= permissive + linux =======   Terminal -> dropbear
                                     Editor -> pico
                                     CPU -> BCM2712 (4) @ 2.40 GHz
                                     Memory -> 150.14 MiB / 3.95 GiB (4%)
                                     Disk (/) -> 8.87 GiB / 29.03 GiB (31%) - ext4
                                     Local IP (eth0) -> 10.0.0.8/16
```

## License

All original jonerix code is released under the **BSD Zero Clause License**
([0BSD](https://opensource.org/licenses/0BSD)). 0BSD imposes no attribution,
notice-retention, or sublicensing requirements — copy and paste freely.
The full text is in [`LICENSE`](LICENSE).
