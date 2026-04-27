# jonerix Raspberry Pi 5 SD image builder

`image/pi5/build-image.py` produces a bootable jonerix disk image for a
Raspberry Pi 5 that boots identically from an SD card or a USB stick.

It is a single Python 3 file with no third-party dependencies (stdlib only --
no PyYAML, no click, no requests). It shells out to `sfdisk`, `losetup`,
`mkfs.vfat`, `mkfs.ext4`, `mount`, `blkid`, `jpkg`, `bsdtar`, and `zstd` --
all tools already present in the jonerix builder containers.

## What the image contains

| Partition | Filesystem | Size | Contents |
|---|---|---|---|
| `/dev/sdX1` | FAT32 (label `BOOT`) | 256 MiB (configurable) | Pi 5 firmware, DTBs, overlays, `kernel_2712.img`, `config.txt`, `cmdline.txt` |
| `/dev/sdX2` | ext4 (label `root`) | rest of the disk | jonerix rootfs populated by `jpkg install -r /mnt` |

The partition table is **MBR**, not GPT. The Pi 5 EEPROM bootloader does not
recognise GPT for the boot partition. `image/mkimage.sh` (x86_64) uses GPT;
that pattern is wrong for Raspberry Pi.

All device references (`/etc/fstab`, `cmdline.txt`) use `PARTUUID=` rather
than `/dev/mmcblk0p*` so the same image boots from SD (`/dev/mmcblk0`) and
from USB (`/dev/sda`) without modification.

### Default package set

| Package | Why |
|---|---|
| `musl` | libc |
| `toybox` | coreutils + `modprobe` with `.ko.xz` decompression |
| `mksh` | login shell |
| `openrc` | init (`/bin/openrc-init`) |
| `dhcpcd` | DHCP client, runs by default at boot |
| `dropbear` | SSH server (MIT) |
| `ifupdown-ng` | interface config |
| `bsdtar` | permissive tar |
| `ca-certificates` | TLS trust |
| `jonerix-raspi5-fixups` | **Always installed**: Pi 5 EEE, fan control, onboard WiFi bring-up, fstab rescue, adduser safety |

Pass `--packages foo,bar` to add more. `jonerix-raspi5-fixups` is mandatory
and added automatically even if the user passes `--packages`.

### Kernel and firmware

The tool downloads a pinned tag of
[`raspberrypi/firmware`](https://github.com/raspberrypi/firmware) at build
time and extracts just the Pi 5-relevant pieces (`kernel_2712.img`,
`bcm2712-rpi-5-b.dtb`, `start4.elf`, `fixup4.dat`, `overlays/`). Use
`--firmware-dir <path>` to point at a pre-fetched copy (useful in CI to
avoid repeated downloads, or when building offline).

The firmware is Broadcom Redistributable -- same exception category as the
Linux kernel. See the README's "Raspberry Pi 5" section for the license
rationale.

## Usage

```sh
# Minimal: 4 GB image with default packages
sudo python3 image/pi5/build-image.py --output jonerix-pi5.img

# Larger image, custom hostname, extra packages, bake in an SSH key + Tailscale auth key
sudo python3 image/pi5/build-image.py \
    --output jonerix-pi5.img \
    --size 8G \
    --hostname jonerix-tormenta \
    --packages micro,btop,tmux,nerdctl,containerd,runc,cni-plugins \
    --ssh-key "$(cat ~/.ssh/id_ed25519.pub)" \
    --tailscale-authkey tskey-auth-xxxx
```

Output:

```
jonerix-pi5.img          # raw image (sparse)
jonerix-pi5.img.zst      # zstd-compressed, ready to distribute
SHA256SUMS               # checksums for both
```

### Arguments

| Flag | Default | Notes |
|---|---|---|
| `--output` | (required) | Path to the raw `.img` file |
| `--size` | `4G` | Total image size. `K`/`M`/`G`/`T` suffixes accepted |
| `--boot-mb` | `256` | FAT32 boot partition size in MiB |
| `--hostname` | `jonerix-pi` | Written to `/etc/hostname` and `/etc/hosts` |
| `--packages` | (see above) | Comma-separated, additive to defaults |
| `--arch` | `aarch64` | Only `aarch64` is supported right now |
| `--ssh-key` | none | Full authorized_keys line (e.g. `"ssh-ed25519 AAAA..."`) |
| `--tailscale-authkey` | none | If set, a first-boot OpenRC oneshot runs `tailscale up --authkey=<key> --ssh` |
| `--firmware-dir` | none | Skip the download; copy firmware from this local directory |
| `--firmware-cache` | `~/.cache/jonerix-pi5-firmware.tar.gz` | Where to cache the firmware tarball |

## Flashing

Write the compressed image directly to the SD card or USB stick with `zstd`
and `dd`. Replace `/dev/sdX` with the actual device (check with `lsblk` --
getting this wrong overwrites your laptop):

```sh
zstd -d < jonerix-pi5.img.zst | sudo dd of=/dev/sdX bs=4M status=progress conv=fsync
sync
```

Or on macOS:

```sh
zstd -d < jonerix-pi5.img.zst | sudo dd of=/dev/rdiskN bs=4m
```

Raspberry Pi Imager (Apache-2.0) also works: point it at the `.img.zst` (or
decompress first).

## First boot

- The Pi boots from the BOOT partition -- Pi 5 EEPROM reads `config.txt` then
  loads `kernel_2712.img` + `bcm2712-rpi-5-b.dtb`.
- OpenRC starts as PID 1, runs `jonerix-raspi5-fixups` (disable-eee,
  fan-control, pi5-wifi) and `dhcpcd`.
- If `--ssh-key` was provided, dropbear listens on port 22; log in as
  `root@<hostname>.local`.
- If `--tailscale-authkey` was provided, `tailscale-firstboot` runs *once*
  after dhcpcd gets an address. The sentinel
  `/var/lib/jonerix/tailscale-firstboot.done` prevents re-running.

## CI artifacts

`.github/workflows/publish-pi5-image.yml` runs this tool on every push of a
tag matching `pi5-image-v*` and on manual `workflow_dispatch`. Output lands
on the GitHub release for the tag:

```
https://github.com/stormj-UH/jonerix/releases/download/<tag>/jonerix-pi5.img.zst
https://github.com/stormj-UH/jonerix/releases/download/<tag>/SHA256SUMS
```

One-liner flash from the release:

```sh
curl -L https://github.com/stormj-UH/jonerix/releases/download/pi5-image-v0.1.0/jonerix-pi5.img.zst \
  | zstd -d \
  | sudo dd of=/dev/sdX bs=4M status=progress conv=fsync
```

On manual dispatch (not a tag push), the artifacts are uploaded to the
workflow run as a zip bundle instead of a release.

## Tools used, and why each one is necessary

| Tool | License | Why |
|---|---|---|
| `truncate` | (toybox 0BSD or coreutils) | Create sparse disk image without writing zeros |
| `sfdisk` | util-linux (GPL, build-time only; never ships) | Lay out MBR partition table scriptably |
| `losetup` | util-linux (build-time only) | Attach image file so kernel partscan sees p1/p2 |
| `mkfs.vfat` | dosfstools (build-time only) | Format FAT32 boot partition |
| `mkfs.ext4` | e2fsprogs (build-time only) | Format ext4 root partition |
| `mount`/`umount` | util-linux (build-time only) | Populate partitions via filesystem mount |
| `blkid` | util-linux (build-time only) | Look up PARTUUID after formatting, for fstab/cmdline |
| `jpkg` | jonerix (MIT) | Install the rootfs by resolving recipes from the jonerix package repo |
| `bsdtar` | libarchive (BSD-2-Clause) | Extract the firmware tarball; matches the rest of jonerix |
| `zstd` | BSD/GPLv2 dual (BSD chosen) | Compress the final image for distribution |
| `sha256sum` | toybox (0BSD) | Integrity manifest |

GPL tools listed above (util-linux, dosfstools, e2fsprogs) are used **only
at image-build time in an Alpine-style container**. They never ship in the
image. This matches jonerix's existing build-time GPL policy (same as
GNU make for bootstrap).

## Known gaps / rough edges

- **Kernel modules and Broadcom WiFi/BT firmware are NOT shipped.** This is
  deliberate: jonerix's runtime is permissive-only (MIT / BSD / Apache-2.0 /
  ISC), so Linux kernel modules (GPL-2.0) and Broadcom's redistributable
  firmware blobs sit on the wrong side of the licensing policy and won't be
  baked into images automatically.

  Instead, every Pi 5 image ships `/usr/local/sbin/jonerix-pi5-restricted`
  and a `/etc/motd` banner pointing users at it. On first run the script
  shows the upstream license URLs (Linux kernel `COPYING`, Broadcom
  Redistributable Firmware Licence) and asks for an unambiguous `y/k/w/N`
  consent; only on `y`/`k`/`w` does it download and install anything.

  Sources:
  - Kernel modules -- `raspberrypi/firmware` (same pin as the boot-partition
    firmware), extracted to `/lib/modules/<kver>/`.
  - WiFi / BT firmware -- `RPi-Distro/firmware-nonfree`, `brcm/` and
    `cypress/` subtrees, extracted to `/lib/firmware/`.

  After the script runs, `raspi5-fixups`'s `pi5-wifi` service (if present)
  wires up the `brcmfmac -> cyfmac` symlinks so the CYW43455 radio comes up
  as `wlan0` on the next `modprobe brcmfmac` or reboot. Wired ethernet and
  SSH work out of the box on the base image without this step.
- **Tailscale binary is assumed to be in a user-supplied package.** The
  first-boot service runs `tailscale up` but doesn't install `tailscale`;
  add `tailscale` (or whatever you call it) to `--packages` if you want
  it. A `tailscaled` OpenRC service likewise must be provided by that
  package.
- **`mksh` isn't in the default recipe set yet.** If `mksh` is missing,
  `jpkg install` will error out and the build fails. Either add it to
  `packages/extra/mksh/` or drop it from the default list -- the
  script prints a readable error.
- **GitHub hosted runners allow `sudo losetup`** (used by the CI workflow)
  but do not allow raw loop device creation by non-root. The workflow runs
  the Python script inside the `jonerix:builder-arm64` Docker container
  with `--privileged`, same pattern as `publish-packages.yml`.
- **Pi 4 / Pi 3 are not supported by this script.** Adding them would
  require shipping `start.elf`/`start4.elf` + `kernel8.img` and a different
  DTB set. Out of scope for the Pi 5 MVP.
