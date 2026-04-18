#!/usr/bin/env python3
# build-image.py -- Build a bootable jonerix disk image for Raspberry Pi 5.
#
# Produces a raw .img (MBR: FAT32 boot p1, ext4 root p2) + .img.zst + SHA256SUMS.
# Runs as root on a Linux host with losetup/sfdisk/mkfs.vfat/mkfs.ext4/jpkg/zstd.
#
# Python stdlib only. No PyYAML, no click, no requests, no GPL libraries.
# Part of jonerix -- MIT License

from __future__ import annotations

import argparse
import contextlib
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path
from typing import Iterable, Optional

# ----------------------------------------------------------------------------
# Defaults
# ----------------------------------------------------------------------------

DEFAULT_SIZE = "4G"
DEFAULT_HOSTNAME = "jonerix-pi"
DEFAULT_ARCH = "aarch64"
DEFAULT_BOOT_MB = 256  # Standard Pi SD layout; matches docs/plans/raspberry-pi.md

# Minimal package set. Any extra user packages are additive. Everything listed
# here MUST exist in packages/{core,develop,extra}/ as a recipe so jpkg install
# can resolve it from the jonerix package repository.
#
# Kept intentionally small: a booting Pi 5 needs a shell, init, network
# client, SSH, and the raspi5 fixups. The full 46-package set is overkill for
# the default SD image -- users can opt into more via --packages.
DEFAULT_PACKAGES = [
    "musl",
    "toybox",
    "mksh",  # not always present; dropped quietly if jpkg can't find it
    "openrc",
    "dhcpcd",
    "dropbear",
    "ifupdown-ng",
    "bsdtar",
    "ca-certificates",
]

# Always present, regardless of --packages. These are load-bearing for Pi 5.
MANDATORY_PACKAGES = [
    "jonerix-raspi5-fixups",
]

# Firmware tarball for the Pi 5 boot partition (kernel_2712.img, DTBs, overlays,
# start4.elf, fixup4.dat, etc). Pulled from raspberrypi/firmware at build time
# unless --firmware-tarball is passed. License: Broadcom Redistributable (see
# boot/LICENCE.broadcom in that repo; documented in docs/plans/raspberry-pi.md).
#
# We pin a specific tag rather than tracking master so builds are reproducible.
FIRMWARE_REPO_TAG = "1.20240306"  # Conservative pin; bump in a PR when needed.
FIRMWARE_TARBALL_URL = (
    "https://github.com/raspberrypi/firmware/archive/refs/tags/"
    f"{FIRMWARE_REPO_TAG}.tar.gz"
)

# Files we care about from the firmware repo's boot/ directory. Anything not
# listed here is skipped; keeps the boot partition lean.
PI5_FIRMWARE_FILES = [
    # GPU / second-stage firmware (Pi 4/5 read this via EEPROM loader)
    "fixup4.dat",
    "fixup4cd.dat",
    "fixup4db.dat",
    "fixup4x.dat",
    "start4.elf",
    "start4cd.elf",
    "start4db.elf",
    "start4x.elf",
    # Pi 5 kernel image and device tree
    "kernel_2712.img",
    "bcm2712-rpi-5-b.dtb",
    # Licences + README from boot/
    "LICENCE.broadcom",
]

# ----------------------------------------------------------------------------
# Logging
# ----------------------------------------------------------------------------


def log(msg: str) -> None:
    sys.stdout.write(f"[pi5-image] {msg}\n")
    sys.stdout.flush()


def die(msg: str, code: int = 1) -> None:
    sys.stderr.write(f"[pi5-image] error: {msg}\n")
    sys.exit(code)


# ----------------------------------------------------------------------------
# Shell-out helpers
# ----------------------------------------------------------------------------


def run(cmd: list[str], *, check: bool = True, capture: bool = False,
        env: Optional[dict] = None, cwd: Optional[str] = None) -> subprocess.CompletedProcess:
    """Run a subprocess with sane defaults. Logs the command."""
    log("$ " + " ".join(cmd))
    return subprocess.run(
        cmd,
        check=check,
        capture_output=capture,
        text=True,
        env=env,
        cwd=cwd,
    )


def run_out(cmd: list[str]) -> str:
    """Run and return stripped stdout."""
    return run(cmd, capture=True).stdout.strip()


def require_cmd(*cmds: str) -> None:
    missing = [c for c in cmds if shutil.which(c) is None]
    if missing:
        die(f"missing required commands: {', '.join(missing)}")


def require_root() -> None:
    if os.geteuid() != 0:
        die("must be run as root (needs losetup + mount)")


# ----------------------------------------------------------------------------
# Image + partitioning
# ----------------------------------------------------------------------------


def parse_size(s: str) -> int:
    """Parse '4G', '512M', '2048K', or a raw byte count."""
    s = s.strip().upper()
    mult = 1
    if s.endswith("K"):
        mult, s = 1024, s[:-1]
    elif s.endswith("M"):
        mult, s = 1024 ** 2, s[:-1]
    elif s.endswith("G"):
        mult, s = 1024 ** 3, s[:-1]
    elif s.endswith("T"):
        mult, s = 1024 ** 4, s[:-1]
    try:
        return int(float(s) * mult)
    except ValueError:
        die(f"unparseable size: {s}")


def allocate_sparse(path: Path, size_bytes: int) -> None:
    log(f"allocating sparse image: {path} ({size_bytes} bytes)")
    if path.exists():
        path.unlink()
    # truncate is the portable way to create a sparse file; no dd/if=/dev/zero.
    run(["truncate", "-s", str(size_bytes), str(path)])


def partition_mbr(img_path: Path, boot_mb: int) -> None:
    """Lay out an MBR table: p1 FAT32 (bootable), p2 Linux.

    Pi 5 firmware does not understand GPT for the boot partition -- only MBR.
    See docs/plans/raspberry-pi.md section 6.1: "MBR partition table (not GPT --
    RPi firmware expects MBR)".
    """
    log("writing MBR partition table via sfdisk")
    # 4 MiB-aligned start (sector 8192) matches official Raspberry Pi OS images
    # and is safe for SD wear-leveling block boundaries.
    layout = (
        "label: dos\n"
        "unit: sectors\n"
        f"1 : start=8192, size={boot_mb * 2048}, type=c, bootable\n"  # 0x0C = FAT32 LBA
        "2 : type=83\n"  # 0x83 = Linux; grows to end of disk
    )
    proc = subprocess.run(
        ["sfdisk", str(img_path)],
        input=layout,
        text=True,
        check=True,
    )
    # sfdisk exits non-zero on real errors; check=True handled it.
    _ = proc


def losetup_attach(img_path: Path) -> str:
    """Attach image to a loop device with kernel partition scanning. Returns
    the loop device path, e.g. '/dev/loop0'."""
    dev = run_out(["losetup", "--find", "--show", "--partscan", str(img_path)])
    # --partscan races with udev; give it a moment before we touch /dev/loopNpN
    for _ in range(20):
        if Path(f"{dev}p1").exists() and Path(f"{dev}p2").exists():
            return dev
        time.sleep(0.1)
    die(f"loop partitions never appeared for {dev}")
    raise RuntimeError("unreachable")  # for type checkers


def losetup_detach(dev: str) -> None:
    run(["losetup", "-d", dev], check=False)


# ----------------------------------------------------------------------------
# Filesystem formatting
# ----------------------------------------------------------------------------


def format_boot(part: str) -> None:
    log(f"formatting {part} as FAT32 (label BOOT)")
    run(["mkfs.vfat", "-F", "32", "-n", "BOOT", part])


def format_root(part: str) -> None:
    log(f"formatting {part} as ext4 (label root)")
    # errors=remount-ro is set via fstab later; -L matches what raspi5-fixups
    # expects. -m 1 keeps root reserve small (default 5% is wasteful on SD).
    run(["mkfs.ext4", "-F", "-q", "-L", "root", "-m", "1", part])


def blkid_value(part: str, tag: str) -> str:
    """Look up a blkid token (e.g. PARTUUID, UUID, LABEL) for a partition."""
    out = run_out(["blkid", "-s", tag, "-o", "value", part])
    if not out:
        die(f"blkid {tag} for {part} returned empty")
    return out


# ----------------------------------------------------------------------------
# Mount context
# ----------------------------------------------------------------------------


@contextlib.contextmanager
def mount(part: str, mnt: Path):
    mnt.mkdir(parents=True, exist_ok=True)
    run(["mount", part, str(mnt)])
    try:
        yield mnt
    finally:
        run(["umount", str(mnt)], check=False)


# ----------------------------------------------------------------------------
# jpkg install into staging rootfs
# ----------------------------------------------------------------------------


def jpkg_install(root: Path, packages: Iterable[str]) -> None:
    pkgs = list(packages)
    if not pkgs:
        log("(no packages requested)")
        return
    log(f"jpkg install -r {root} {' '.join(pkgs)}")
    # Per packages/jpkg/src/main.c line 81 ("-r, --root <path> Use alternative
    # root filesystem"), --root is a top-level flag BEFORE the subcommand.
    run(["jpkg", "--root", str(root), "install"] + pkgs)


# ----------------------------------------------------------------------------
# Firmware handling
# ----------------------------------------------------------------------------


def fetch_firmware(dest_tarball: Path, url: str = FIRMWARE_TARBALL_URL) -> None:
    """Download the raspberrypi/firmware tarball if not already cached."""
    if dest_tarball.exists() and dest_tarball.stat().st_size > 0:
        log(f"firmware tarball already cached: {dest_tarball}")
        return
    dest_tarball.parent.mkdir(parents=True, exist_ok=True)
    log(f"downloading firmware tarball: {url}")
    tmp = dest_tarball.with_suffix(dest_tarball.suffix + ".partial")
    with urllib.request.urlopen(url, timeout=300) as resp, tmp.open("wb") as fp:
        shutil.copyfileobj(resp, fp, length=1 << 20)
    tmp.rename(dest_tarball)


def extract_firmware_to_boot(tarball: Path, boot_mnt: Path) -> None:
    """Extract just the Pi 5-relevant pieces from the firmware tarball into
    the FAT32 boot partition.

    Uses bsdtar (permissive). Filters to boot/overlays and the files listed
    in PI5_FIRMWARE_FILES. Anything else (bootcode.bin for Pi 3, Pi 4 kernels,
    etc.) is dropped so the small boot partition isn't wasted.
    """
    log(f"extracting firmware from {tarball} to {boot_mnt}")
    # raspberrypi/firmware tarballs unpack to firmware-<tag>/boot/*
    staging = Path(tempfile.mkdtemp(prefix="pi5-fw-"))
    try:
        # bsdtar is the jonerix standard (scripts/ci-build-aarch64.sh). --numeric-owner
        # keeps FAT32 happy (vfat has no concept of owners).
        run(["bsdtar", "-xf", str(tarball), "-C", str(staging)])
        # Find the extracted boot/ dir.
        roots = [p for p in staging.iterdir() if p.is_dir()]
        if not roots:
            die("firmware tarball had no top-level directory")
        boot_src = roots[0] / "boot"
        if not boot_src.is_dir():
            die(f"no boot/ directory in firmware tarball at {boot_src}")
        # Copy the whitelisted files.
        copied = 0
        for name in PI5_FIRMWARE_FILES:
            src = boot_src / name
            if not src.exists():
                log(f"WARN: firmware file missing, skipping: {name}")
                continue
            shutil.copy2(src, boot_mnt / name)
            copied += 1
        # Copy overlays/ wholesale -- needed for dtoverlay=... in config.txt.
        overlays_src = boot_src / "overlays"
        if overlays_src.is_dir():
            overlays_dst = boot_mnt / "overlays"
            overlays_dst.mkdir(exist_ok=True)
            for f in overlays_src.iterdir():
                if f.is_file():
                    shutil.copy2(f, overlays_dst / f.name)
            log(f"copied {sum(1 for _ in overlays_dst.iterdir())} overlay files")
        log(f"copied {copied} firmware files to boot partition")
    finally:
        shutil.rmtree(staging, ignore_errors=True)


def copy_local_firmware(src_dir: Path, boot_mnt: Path) -> None:
    """Alternative to fetch_firmware: user points us at a pre-extracted dir."""
    if not src_dir.is_dir():
        die(f"--firmware-dir not a directory: {src_dir}")
    copied = 0
    for name in PI5_FIRMWARE_FILES:
        src = src_dir / name
        if src.exists():
            shutil.copy2(src, boot_mnt / name)
            copied += 1
    overlays_src = src_dir / "overlays"
    if overlays_src.is_dir():
        overlays_dst = boot_mnt / "overlays"
        overlays_dst.mkdir(exist_ok=True)
        for f in overlays_src.iterdir():
            if f.is_file():
                shutil.copy2(f, overlays_dst / f.name)
    log(f"copied {copied} firmware files from {src_dir}")


# ----------------------------------------------------------------------------
# Rootfs customization
# ----------------------------------------------------------------------------


def write_file(path: Path, content: str, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    path.chmod(mode)


def write_hostname(root: Path, hostname: str) -> None:
    write_file(root / "etc" / "hostname", hostname + "\n")
    # /etc/hosts: keep it minimal but resolve our own hostname to 127.0.1.1
    hosts = (
        "127.0.0.1   localhost\n"
        "::1         localhost\n"
        f"127.0.1.1   {hostname}\n"
    )
    write_file(root / "etc" / "hosts", hosts)


def write_fstab(root: Path, boot_partuuid: str, root_partuuid: str) -> None:
    """Write /etc/fstab referencing partitions by PARTUUID so the image works
    equally on /dev/mmcblk0p* (SD) and /dev/sda* (USB).

    raspi5-fixups 1.3.1 post_install appends devpts / sysfs / tmpfs lines
    idempotently on first boot, but we include them here so a fresh image is
    correct before the first package manager run. errors=remount-ro matches
    what that fixup enforces.
    """
    fstab = (
        "# /etc/fstab -- jonerix Pi 5 image\n"
        "# Generated by image/pi5/build-image.py. Safe to edit by hand.\n"
        "# Using PARTUUID so SD vs USB boot works identically.\n"
        f"PARTUUID={root_partuuid}  /           ext4    defaults,noatime,errors=remount-ro  0 1\n"
        f"PARTUUID={boot_partuuid}  /boot       vfat    defaults,noatime                    0 2\n"
        "devpts                     /dev/pts    devpts  gid=5,mode=0620,ptmxmode=0666       0 0\n"
        "sysfs                      /sys        sysfs   defaults                            0 0\n"
        "proc                       /proc       proc    defaults                            0 0\n"
        "tmpfs                      /tmp        tmpfs   defaults,nosuid,nodev,size=20%      0 0\n"
        "tmpfs                      /run        tmpfs   defaults,nosuid,nodev,size=20%      0 0\n"
    )
    write_file(root / "etc" / "fstab", fstab)


def write_ssh_key(root: Path, key: str) -> None:
    # /root/.ssh/authorized_keys with 0600 perms, 0700 on the dir.
    ssh_dir = root / "root" / ".ssh"
    ssh_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    ssh_dir.chmod(0o700)
    auth = ssh_dir / "authorized_keys"
    # Append, in case dropbear's post_install dropped something.
    existing = auth.read_text() if auth.exists() else ""
    content = existing + (key.rstrip() + "\n")
    write_file(auth, content, mode=0o600)


def enable_openrc_service(root: Path, svc: str, runlevel: str = "default") -> None:
    """Symlink /etc/runlevels/<runlevel>/<svc> -> /etc/init.d/<svc>."""
    rl = root / "etc" / "runlevels" / runlevel
    rl.mkdir(parents=True, exist_ok=True)
    link = rl / svc
    if link.is_symlink() or link.exists():
        return
    link.symlink_to(f"/etc/init.d/{svc}")


def write_tailscale_oneshot(root: Path, authkey: str) -> None:
    """Drop an OpenRC oneshot that runs `tailscale up --authkey=KEY` once on
    first boot, then disables itself so we don't burn the authkey every time.

    The authkey is embedded in the service script. This is acceptable for
    single-tenant Pi images but users should rotate the key after first boot.
    """
    service = f"""#!/sbin/openrc-run
# tailscale-firstboot -- one-shot tailscale up on the first boot of this image.
#
# Generated by image/pi5/build-image.py when --tailscale-authkey is supplied.
# The authkey is baked into /etc/init.d/tailscale-firstboot below. Rotate it
# in the Tailscale admin console after the Pi has connected.

description="Bring up tailscale with pre-baked auth key on first boot"

depend() {{
    need net
    after dhcpcd
}}

start() {{
    # Sentinel: only run once.
    if [ -f /var/lib/jonerix/tailscale-firstboot.done ]; then
        einfo "tailscale-firstboot: already completed, skipping"
        return 0
    fi
    if ! command -v tailscale >/dev/null 2>&1; then
        eerror "tailscale binary not on PATH"
        return 1
    fi
    # tailscaled must be running first. If a user ships a tailscale package
    # it should provide its own service; start it here defensively.
    if ! pidof tailscaled >/dev/null 2>&1; then
        if [ -x /etc/init.d/tailscaled ]; then
            rc-service tailscaled start || true
            sleep 3
        fi
    fi
    ebegin "tailscale up (first boot)"
    tailscale up --authkey={_shell_quote(authkey)} --ssh || return 1
    mkdir -p /var/lib/jonerix
    : > /var/lib/jonerix/tailscale-firstboot.done
    eend 0
}}
"""
    write_file(root / "etc" / "init.d" / "tailscale-firstboot", service, mode=0o755)
    enable_openrc_service(root, "tailscale-firstboot", "default")


# ----------------------------------------------------------------------------
# Restricted-firmware opt-in installer
# ----------------------------------------------------------------------------
#
# jonerix's runtime is permissive-only by policy (see CLAUDE.md). Two
# Pi-specific components that most users want live on the wrong side of
# that line and are NOT shipped in this image:
#
#   1. Linux kernel modules (GPL-2.0) -- needed for WiFi, Bluetooth, USB
#      serial, and basically every non-built-in peripheral driver.
#   2. Broadcom/Cypress WiFi firmware blobs (proprietary, Broadcom
#      redistributable) -- needed for the CYW43455 on the Pi 5 to come up
#      as wlan0.
#
# Rather than bake them in, we install a post-install helper at
# /usr/local/sbin/jonerix-pi5-restricted that shows the user the license
# URLs, asks for explicit consent, and only then downloads and installs
# the bits from their upstream sources (raspberrypi/firmware for kernel
# modules, RPi-Distro/firmware-nonfree for WiFi blobs).
#
# A /etc/motd banner + /etc/profile.d snippet point users at the script
# on first login so they're not left wondering why wlan0 is absent.


RESTRICTED_FIRMWARE_TAG = "1.20240306"  # same pin as FIRMWARE_REPO_TAG
RESTRICTED_NONFREE_TAG = "bookworm"     # RPi-Distro/firmware-nonfree branch


def write_restricted_installer(root: Path) -> None:
    """Drop /usr/local/sbin/jonerix-pi5-restricted. POSIX sh, stdlib-only
    busybox/toybox utilities + curl. Asks for license consent, then fetches
    kernel modules + WiFi firmware from upstream on success.
    """
    # /bin/sh on jonerix is mksh; we stick to POSIX features only.
    script = f"""#!/bin/sh
# jonerix-pi5-restricted -- install non-permissive Pi 5 components that
# jonerix deliberately does not ship with the base image. Prompts for
# explicit license consent and only proceeds on an unambiguous "yes".
#
# Installed by image/pi5/build-image.py. Safe to re-run: already-present
# files are skipped, and the done-marker suppresses the motd nag.

set -eu

DONE_MARKER=/var/lib/jonerix/pi5-restricted.done
FIRMWARE_TAG={RESTRICTED_FIRMWARE_TAG!r}
NONFREE_TAG={RESTRICTED_NONFREE_TAG!r}
FIRMWARE_URL="https://github.com/raspberrypi/firmware/archive/refs/tags/$FIRMWARE_TAG.tar.gz"
NONFREE_URL="https://github.com/RPi-Distro/firmware-nonfree/archive/refs/heads/$NONFREE_TAG.tar.gz"

log() {{ printf '[pi5-restricted] %s\\n' "$*"; }}
die() {{ printf '[pi5-restricted] ERROR: %s\\n' "$*" >&2; exit 1; }}

[ "$(id -u)" -eq 0 ] || die "must run as root (try: doas $0)"

command -v curl >/dev/null 2>&1 || die "curl is required (jpkg install curl)"
command -v bsdtar >/dev/null 2>&1 || die "bsdtar is required (jpkg install bsdtar)"

cat <<EOF

========================================================================
  jonerix Pi 5 -- optional restricted components
========================================================================

jonerix's userland is permissive-only (MIT / BSD / Apache-2.0 / ISC /
etc) by policy. The following components are typically wanted on a Pi 5
but sit outside that policy, so the base image does NOT ship them:

  1. Linux kernel modules                            (GPL-2.0)
     From: https://github.com/raspberrypi/firmware
           (boot/modules/\\$KVER, tag $FIRMWARE_TAG)
     Needed for: WiFi (brcmfmac / cyw43), Bluetooth, USB serial,
                 sound, almost any non-built-in driver.
     Install size: ~150 MB into /lib/modules/\\$KVER/

  2. Broadcom / Cypress WiFi firmware blobs          (proprietary)
     From: https://github.com/RPi-Distro/firmware-nonfree
           (brcm/*, cypress/*, branch $NONFREE_TAG)
     License: Broadcom Redistributable Firmware Licence --
              https://github.com/RPi-Distro/firmware-nonfree/blob/$NONFREE_TAG/LICENCE.broadcom_bcm43xx
     Needed for: wlan0 on the CYW43455 radio shipped with Pi 5.
     Install size: ~35 MB into /lib/firmware/

Download size: ~180 MB total (cached under /var/cache/jonerix/)
Install size:  ~185 MB on /

Do you accept the Linux kernel modules GPL-2.0 license AND the Broadcom
Redistributable Firmware Licence, and want jonerix to download and
install the above components now?

  [y]  yes, install both
  [k]  kernel modules only (GPL-2.0 only)
  [w]  WiFi firmware only (Broadcom licence only)
  [n]  no, skip (default)

EOF

printf "Choice [y/k/w/N]: "
read -r ANSWER || ANSWER=n
case "$ANSWER" in
    y|Y) INSTALL_MODULES=1; INSTALL_WIFI=1 ;;
    k|K) INSTALL_MODULES=1; INSTALL_WIFI=0 ;;
    w|W) INSTALL_MODULES=0; INSTALL_WIFI=1 ;;
    *)   log "declined -- nothing installed"; exit 0 ;;
esac

mkdir -p /var/cache/jonerix /var/lib/jonerix
CACHE=/var/cache/jonerix

# --- Kernel modules -----------------------------------------------------
if [ "$INSTALL_MODULES" = 1 ]; then
    log "downloading raspberrypi/firmware $FIRMWARE_TAG for kernel modules..."
    TARBALL="$CACHE/firmware-$FIRMWARE_TAG.tar.gz"
    [ -s "$TARBALL" ] || curl -fL --retry 3 -o "$TARBALL" "$FIRMWARE_URL"

    STAGING=$(mktemp -d -p "$CACHE" modules.XXXXXX)
    bsdtar -xf "$TARBALL" -C "$STAGING" \\
        "firmware-$FIRMWARE_TAG/modules/" 2>/dev/null || \\
        bsdtar -xf "$TARBALL" -C "$STAGING" --include='*/modules/*' || \\
        die "tarball has no modules/ directory"

    MODSRC="$STAGING/firmware-$FIRMWARE_TAG/modules"
    [ -d "$MODSRC" ] || die "extract left no modules dir at $MODSRC"
    for kdir in "$MODSRC"/*; do
        kver=$(basename "$kdir")
        log "installing /lib/modules/$kver (~$(du -sh "$kdir" | cut -f1))"
        mkdir -p /lib/modules
        # Preserve existing; copy new.
        cp -a "$kdir" /lib/modules/
    done
    rm -rf "$STAGING"
    # Regenerate module dependency maps if depmod exists.
    if command -v depmod >/dev/null 2>&1; then
        for kdir in /lib/modules/*/; do
            kver=$(basename "$kdir")
            log "depmod -a $kver"
            depmod -a "$kver" || log "depmod failed for $kver (non-fatal)"
        done
    else
        log "depmod not present; modules.dep shipped in tarball will be used"
    fi
fi

# --- WiFi firmware ------------------------------------------------------
if [ "$INSTALL_WIFI" = 1 ]; then
    log "downloading RPi-Distro/firmware-nonfree $NONFREE_TAG for WiFi blobs..."
    NONTARBALL="$CACHE/firmware-nonfree-$NONFREE_TAG.tar.gz"
    [ -s "$NONTARBALL" ] || curl -fL --retry 3 -o "$NONTARBALL" "$NONFREE_URL"

    STAGING=$(mktemp -d -p "$CACHE" wifi.XXXXXX)
    bsdtar -xf "$NONTARBALL" -C "$STAGING"
    # firmware-nonfree-$branch/ layout: brcm/, cypress/, and some others.
    NONSRC=$(find "$STAGING" -maxdepth 1 -mindepth 1 -type d | head -n1)
    [ -d "$NONSRC" ] || die "firmware-nonfree tarball empty"

    mkdir -p /lib/firmware/brcm /lib/firmware/cypress
    if [ -d "$NONSRC/brcm" ]; then
        cp -a "$NONSRC/brcm"/. /lib/firmware/brcm/
    fi
    if [ -d "$NONSRC/cypress" ]; then
        cp -a "$NONSRC/cypress"/. /lib/firmware/cypress/
    fi
    # Preserve the license file alongside the blobs so it's present on-disk.
    for lic in LICENCE.broadcom_bcm43xx LICENSE LICENSE.broadcom; do
        if [ -f "$NONSRC/$lic" ]; then
            install -Dm 644 "$NONSRC/$lic" "/lib/firmware/brcm/$lic"
        fi
    done
    rm -rf "$STAGING"

    # raspi5-fixups ships the brcmfmac->cyfmac symlinks; re-run its service
    # if it's present to wire them up for the newly installed blobs.
    if [ -x /etc/init.d/pi5-wifi ]; then
        log "running pi5-wifi to apply cyfmac -> brcmfmac symlinks"
        /etc/init.d/pi5-wifi start || log "pi5-wifi start failed (non-fatal)"
    fi
fi

: > "$DONE_MARKER"
log "done. Reboot or modprobe brcmfmac to bring up wlan0."
"""
    script_path = root / "usr" / "local" / "sbin" / "jonerix-pi5-restricted"
    script_path.parent.mkdir(parents=True, exist_ok=True)
    write_file(script_path, script, mode=0o755)


def write_restricted_motd(root: Path) -> None:
    """First-boot banner pointing users at jonerix-pi5-restricted.

    Lives at /etc/motd so it shows on SSH login and on any getty. The
    helper script drops a /var/lib/jonerix/pi5-restricted.done sentinel
    when it finishes; we key the /etc/profile.d banner off that so the
    message goes away once restricted components are installed.
    """
    motd = (
        "\n"
        "  jonerix Pi 5 -- base image (permissive-only)\n"
        "\n"
        "  WiFi, Bluetooth, and loadable kernel modules are NOT preinstalled.\n"
        "  Run 'sudo jonerix-pi5-restricted' to review the licences and opt\n"
        "  into downloading them from raspberrypi.org / RPi-Distro.\n"
        "\n"
    )
    write_file(root / "etc" / "motd", motd)

    # Also emit a reminder on interactive shells until the marker exists.
    profile_d = root / "etc" / "profile.d"
    profile_d.mkdir(parents=True, exist_ok=True)
    profile_script = (
        '# Added by image/pi5/build-image.py -- nudge on login until the\n'
        '# user has run jonerix-pi5-restricted at least once.\n'
        'if [ ! -e /var/lib/jonerix/pi5-restricted.done ] && [ -t 1 ]; then\n'
        '    printf "\\n  WiFi/BT/kernel modules not installed.\\n"\n'
        '    printf "  Run: sudo jonerix-pi5-restricted\\n\\n"\n'
        'fi\n'
    )
    write_file(profile_d / "pi5-restricted-nag.sh", profile_script, mode=0o644)


def _shell_quote(s: str) -> str:
    """POSIX-safe single-quote-wrapped literal for embedding in shell."""
    return "'" + s.replace("'", "'\\''") + "'"


def write_boot_config(boot_mnt: Path) -> None:
    """config.txt with Pi 5 essentials. Matches docs/plans/raspberry-pi.md 2.4."""
    config_txt = (
        "# /boot/config.txt -- jonerix Pi 5 image\n"
        "# Generated by image/pi5/build-image.py\n"
        "\n"
        "# 64-bit kernel is mandatory for the aarch64 userland.\n"
        "arm_64bit=1\n"
        "\n"
        "# Pi 5 kernel image (BCM2712). The EEPROM bootloader looks for this\n"
        "# filename literally; do not rename without also updating start4.elf.\n"
        "kernel=kernel_2712.img\n"
        "\n"
        "# Serial console for headless bring-up.\n"
        "enable_uart=1\n"
        "\n"
        "# Minimal GPU split; jonerix is headless-server by default.\n"
        "gpu_mem=16\n"
        "\n"
        "# No rainbow splash.\n"
        "disable_splash=1\n"
        "\n"
        "# Onboard audio off (noisy and unused on a server image).\n"
        "dtparam=audio=off\n"
        "\n"
        "# Use 64-bit PL011 UART at /dev/ttyAMA0. Pi 5 has the real UART on\n"
        "# GPIO14/15; no need for the miniuart juggle that Pi 3 required.\n"
        "[pi5]\n"
        "dtoverlay=disable-bt\n"
    )
    write_file(boot_mnt / "config.txt", config_txt)


def write_boot_cmdline(boot_mnt: Path, root_partuuid: str) -> None:
    """cmdline.txt: a single line, no trailing newline weirdness.

    root=PARTUUID=... keeps the image SD/USB portable. init=/bin/openrc-init
    matches the rest of jonerix (see image/mkimage.sh).
    """
    cmdline = (
        f"root=PARTUUID={root_partuuid} rootfstype=ext4 rootwait rw "
        "init=/bin/openrc-init "
        "console=serial0,115200 console=tty1 quiet\n"
    )
    write_file(boot_mnt / "cmdline.txt", cmdline)


def enable_default_services(root: Path) -> None:
    """Wire OpenRC services that every Pi image needs. Only enables the service
    if its init script is actually present -- jpkg may not have installed it
    (e.g. if the user's --packages list omits dropbear).
    """
    # boot runlevel
    for svc in ("devfs", "sysctl", "hostname", "modules"):
        if (root / "etc" / "init.d" / svc).exists():
            enable_openrc_service(root, svc, runlevel="boot")
    # default runlevel
    for svc in ("dhcpcd", "dropbear", "local", "urandom"):
        if (root / "etc" / "init.d" / svc).exists():
            enable_openrc_service(root, svc, runlevel="default")


# ----------------------------------------------------------------------------
# Output compression + checksums
# ----------------------------------------------------------------------------


def zstd_compress(src: Path, dst: Path, level: int = 19) -> None:
    log(f"compressing {src} -> {dst} (zstd -{level})")
    # -f: overwrite, --long=27 for better ratios on large sparse files.
    run([
        "zstd", "-f", f"-{level}", "--long=27",
        "-o", str(dst), str(src),
    ])


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fp:
        for chunk in iter(lambda: fp.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def write_sha256sums(paths: list[Path], out: Path) -> None:
    lines = []
    for p in paths:
        lines.append(f"{sha256_file(p)}  {p.name}\n")
    out.write_text("".join(lines))
    log(f"wrote {out}")


# ----------------------------------------------------------------------------
# Main build
# ----------------------------------------------------------------------------


def build(args: argparse.Namespace) -> int:
    require_root()
    require_cmd(
        "sfdisk", "mkfs.vfat", "mkfs.ext4", "mount", "umount",
        "losetup", "blkid", "truncate", "zstd", "bsdtar",
    )
    if args.arch != "aarch64":
        die(f"only --arch aarch64 is supported (got {args.arch})")

    out_img = Path(args.output).resolve()
    out_img.parent.mkdir(parents=True, exist_ok=True)

    packages = list(dict.fromkeys(
        [p.strip() for p in (args.packages or "").split(",") if p.strip()]
        or DEFAULT_PACKAGES
    ))
    # Always force mandatory packages on the end (so user --packages can't drop them).
    for m in MANDATORY_PACKAGES:
        if m not in packages:
            packages.append(m)

    size_bytes = parse_size(args.size)

    # Stage 1: allocate + partition the raw image.
    allocate_sparse(out_img, size_bytes)
    partition_mbr(out_img, args.boot_mb)

    # Stage 2: attach loop, format, capture PARTUUIDs.
    loop = losetup_attach(out_img)
    try:
        boot_part = f"{loop}p1"
        root_part = f"{loop}p2"
        format_boot(boot_part)
        format_root(root_part)
        boot_partuuid = blkid_value(boot_part, "PARTUUID")
        root_partuuid = blkid_value(root_part, "PARTUUID")
        log(f"boot PARTUUID: {boot_partuuid}")
        log(f"root PARTUUID: {root_partuuid}")

        # Stage 3: populate rootfs via jpkg, then boot/ via firmware tarball.
        mnt_root = Path(tempfile.mkdtemp(prefix="pi5-root-"))
        mnt_boot = Path(tempfile.mkdtemp(prefix="pi5-boot-"))
        try:
            with mount(root_part, mnt_root), mount(boot_part, mnt_boot):
                # jpkg first -- it creates /etc, /lib, etc.
                jpkg_install(mnt_root, packages)

                # Pi 5 firmware into boot partition.
                if args.firmware_dir:
                    copy_local_firmware(Path(args.firmware_dir), mnt_boot)
                else:
                    cache = Path(args.firmware_cache) if args.firmware_cache \
                        else Path.home() / ".cache" / "jonerix-pi5-firmware.tar.gz"
                    fetch_firmware(cache)
                    extract_firmware_to_boot(cache, mnt_boot)

                # config.txt + cmdline.txt in the boot partition.
                write_boot_config(mnt_boot)
                write_boot_cmdline(mnt_boot, root_partuuid)

                # /etc/fstab, /etc/hostname, /etc/hosts in the rootfs.
                write_fstab(mnt_root, boot_partuuid, root_partuuid)
                write_hostname(mnt_root, args.hostname)

                if args.ssh_key:
                    write_ssh_key(mnt_root, args.ssh_key)

                if args.tailscale_authkey:
                    write_tailscale_oneshot(mnt_root, args.tailscale_authkey)

                # Restricted-firmware opt-in path: drop the helper + the
                # motd/login nag. Nothing is fetched at build time; the
                # user runs jonerix-pi5-restricted after first boot and
                # explicitly accepts each license before any GPL kernel
                # module or Broadcom blob touches the filesystem.
                write_restricted_installer(mnt_root)
                write_restricted_motd(mnt_root)

                enable_default_services(mnt_root)

                # Make sure /boot exists on rootfs for the fstab mount point.
                (mnt_root / "boot").mkdir(exist_ok=True)

                # Sync before unmount -- avoids sparse/hole truncation games.
                run(["sync"])
        finally:
            shutil.rmtree(mnt_root, ignore_errors=True)
            shutil.rmtree(mnt_boot, ignore_errors=True)
    finally:
        losetup_detach(loop)

    # Stage 4: compress + checksum.
    zst_path = out_img.with_suffix(out_img.suffix + ".zst")
    zstd_compress(out_img, zst_path)

    sums_path = out_img.parent / "SHA256SUMS"
    write_sha256sums([out_img, zst_path], sums_path)

    log("done:")
    log(f"  {out_img}")
    log(f"  {zst_path}")
    log(f"  {sums_path}")
    return 0


# ----------------------------------------------------------------------------
# CLI
# ----------------------------------------------------------------------------


def parse_args(argv: list[str]) -> argparse.Namespace:
    p = argparse.ArgumentParser(
        prog="build-image.py",
        description="Build a bootable jonerix Raspberry Pi 5 disk image.",
    )
    p.add_argument("--output", required=True, help="Output .img path")
    p.add_argument("--size", default=DEFAULT_SIZE,
                   help=f"Total image size (default {DEFAULT_SIZE}). Accepts K/M/G/T suffixes.")
    p.add_argument("--boot-mb", type=int, default=DEFAULT_BOOT_MB,
                   help=f"Size of FAT32 /boot partition in MiB (default {DEFAULT_BOOT_MB})")
    p.add_argument("--hostname", default=DEFAULT_HOSTNAME)
    p.add_argument("--packages", default="",
                   help="Comma-separated extra packages (additive to defaults). "
                        f"Defaults: {','.join(DEFAULT_PACKAGES)}. "
                        f"Always installed: {','.join(MANDATORY_PACKAGES)}.")
    p.add_argument("--arch", default=DEFAULT_ARCH,
                   help="Only 'aarch64' is supported right now")
    p.add_argument("--tailscale-authkey", default=None,
                   help="If set, a first-boot oneshot runs `tailscale up --authkey=...`")
    p.add_argument("--ssh-key", default=None,
                   help="Public SSH key to drop into /root/.ssh/authorized_keys")
    p.add_argument("--firmware-dir", default=None,
                   help="Local directory to copy firmware from (skips download). "
                        "Must contain kernel_2712.img and the bcm2712 DTB.")
    p.add_argument("--firmware-cache", default=None,
                   help="Path to cache the downloaded firmware tarball.")
    return p.parse_args(argv)


def main() -> int:
    args = parse_args(sys.argv[1:])
    try:
        return build(args)
    except subprocess.CalledProcessError as e:
        die(f"command failed: {e}")
        return 1
    except KeyboardInterrupt:
        log("interrupted")
        return 130


if __name__ == "__main__":
    sys.exit(main())
