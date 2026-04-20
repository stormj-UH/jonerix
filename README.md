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
