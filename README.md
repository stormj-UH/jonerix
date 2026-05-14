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

**jonerix is a self-hosting alt-Linux built around permissive licensing.**

The current tagged distro release is
**[v1.2.1](https://github.com/stormj-UH/jonerix/releases/tag/v1.2.1)**.
The tree currently tracks **106 package recipes** and builds for
`x86_64` and `aarch64`.

jonerix is a "bring your own kernel" distribution. The Linux kernel is not part
of the permissive-userland promise, and Pi firmware/kernel blobs are fetched
from Raspberry Pi upstream under their own licenses instead of being
redistributed here.

There are rough edges. The goal is not to look like a conventional distro; the
goal is to make a practical, source-built, low-friction userland where the
runtime surface is MIT/BSD/ISC/Apache/0BSD/public-domain style code.

## What It Is

jonerix is built around three rules:

| Rule | Consequence |
| ---- | ----------- |
| Permissive userland only | GPL/LGPL/AGPL packages are replaced, excluded, or kept out of the userland package set. |
| Self-host from source | The builder image can rebuild the package set with jpkg recipes and vendored source inputs. |
| POSIX-first plumbing | `/bin/sh` is mksh, core utilities are toybox/uutils, and shipped scripts avoid bashisms and GNU-only options. |

The project ships:

- a small `minimal` rootfs for containers and bootstrap work
- a richer `core` runtime with shells, networking, editor, Git replacement, and diagnostics
- a `builder` image with Clang/LLVM, Rust, Go, Python, Node.js, Perl, CMake, jmake, and samurai
- a `router` image for gateway/AP/firewall use
- Raspberry Pi 5 install and image tooling
- WSL2 rootfs import tooling

## Quick Start

Pull a published image:

```sh
docker pull ghcr.io/stormj-uh/jonerix:minimal
docker pull ghcr.io/stormj-uh/jonerix:core
docker pull ghcr.io/stormj-uh/jonerix:builder
docker pull ghcr.io/stormj-uh/jonerix:router
```

Run an interactive shell:

```sh
docker run --rm -it ghcr.io/stormj-uh/jonerix:core
docker run --rm -it ghcr.io/stormj-uh/jonerix:builder
```

Per-architecture tags are also published:

```sh
docker pull ghcr.io/stormj-uh/jonerix:builder-amd64
docker pull ghcr.io/stormj-uh/jonerix:builder-arm64
```

Build the image chain locally:

```sh
docker build -f Dockerfile.minimal --tag jonerix:minimal .
docker build -f Dockerfile.core --tag jonerix:core .
docker build -f Dockerfile.builder --tag jonerix:builder .
docker build -f Dockerfile.router --tag jonerix:router .
```

## Image Layers

| Image | Base | Contents |
| ----- | ---- | -------- |
| `minimal` | scratch | musl, toybox, dropbear, curl, LibreSSL, OpenRC, jpkg |
| `core` | minimal | mksh, zsh, uutils, pico, fastfetch, ripgrep, brash, gitredoxide, networking tools |
| `builder` | core | clang/LLVM/LLD, Rust, rustdoc, rustfmt, rustup, Go, Node.js, Python 3, Perl, CMake, jmake, samurai |
| `router` | core | jcarp, nloxide, hostapd, wpa_supplicant, stormwall, router/AP support |

The full package inventory lives in [PACKAGES.md](PACKAGES.md).

## Install Targets

### Containers

The published images are the fastest way to try jonerix. The `builder` image is
the normal development environment for package work.

```sh
docker run --rm -it \
  -v "$PWD:/workspace" \
  -w /workspace \
  ghcr.io/stormj-uh/jonerix:builder-amd64
```

### WSL2

CI publishes ready-to-import WSL rootfs tarballs on the rolling
[`packages`](https://github.com/stormj-UH/jonerix/releases/tag/packages)
release.

From a regular PowerShell:

```powershell
iwr -useb https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/wsl/install.ps1 `
  | iex
```

Launch it:

```powershell
wsl -d jonerix
```

Pinned or custom install:

```powershell
iwr -useb https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/wsl/install.ps1 `
  -OutFile $env:TEMP\jonerix-install.ps1

& $env:TEMP\jonerix-install.ps1 -InstallDir "D:\WSL\jonerix" -DistroName "jonerix-dev"
& $env:TEMP\jonerix-install.ps1 -Release "v1.2.1"
```

See [install/wsl/install.ps1](install/wsl/install.ps1) and
[install/wsl/build-rootfs.sh](install/wsl/build-rootfs.sh).

### Raspberry Pi 5

The small wrapper at [install/jonerix-pi5.sh](install/jonerix-pi5.sh) downloads
and runs the full installer at [install/pi5-install.sh](install/pi5-install.sh).

Fresh install to an attached SD, USB, or NVMe device:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5.sh \
  | sudo sh -s -- -d /dev/sdX
```

Pin the package set to a tagged release:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5.sh \
  | sudo sh -s -- -d /dev/sdX --release-tag v1.2.1
```

Complete a CI image after `dd` by adding the firmware/kernel payload that the
image deliberately does not redistribute:

```sh
curl -fsSL https://raw.githubusercontent.com/stormj-UH/jonerix/main/install/jonerix-pi5.sh \
  | sudo sh -s -- -d /dev/sdX --firmware-only
```

Pi 5 specifics shipped by `jonerix-raspi5-fixups`:

| Area | jonerix default |
| ---- | --------------- |
| Reboot mode | Pins the kernel to cold reboot so RP1 resets reliably. |
| Wake on power | Leaves factory wake-on-power behavior enabled and auditable. |
| RTC coin cell | Trickle charging is off until explicitly enabled in `/boot/config.txt`. |
| Ethernet | Disables EEE on the BCM54213PE PHY to avoid LPI link loss. |
| Cooling | Loads PWM fan support for the Pi 5 Active Cooler. |
| Wi-Fi | Adds firmware symlinks and OpenRC bring-up for onboard Wi-Fi. |
| Console | Ships a first-boot console login path; set or lock the root password before exposing hardware. |

Useful Pi commands after boot:

```sh
sudo pi5-fw measure_temp
sudo pi5-fw bootloader_config
sudo pi5-wake-on-power
sudo passwd root
```

More Pi image details live in [image/pi5/README.md](image/pi5/README.md).

## Package Manager

`jpkg` is the jonerix package manager. Packages are signed
zstd-compressed tar archives with TOML metadata and a deterministic installed
database under `/var/db/jpkg/installed`.

Common commands:

```sh
jpkg update
jpkg search fastfetch
jpkg install fastfetch
jpkg list
jpkg local install ./pkg.jpkg
jpkg local build ./packages/core/jpkg
jpkg conform 1.2.1
```

The current source tree packages `jpkg` **2.2.3**. The Rust implementation
keeps the C jpkg wire formats compatible and keeps the crate itself
`unsafe`-free.

## Self-Hosting Toolchain

The builder image can rebuild the toolchain stack from jpkg recipes:

| Toolchain | Status |
| --------- | ------ |
| C/C++ | Clang, LLVM, LLD, compiler-rt, libc++, and libunwind. |
| Rust | Rust 1.95.0 for `x86_64-jonerix-linux-musl` and `aarch64-jonerix-linux-musl`, plus split `rustdoc`, `rustfmt`, and `rustup` packages. |
| Go | Bootstrapped from C through the Go bootstrap chain. |
| Scripting | Python 3, Node.js, Perl, zsh, mksh, and brash. |
| Builds | jmake, samurai, CMake, byacc, flex, bc, pkgconf. |

Rust toolchain packages are split intentionally:

| Package | Owns |
| ------- | ---- |
| `rust` | `rustc`, `cargo`, standard library, system Cargo defaults |
| `rustdoc` | documentation generator and doctest runner |
| `rustfmt` | `rustfmt` and `cargo-fmt` |
| `rustup` | `rustup` and `rustup-init` only, without replacing jpkg-owned compiler proxies |

## GNO Packages

"GNO" means "GNO is not GNU": permissive replacements for GPL or LGPL pieces
that traditional Linux systems usually take for granted.

### In-House Replacements

| Replaces | Package | Current version | Notes |
| -------- | ------- | --------------- | ----- |
| Bash | `brash` | 1.0.16 | Clean-room Bash 5.3-compatible shell. Provides `/bin/bash`; `/bin/sh` stays mksh. |
| GNU make | `jmake` | 1.2.6 | Clean-room make with GNU make compatibility work and POSIX-friendly bootstrap behavior. |
| GNU libreadline/libhistory | `readlineoxide` | 0.1.12-r0 | Shared-library compatibility layer for readline users without GPL-3. |
| Git | `gitredoxide` | 1.0.21 | Drop-in `/bin/git` plus helper-mode dispatch for upload-pack and receive-pack. |
| nftables, pf, iptables front ends | `stormwall` | 1.1.11 | One firewall front end for nft, OpenBSD pf syntax, and iptables-style CLIs. |
| e2fsprogs/dosfstools surface | `anvil`, `jfsck` | see PACKAGES.md | Clean-room filesystem tools for ext2/3/4 and rescue workflows. |
| libnl | `nloxide` | 1.2.3 | Netlink and Generic Netlink for hostapd/wpa_supplicant. |
| expr | `exproxide` | 0.1.1-r0 | POSIX `expr(1)` for configure scripts. |
| util-linux subset | `jonerix-util` | see PACKAGES.md | Small replacements for the util-linux commands jonerix actually needs. |
| jpkg C implementation | `jpkg` | 2.2.3 | Rust package manager retaining the historical jpkg file formats. |

Third-party permissive replacements include `uutils` for a larger coreutils
surface and `ripgrep` for recursive search.

The clean-room boundary matters. New replacement work should use
documentation, standards text, kernel UAPI headers, test corpora, binary
behavior, and explicit provenance notes. Do not import GPL or LGPL source into
permissive replacement implementations.

## Build and Verify

Most package work should run in a jonerix builder container.

Source and recipe checks:

```sh
sh scripts/check-vendored-sources.sh
sh scripts/check-cargo-offline.sh
sh scripts/license-audit.sh
```

Rust warning gate for jpkg:

```sh
RUSTFLAGS='-D warnings' \
  cargo test --manifest-path packages/core/jpkg/Cargo.toml --locked --lib --bins
```

The same test inside the builder image, isolated from host target artifacts:

```sh
docker run --rm --platform linux/amd64 \
  --entrypoint /bin/sh \
  -v "$PWD:/workspace" \
  -w /workspace \
  ghcr.io/stormj-uh/jonerix:builder-amd64 \
  -lc "CARGO_TARGET_DIR=/tmp/jpkg-target RUSTFLAGS='-D warnings' cargo test --manifest-path packages/core/jpkg/Cargo.toml --locked --lib --bins"
```

Build one package with cached sources:

```sh
docker run --rm --platform linux/amd64 \
  --entrypoint /bin/sh \
  -v "$PWD:/workspace" \
  -w /workspace \
  ghcr.io/stormj-uh/jonerix:builder-amd64 \
  -lc 'JPKG_SOURCE_CACHE=/workspace/sources jpkg build /workspace/packages/core/jpkg --output /tmp/jpkgs'
```

Build the local image chain:

```sh
sh scripts/build-local.sh
```

The canonical package order is [scripts/build-order.txt](scripts/build-order.txt).
The full architecture and packaging model is documented in [DESIGN.md](DESIGN.md).

## Repository Map

| Path | Purpose |
| ---- | ------- |
| [packages/](packages) | Package recipes grouped as `core`, `develop`, and `extra`. |
| [sources/](sources) | Vendored source tarballs and Git LFS source inputs. |
| [scripts/build-order.txt](scripts/build-order.txt) | Dependency-respecting package build order. |
| [scripts/check-vendored-sources.sh](scripts/check-vendored-sources.sh) | Ensures every recipe source has the expected local source input. |
| [scripts/check-cargo-offline.sh](scripts/check-cargo-offline.sh) | Ensures Rust packages can build without surprise network fetches. |
| [scripts/license-audit.sh](scripts/license-audit.sh) | Recipe/userland license audit. |
| [PACKAGES.md](PACKAGES.md) | Generated package inventory and image/package mapping. |
| [DESIGN.md](DESIGN.md) | Architecture, policy, recipe format, and POSIX discipline. |
| [.github/workflows/](.github/workflows) | Package, image, Pi, WSL, Rust dist, and bootstrap CI. |

## Architecture Notes

### Merged Root

jonerix uses a merged-root layout: `/usr` is a symlink to `/`. Binaries live in
`/bin`, libraries in `/lib`, headers in `/include`, and package recipes are
audited to avoid drifting back into a split `/usr` tree.

### Licensing

Runtime packages must be permissively licensed. The allowlist includes MIT,
BSD-2-Clause, BSD-3-Clause, ISC, Apache-2.0, 0BSD, Zlib, PSF-2.0,
Artistic-2.0, MirOS, public-domain style licenses, and compatible compound
SPDX expressions.

The explicit exception is Linux itself, which is GPL-2.0-only and is built or
fetched outside the permissive userland package set.

### Shell Discipline

Anything that ships as a script should be POSIX-first:

- `#!/bin/sh`
- `[ ... ]`, not `[[ ... ]]`
- `printf`, not `echo -e`
- no GNU-only `sed -i`, `grep -P`, `readlink -f`, or bash arrays
- no hidden dependency on GNU coreutils behavior

See [DESIGN.md](DESIGN.md) for the full rule set.

## Releases

| Release surface | Location |
| --------------- | -------- |
| Tagged distro release | GitHub release tags like `v1.2.1` |
| Rolling jpkg packages | [`packages`](https://github.com/stormj-UH/jonerix/releases/tag/packages) release |
| Source mirrors | `source-*` releases plus vendored files under [sources/](sources) |
| Container images | `ghcr.io/stormj-uh/jonerix:*` |
| Pi 5 images and netboot payloads | Pi-specific workflows and release assets |
| WSL rootfs | Rolling package release assets |

Package publication is incremental: CI builds `.jpkg` files, uploads them to
the release, and regenerates a signed package index.

## fastfetch

```
    _                       _            jonerik@tormenta
   (_) ___  _ __   ___ _ __(_)_  __      ----------------
   | |/ _ \| '_ \ / _ \ '__| \ \/ /      Host -> Raspberry Pi 5 Model B Rev 1.1
   | | (_) | | | |  __/ |  | |>  <       OS -> jonerix 1.2.1 aarch64
  _/ |\___/|_| |_|\___|_|  |_/_/\_\      Init System -> openrc-init
 |__/                                    Packages -> 87 (jpkg)
 ========= permissive + linux =========  Shell -> brash
                                         Editor -> pico
                                         CPU -> BCM2712 (4) @ 2.40 GHz
```

## License

Package licenses are tracked per recipe. Repository glue and original jonerix
infrastructure default to permissive licensing; see individual
`packages/**/recipe.toml` files for package-level license truth.

The repository license text is in [LICENSE](LICENSE).
