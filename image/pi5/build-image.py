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
import re
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
DEFAULT_BOOT_MB = 256  # Standard Pi SD layout

# Minimal package set. Any extra user packages are additive. Everything listed
# here MUST exist in packages/{core,develop,extra}/ as a recipe so jpkg install
# can resolve it from the jonerix package repository.
#
# Kept intentionally small: a booting Pi 5 needs a shell, init, network
# client, SSH, and the raspi5 fixups. The full 46-package set is overkill for
# the default SD image -- users can opt into more via --packages.
DEFAULT_PACKAGES = [
    # ── Minimal boot userland ─────────────────────────────────────────
    "musl",
    "toybox",
    "mksh",       # /bin/sh
    "openrc",     # init system
    "dhcpcd",     # DHCP client
    "ifupdown-ng",
    "dropbear",   # SSH server
    "bsdtar",
    "openntpd",   # NTP client (no GPL coreutils)
    "jonerix-ntp-http-bootstrap",  # HTTP Date fallback when RTC time is stale
    "sudo",
    "python3",    # raspi-config nonint shells out to it; cheap to include
    # anvil: MIT clean-room ext2/3/4 + FAT12/16/32 userland
    # (mkfs.ext4, mkfs.vfat, e2fsck, tune2fs, debugfs, resize2fs,
    # dumpe2fs, e2image, e2label, e2freefrag, e4defrag, logsave,
    # findfs, filefrag, blkid, chattr, lsattr, mklost+found).
    "anvil",
    # raspi-config: MIT-licensed Raspberry Pi configuration tool vendored
    # from RPi-Distro/raspi-config@08a52319 (trixie branch). nonint
    # subcommands work without whiptail + parted.
    "raspi-config",

    # ── Login chain ───────────────────────────────────────────────────
    # shadow: real /bin/login with shadow-getty supervised on tty1 by
    # OpenRC. Replaces toybox's truncation-bug-prone /bin/login + passwd.
    # Without this, the Pi boots to a tty1 with no usable login prompt.
    "shadow",

    # ── Network tooling ───────────────────────────────────────────────
    # iproute-go: u-root's ip(8) replacement. toybox's ip applet can't
    # enumerate TUN devices (e.g. tailscale0) and prints "Invalid link"
    # for the whole `ip` invocation. iproute-go's recipe
    # `replaces=["toybox"]` claims /bin/ip via the package transition.
    "iproute-go",

    # ── Interactive niceties (parity with running jonerix-tormenta) ───
    # These bloat the image by ~25 MB total but make the resulting host
    # feel like a usable workstation rather than an embedded appliance.
    # Strip them via `--packages "..."` if you want a leaner image.
    "zsh",        # interactive shell w/ prompt
    "gitredoxide",   # git in pure Rust (no GPL git userland)
    "ripgrep",    # fast recursive grep
    "pico",       # text editor (apache-2.0, alpine-2.26)
    "fastfetch",  # system-info banner; pleasant on first login

    # Intentionally NOT in the default set: ca-certificates.
    # jonerix doesn't yet ship a ca-certificates jpkg; the WSL rootfs
    # curl's the Mozilla bundle from curl.se at build time instead.
]

# Always present, regardless of --packages. These are load-bearing for Pi 5.
MANDATORY_PACKAGES = [
    "jonerix-raspi5-fixups",
]

# Default release tag the image pins to. Read from
# config/defaults/etc/os-release's VERSION_ID at build time so a
# `jonerix 1.1.7` source tree builds an image whose package set is the
# v1.1.7 release set, not whatever the rolling `packages` mirror
# happens to serve at build time. This makes the image artifact
# reproducible: same source tree → same image, regardless of when
# you build.
#
# Override at runtime with --release-tag to build a "preview" image
# pointing at the rolling mirror (`packages`) or a different release.
def _default_release_tag() -> str:
    try:
        repo_root = Path(__file__).resolve().parents[2]
        osr = (repo_root / "config" / "defaults" / "etc" / "os-release").read_text()
        for line in osr.splitlines():
            if line.startswith("VERSION_ID="):
                return "v" + line.split("=", 1)[1].strip().strip('"')
    except Exception:
        pass
    return "packages"  # safe rolling fallback

DEFAULT_RELEASE_TAG = _default_release_tag()
RELEASE_BASE_URL = "https://github.com/stormj-UH/jonerix/releases/download"
ROLLING_TAG = "packages"

# Firmware tarball for the Pi 5 boot partition (kernel_2712.img, DTBs, overlays,
# start4.elf, fixup4.dat, etc). Pulled from raspberrypi/firmware at build time
# when --firmware-cache is passed, or copied from --firmware-dir. License:
# Broadcom Redistributable (see boot/LICENCE.broadcom in that repo).
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
    """
    log("writing MBR partition table via sfdisk")
    # 4 MiB-aligned start (sector 8192) matches official Raspberry Pi OS images
    # and is safe for SD wear-leveling block boundaries.
    # Partition 1 starts at sector 8192 (4 MiB-aligned) — matches official
    # Raspberry Pi OS images and is SD-friendly. Partition 2 must start
    # immediately after p1 (start = 8192 + size_of_p1) and takes all
    # remaining space. Leaving p2's start unspecified lets sfdisk pick
    # the first free region, which is the 6 KiB pre-p1 gap from sectors
    # 2048-8191 — giving you a 3 MiB "root" partition and partition-
    # table chaos. Pin it explicitly.
    p1_start = 8192
    p1_size = boot_mb * 2048
    p2_start = p1_start + p1_size
    layout = (
        "label: dos\n"
        "unit: sectors\n"
        f"1 : start={p1_start}, size={p1_size}, type=c, bootable\n"  # 0x0C = FAT32 LBA
        f"2 : start={p2_start}, type=83\n"  # 0x83 = Linux; grows to end of disk
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
    """Attach image to a loop device and make partition sub-devices appear.

    Returns the parent loop device path, e.g. '/dev/loop0'. After this
    call, '/dev/loop0p1' and '/dev/loop0p2' are expected to exist.

    Why this is fiddly: `losetup --partscan` only creates /dev/loopNpM
    devices when the loop driver has `max_part > 0`. On GitHub-hosted
    runners the loop module is loaded with max_part=0 by default, so
    --partscan silently no-ops. We work around that by calling
    `partprobe` (from util-linux) after losetup, which issues a
    BLKRRPART ioctl that the kernel honors even when max_part=0 — it
    just surfaces the partitions as block devices through a different
    code path.
    """
    dev = run_out(["losetup", "--find", "--show", "--partscan", str(img_path)])

    # Belt + braces: nudge the kernel to enumerate the partitions even
    # if --partscan silently didn't. partprobe returns 0 whether or not
    # the kernel created nodes, so this is cheap.
    run(["partprobe", dev], check=False)

    # Wait up to 3s for the partition nodes. If they still aren't
    # there, fall back to a second-loop-per-partition layout using
    # --offset/--sizelimit (no partscan needed at all).
    for _ in range(30):
        if Path(f"{dev}p1").exists() and Path(f"{dev}p2").exists():
            return dev
        time.sleep(0.1)

    # Fallback: detach and re-attach with per-partition loop devices.
    log("partscan+partprobe didn't expose partitions; using offset-loops")
    run(["losetup", "-d", dev], check=False)
    return _losetup_offset_pair(img_path)


def _losetup_offset_pair(img_path: Path) -> str:
    """Create two loop devices, one per partition, by reading the MBR
    and passing --offset/--sizelimit to losetup. Returns a synthetic
    prefix such that `<prefix>p1` and `<prefix>p2` resolve to the
    per-partition loop devices. The prefix is the directory where we
    symlinked them; this keeps the rest of the pipeline's
    f"{loop}p1" / f"{loop}p2" idiom working unchanged.
    """
    # Read the four 16-byte MBR partition entries at offset 0x1BE.
    with img_path.open("rb") as f:
        f.seek(0x1BE)
        mbr = f.read(64)
    # Each entry: 8 bytes of flags + 4 bytes start LBA + 4 bytes sector count
    # (little-endian). We only use entries 1 and 2.
    def _entry(idx: int) -> tuple[int, int]:
        off = idx * 16
        start = int.from_bytes(mbr[off + 8:off + 12], "little")
        count = int.from_bytes(mbr[off + 12:off + 16], "little")
        return start * 512, count * 512

    p1_off, p1_sz = _entry(0)
    p2_off, p2_sz = _entry(1)
    if p1_sz == 0 or p2_sz == 0:
        die(f"MBR doesn't describe two partitions: p1={p1_sz} p2={p2_sz}")

    loop1 = run_out([
        "losetup", "--find", "--show",
        f"--offset={p1_off}", f"--sizelimit={p1_sz}", str(img_path),
    ])
    loop2 = run_out([
        "losetup", "--find", "--show",
        f"--offset={p2_off}", f"--sizelimit={p2_sz}", str(img_path),
    ])

    # Expose as /tmp/pi5-loopXXXXXXp1 / p2 so the caller's f-string
    # concatenation still works. Symlink targets point at the two
    # real loop devices.
    prefix_dir = Path(tempfile.mkdtemp(prefix="pi5-loop-"))
    prefix = str(prefix_dir / "loop")
    Path(f"{prefix}p1").symlink_to(loop1)
    Path(f"{prefix}p2").symlink_to(loop2)
    # Stash real loop-device names on the prefix directory so detach
    # can find them without re-parsing the MBR.
    (prefix_dir / ".loops").write_text(f"{loop1}\n{loop2}\n")
    log(f"offset-loops: p1={loop1} p2={loop2}")
    return prefix


def losetup_detach(dev: str) -> None:
    """Detach whatever `losetup_attach` returned. Handles both the
    partscan-happy path (`dev` is a /dev/loopN) and the fallback
    offset-pair path (`dev` is a symlink-prefix with a sibling
    `.loops` manifest naming the two real loop devices)."""
    # Fallback path: prefix_dir contains a .loops manifest.
    loops_file = Path(dev).parent / ".loops"
    if loops_file.exists():
        for loop in loops_file.read_text().split():
            run(["losetup", "-d", loop], check=False)
        # Clean up the symlink directory too.
        shutil.rmtree(Path(dev).parent, ignore_errors=True)
        return
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


def read_mbr_partuuid(img_path: Path, partnum: int) -> str:
    """Compute the PARTUUID of an MBR partition from the image directly.

    For MBR disks the kernel synthesises PARTUUID as
    `<disk-signature>-<partition-number>`, where disk-signature is the
    4-byte little-endian value at offset 0x1B8 of the MBR, formatted
    as 8 lowercase hex digits; partition-number is the 1-based index
    formatted as 2 hex digits. Reading the image ourselves dodges two
    problems: per-partition loop devices (the fallback path) don't
    expose PARTUUID via blkid, and some blkid versions lag behind
    recent kernel changes to PARTUUID formatting.
    """
    with img_path.open("rb") as f:
        f.seek(0x1B8)
        sig = f.read(4)
    if len(sig) != 4:
        die(f"short read on MBR disk signature in {img_path}")
    return f"{int.from_bytes(sig, 'little'):08x}-{partnum:02x}"


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


def jpkg_install(root: Path, packages: Iterable[str], release_tag: str) -> None:
    pkgs = list(packages)
    if not pkgs:
        log("(no packages requested)")
        return

    # merged-usr: jonerix ships /usr as a symlink to / so every path
    # that references /usr/... resolves to the same flat tree jpkg
    # writes to (/bin, /lib, /include, /share). Python 3.14 was
    # built with --prefix=/usr; at runtime it resolves sys.prefix
    # via /proc/self/exe + landmark discovery and expects its stdlib
    # at /usr/lib/python3.14/. Without the symlink the chrooted
    # `python3 -m ensurepip` post_install hook dies with:
    #   "Fatal Python error: Failed to import encodings module"
    # Reproduced in CI 2026-04-20 run 24649369096. Create it before
    # any jpkg install so /usr resolves correctly from the very first
    # hook run. Safe to re-create (symlink target is stable).
    usr_link = root / "usr"
    if not usr_link.is_symlink():
        # If something installed earlier created it as a dir, move
        # its contents into the flat tree; then replace with symlink.
        if usr_link.is_dir():
            for child in usr_link.iterdir():
                dest = root / child.name
                if dest.exists():
                    # Merge directories; leave conflicting files alone.
                    if dest.is_dir() and child.is_dir():
                        for sub in child.rglob("*"):
                            rel = sub.relative_to(child)
                            target = dest / rel
                            if not target.exists():
                                target.parent.mkdir(parents=True, exist_ok=True)
                                shutil.move(str(sub), str(target))
                else:
                    shutil.move(str(child), str(dest))
            usr_link.rmdir()
        # Use a relative symlink ('.') so mounts under $root don't
        # escape to the host when the chroot resolves /usr.
        usr_link.symlink_to(".")
    log(f"merged-usr: {usr_link} -> .")

    # `--root <path>` redirects jpkg's entire worldview into <path>,
    # including its cache + index + db. So even if we've called
    # `jpkg update` on the host, the rooted install still needs its
    # own `jpkg --root <path> update` first to populate
    # <path>/var/cache/jpkg/INDEX and the keys dir. Before the update
    # can succeed we need:
    #  - /etc/jpkg/repos.conf pointing at the release mirror
    #  - /etc/jpkg/keys/jonerix.pub to verify INDEX signatures
    # ...both cloned from the host's jpkg config (CI shipped a ready
    # jpkg layout in /etc/jpkg).
    staging_jpkg = root / "etc" / "jpkg"
    (staging_jpkg / "keys").mkdir(parents=True, exist_ok=True)

    # Pin the staging mirror to the release tag for the duration of the
    # install. This is what makes the artifact reproducible: jpkg
    # resolves every package against the v<VERSION_ID> release's pinned
    # INDEX, not the rolling `packages` release that floats forward.
    # The post-install rewrite below switches it to rolling so the
    # booted Pi tracks main going forward (same model as Debian:
    # install from a release snapshot, then `apt update` follows
    # whatever channel the system is configured for).
    pinned_repo_conf = (
        f"# Generated by build-image.py — pinned to release tag {release_tag}\n"
        f"# for a reproducible install. Rewritten to the rolling mirror\n"
        f"# below before the image is sealed.\n"
        f"[repo]\n"
        f'url = "{RELEASE_BASE_URL}/{release_tag}"\n'
        # signature_policy=warn — jpkg 2.2.0+ defaults to require, but
        # the rolling release ships unsigned .jpkgs (only INDEX.zst.sig
        # exists; per-package sigs missing as of 2026-05-09). Opt back
        # to warn so unsigned packages install with a log line instead
        # of erroring out.
        f'signature_policy = "warn"\n'
    )
    (staging_jpkg / "repos.conf").write_text(pinned_repo_conf)
    log(f"staging jpkg repos.conf -> {RELEASE_BASE_URL}/{release_tag} (pinned)")

    # Trust keys: copy from host so INDEX signatures verify.
    host_jpkg = Path("/etc/jpkg")
    if (host_jpkg / "keys").is_dir():
        for k in (host_jpkg / "keys").iterdir():
            dst = staging_jpkg / "keys" / k.name
            if not dst.exists():
                shutil.copy(k, dst)

    log(f"jpkg update -r {root}")
    run(["jpkg", "--root", str(root), "update"])

    if "toybox" in pkgs:
        # Several later packages replace toybox applet links in post_install
        # hooks (/bin/sh, /bin/login, /bin/reboot). Install toybox first so
        # those hooks have cat/sed/ln/cp available and toybox cannot clobber
        # their final links later in the same install transaction.
        log(f"jpkg install -r {root} toybox (bootstrap)")
        run(["jpkg", "--root", str(root), "install", "toybox"])

    log(f"jpkg install -r {root} {' '.join(pkgs)}")
    # Per packages/jpkg/src/main.c line 81 ("-r, --root <path> Use alternative
    # root filesystem"), --root is a top-level flag BEFORE the subcommand.
    run(["jpkg", "--root", str(root), "install"] + pkgs)

    # Switch the booted system back to the rolling mirror so future
    # `jpkg update` / `jpkg upgrade` follow main rather than staying
    # frozen on the release snapshot. Users who want the host pinned
    # forever can `jpkg conform <ver>` post-boot.
    rolling_repo_conf = (
        f"# Default jonerix package mirror. Tracks the rolling `{ROLLING_TAG}` release\n"
        f"# at github.com/stormj-UH/jonerix/releases. To pin this host to a specific\n"
        f"# release version (e.g. v1.1.7), run: jpkg conform 1.1.7\n"
        f"[repo]\n"
        f'url = "{RELEASE_BASE_URL}/{ROLLING_TAG}"\n'
        # See pinned_repo_conf above. Drop this line once per-package
        # signatures are restored to the rolling mirror.
        f'signature_policy = "warn"\n'
    )
    (staging_jpkg / "repos.conf").write_text(rolling_repo_conf)
    log(f"sealed jpkg repos.conf -> {RELEASE_BASE_URL}/{ROLLING_TAG} (rolling)")


# ----------------------------------------------------------------------------
# Firmware handling
# ----------------------------------------------------------------------------


def fetch_firmware(dest_tarball: Path, url: str = FIRMWARE_TARBALL_URL) -> None:
    """Download the raspberrypi/firmware tarball if not already cached,
    validating any existing cache file before trusting it.

    A size > 0 check isn't enough: a previous run that died mid-
    download leaves a partial file the next run then hands to bsdtar
    and gets 'Error opening archive'. Run a cheap `bsdtar -tf` listing
    to confirm the cache is a valid archive before reusing it; fall
    through to re-download on any failure.
    """
    if dest_tarball.exists() and dest_tarball.stat().st_size > 0:
        # Cheap integrity probe.
        probe = subprocess.run(
            ["bsdtar", "-tf", str(dest_tarball)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        if probe.returncode == 0:
            log(f"firmware tarball already cached (valid): {dest_tarball}")
            return
        log(f"firmware tarball cached but UNREADABLE; redownloading")
        dest_tarball.unlink(missing_ok=True)

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


def require_boot_firmware(boot_mnt: Path) -> None:
    """Fail fast if a supposedly self-contained image lacks Pi 5 boot files."""
    required = [
        "kernel_2712.img",
        "bcm2712-rpi-5-b.dtb",
        "start4.elf",
        "fixup4.dat",
    ]
    missing = [
        name for name in required
        if not (boot_mnt / name).is_file() or (boot_mnt / name).stat().st_size == 0
    ]
    if missing:
        die("firmware payload missing required boot files: " + ", ".join(missing))


# ----------------------------------------------------------------------------
# Rootfs customization
# ----------------------------------------------------------------------------


def write_file(path: Path, content: str, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content)
    path.chmod(mode)


def fetch_ca_bundle(root: Path) -> None:
    """Drop a current Mozilla CA bundle into the image at
    /etc/ssl/certs/ca-certificates.crt.

    jonerix doesn't yet ship a ca-certificates jpkg, but tailscale,
    curl, openntpd-with-TLS, and dropbear-with-TLS all need a trust
    store on first boot. Matches what install/wsl/build-rootfs.sh does
    (curl the Mozilla bundle directly from curl.se). Same permissive
    licence (MPL-2.0 for the bundle, curl's distribution is BSD).
    """
    certs_dir = root / "etc" / "ssl" / "certs"
    certs_dir.mkdir(parents=True, exist_ok=True)
    dest = certs_dir / "ca-certificates.crt"
    if dest.exists() and dest.stat().st_size > 0:
        log(f"ca-certificates.crt already present ({dest.stat().st_size} bytes)")
        return
    url = "https://curl.se/ca/cacert.pem"
    log(f"fetching CA bundle from {url}")
    try:
        with urllib.request.urlopen(url, timeout=60) as resp, dest.open("wb") as fp:
            shutil.copyfileobj(resp, fp)
    except Exception as e:
        die(f"failed to download CA bundle: {e}")
    log(f"wrote {dest} ({dest.stat().st_size} bytes)")


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
        "tracefs                    /sys/kernel/tracing  tracefs  nosuid,nodev,noexec,relatime  0 0\n"
        "proc                       /proc       proc    defaults                            0 0\n"
        "tmpfs                      /tmp        tmpfs   defaults,nosuid,nodev,size=20%      0 0\n"
        "tmpfs                      /run        tmpfs   defaults,nosuid,nodev,size=20%      0 0\n"
    )
    write_file(root / "etc" / "fstab", fstab)


def write_baseline_users(root: Path) -> None:
    """Bake a minimal /etc/passwd, /etc/shadow, /etc/group containing
    `root` (locked password — login is via the SSH key written by
    --ssh-key) plus the system-account placeholders openrc / dhcpcd /
    openntpd / dropbear expect to look up at runtime.

    Without this, the freshly-installed shadow + toybox packages
    leave /etc/passwd EMPTY (no recipe owns those files), so:
      - dropbear can't resolve uid 0 -> 'root' for pubkey auth
      - shadow-getty fails on every console because login() needs root
      - dockerd can't drop privileges to the docker group on start
    Result: the image boots to a kernel panic-ish dead end.
    """
    passwd = (
        "root:x:0:0:root:/root:/bin/mksh\n"
        "daemon:x:1:1:daemon:/:/bin/false\n"
        "bin:x:2:2:bin:/bin:/bin/false\n"
        "sys:x:3:3:sys:/dev:/bin/false\n"
        "nobody:x:65534:65534:nobody:/nonexistent:/bin/false\n"
        "_ntp:x:123:123:NTP daemon:/var/empty:/bin/false\n"
        "dhcpcd:x:252:252:dhcpcd:/var/lib/dhcpcd:/bin/false\n"
    )
    # `!` in the password field locks console login while still allowing
    # SSH pubkey auth (dropbear treats `!`/`*` as locked-but-account-exists).
    # 19850 = days since epoch on 2024-05-something — arbitrary; matches
    # `chage -d` output for accounts created at install time.
    shadow = (
        "root:!:19850:0:99999:7:::\n"
        "daemon:*:19850:0:99999:7:::\n"
        "bin:*:19850:0:99999:7:::\n"
        "sys:*:19850:0:99999:7:::\n"
        "nobody:*:19850:0:99999:7:::\n"
        "_ntp:*:19850:0:99999:7:::\n"
        "dhcpcd:*:19850:0:99999:7:::\n"
    )
    group = (
        "root:x:0:root\n"
        "daemon:x:1:\n"
        "bin:x:2:\n"
        "sys:x:3:\n"
        "tty:x:5:\n"
        "disk:x:6:\n"
        "wheel:x:10:\n"
        "nobody:x:65534:\n"
        "uucp:x:14:\n"
        "_ntp:x:123:\n"
        "dhcpcd:x:252:\n"
        "docker:x:998:\n"
    )
    write_file(root / "etc" / "passwd", passwd, mode=0o644)
    write_file(root / "etc" / "shadow", shadow, mode=0o640)
    write_file(root / "etc" / "group",  group,  mode=0o644)


def write_sshd_init(root: Path) -> None:
    """Drop /etc/init.d/sshd if no jpkg package shipped one. The dropbear
    package as of 2024.86-r4 ships only the binaries (/bin/dropbear,
    /bin/dropbearkey, ...) — no init script — so a freshly-installed
    image has no way to start an SSH daemon at boot.

    The init script wraps dropbear with on-demand host-key generation;
    matches the script that's been running on jonerix-tormenta since
    first bring-up. Once dropbear's recipe ships its own
    /etc/init.d/sshd this function should be a no-op (the existence
    check below makes it idempotent).
    """
    init_path = root / "etc" / "init.d" / "sshd"
    if init_path.exists():
        return
    init = (
        "#!/bin/openrc-run\n"
        "# OpenRC service script for dropbear SSH daemon\n"
        "# Generated by image/pi5/build-image.py — once the dropbear jpkg\n"
        "# package starts shipping its own init script this file is\n"
        "# replaced on package install.\n"
        "\n"
        "name=\"sshd\"\n"
        "description=\"Dropbear SSH daemon\"\n"
        "\n"
        "command=\"/bin/dropbear\"\n"
        "command_args=\"-R -p 22 -P /run/dropbear.pid ${DROPBEAR_OPTS:-}\"\n"
        "pidfile=\"/run/dropbear.pid\"\n"
        "\n"
        "extra_started_commands=\"reload\"\n"
        "\n"
        "depend() {\n"
        "    need net\n"
        "    use dns logger\n"
        "    after firewall\n"
        "}\n"
        "\n"
        "start_pre() {\n"
        "    local keydir=\"/etc/dropbear\"\n"
        "    checkpath -d -m 0700 \"$keydir\"\n"
        "    if [ ! -f \"$keydir/dropbear_ed25519_host_key\" ]; then\n"
        "        einfo \"Generating Ed25519 host key...\"\n"
        "        dropbearkey -t ed25519 -f \"$keydir/dropbear_ed25519_host_key\" || return 1\n"
        "    fi\n"
        "    if [ ! -f \"$keydir/dropbear_ecdsa_host_key\" ]; then\n"
        "        einfo \"Generating ECDSA host key...\"\n"
        "        dropbearkey -t ecdsa -s 256 -f \"$keydir/dropbear_ecdsa_host_key\" || return 1\n"
        "    fi\n"
        "    checkpath -d -m 0755 /run\n"
        "}\n"
        "\n"
        "reload() {\n"
        "    ebegin \"Reloading ${name}\"\n"
        "    start-stop-daemon --signal HUP --pidfile \"$pidfile\"\n"
        "    eend $?\n"
        "}\n"
    )
    write_file(init_path, init, mode=0o755)
    # /etc/dropbear is created by start_pre on first boot, but having
    # it pre-exist with the right perms keeps the boot-trace clean.
    (root / "etc" / "dropbear").mkdir(mode=0o700, exist_ok=True)


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


def disable_openrc_service(root: Path, svc: str, runlevel: str) -> None:
    link = root / "etc" / "runlevels" / runlevel / svc
    link.unlink(missing_ok=True)


def force_symlink(root: Path, path: str, target: str) -> None:
    link = root / path.lstrip("/")
    link.parent.mkdir(parents=True, exist_ok=True)
    if link.exists() or link.is_symlink():
        if link.is_dir() and not link.is_symlink():
            die(f"cannot replace directory with symlink: /{path.lstrip('/')}")
        link.unlink()
    link.symlink_to(target)


def normalize_login_timeout(root: Path) -> None:
    login_defs = root / "etc" / "login.defs"
    if not login_defs.exists():
        return
    lines = login_defs.read_text().splitlines()
    saw = False
    out = []
    for line in lines:
        if re.match(r"^\s*LOGIN_TIMEOUT\s", line):
            out.append("LOGIN_TIMEOUT\t0")
            saw = True
        else:
            out.append(line)
    if not saw:
        out.append("LOGIN_TIMEOUT\t0")
    login_defs.write_text("\n".join(out) + "\n")


def enforce_pi5_boot_defaults(root: Path) -> None:
    """Make image-critical replacement links match the known-good Pi."""
    if (root / "bin" / "mksh").exists():
        force_symlink(root, "/bin/sh", "mksh")

    shadow_bins = (
        "passwd login chpasswd chage chfn chsh chgpasswd useradd userdel "
        "usermod groupadd groupdel groupmod gpasswd newgrp newusers nologin "
        "vipw pwck grpck pwconv pwunconv grpconv grpunconv expiry logoutd "
        "newgidmap newuidmap faillog lastlog sulogin"
    ).split()
    for name in shadow_bins:
        if (root / "bin" / f"shadow-{name}").exists():
            force_symlink(root, f"/bin/{name}", f"shadow-{name}")

    if (root / "bin" / "adduser-safe").exists():
        force_symlink(root, "/bin/adduser", "adduser-safe")
    if (root / "local" / "bin" / "reboot-openrc").exists():
        force_symlink(root, "/bin/reboot", "/usr/local/bin/reboot-openrc")

    normalize_login_timeout(root)


def validate_pi5_boot_defaults(root: Path) -> None:
    problems: list[str] = []

    def expect_link(path: str, targets: set[str]) -> None:
        link = root / path.lstrip("/")
        if not link.is_symlink():
            problems.append(f"{path} is not a symlink")
            return
        actual = os.readlink(link)
        if actual not in targets:
            problems.append(f"{path} -> {actual}, expected one of {sorted(targets)}")

    expect_link("/bin/sh", {"mksh"})
    expect_link("/bin/login", {"shadow-login"})
    expect_link("/bin/passwd", {"shadow-passwd"})
    expect_link("/bin/adduser", {"adduser-safe"})
    expect_link("/bin/reboot", {"/usr/local/bin/reboot-openrc", "/local/bin/reboot-openrc"})

    shadow_login = root / "etc" / "init.d" / "shadow-login"
    if not shadow_login.is_file() or shadow_login.stat().st_size == 0:
        problems.append("/etc/init.d/shadow-login is missing or empty")
    else:
        text = shadow_login.read_text(errors="replace")
        if "while :; do /bin/shadow-getty /dev/tty1; sleep 1; done" not in text:
            problems.append("/etc/init.d/shadow-login is not the fixed shadow-getty loop")

    disable_eee = root / "etc" / "init.d" / "disable-eee"
    if not disable_eee.is_file():
        problems.append("/etc/init.d/disable-eee is missing")
    else:
        text = disable_eee.read_text(errors="replace")
        if "after dhcpcd" not in text or "continuing boot" not in text:
            problems.append("/etc/init.d/disable-eee is not the tolerant post-dhcpcd service")

    login_defs = root / "etc" / "login.defs"
    if login_defs.exists():
        for line in login_defs.read_text(errors="replace").splitlines():
            if re.match(r"^\s*LOGIN_TIMEOUT\s", line) and line.split()[-1] != "0":
                problems.append(f"/etc/login.defs keeps nonzero {line!r}")

    if problems:
        die("Pi 5 boot defaults failed validation:\n  - " + "\n  - ".join(problems))


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

[ "$(id -u)" -eq 0 ] || die "must run as root (try: sudo $0)"

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
    """config.txt with Pi 5 essentials."""
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
    """Wire runlevels to match jonerix-tormenta's core Pi startup.

    `sshd` (the dropbear-wrapping init script) IS enabled here so that
    a flashed image with --ssh-key baked in actually accepts SSH on
    first boot — without it the only way in is the HDMI console.

    Host-specific apps (Tailscale variants, syslog forwarding
    destinations) are deliberately left out. Only enables services
    whose init scripts are present on disk.
    """
    for svc in ("devfs", "modules"):
        disable_openrc_service(root, svc, "boot")
    for svc in ("dhcpcd", "local"):
        disable_openrc_service(root, svc, "default")

    for svc in (
        "boot-trace",
        "dhcpcd",
        "disable-eee",
        "fan-control",
        "hostname",
        "hwclock",
        "localmount",
        "loopback",
        "netfilter-nft-modules",
        "pi5-cold-reboot",
        "pi5-wifi",
        "root",
        "sysctl",
    ):
        if (root / "etc" / "init.d" / svc).exists():
            enable_openrc_service(root, svc, runlevel="boot")

    # `sshd` here is the dropbear-wrapping init script (jonerix calls it
    # sshd regardless of which daemon backs it). The script is either
    # shipped by the dropbear jpkg recipe or written by write_sshd_init()
    # in build(). Without this, a flashed image has dropbear binaries on
    # disk but nothing runs them at boot — the user has to console in to
    # start SSH manually, defeating the purpose of --ssh-key.
    for svc in ("ntp-bootstrap", "ntpd", "shadow-login", "sshd", "syslogd", "wpa_supplicant_wlan0"):
        if (root / "etc" / "init.d" / svc).exists():
            enable_openrc_service(root, svc, runlevel="default")

    if (root / "etc" / "init.d" / "reboot-trace-shutdown").exists():
        enable_openrc_service(root, "reboot-trace-shutdown", runlevel="shutdown")


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

    # --packages is additive to DEFAULT_PACKAGES, matching the CLI help.
    # Prior behaviour ('or DEFAULT_PACKAGES' fallback) silently dropped
    # the entire default set the moment a user passed --packages, which
    # produced unbootable images (no musl/toybox/openrc/dropbear/sudo).
    extra = [p.strip() for p in (args.packages or "").split(",") if p.strip()]
    packages = list(dict.fromkeys(DEFAULT_PACKAGES + extra))
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
        # Compute PARTUUIDs from the image's MBR directly — works
        # identically whether the loop-device pipeline went through
        # the partscan path or the per-partition-offset fallback path
        # (the latter doesn't let blkid see a partition-table context).
        boot_partuuid = read_mbr_partuuid(out_img, 1)
        root_partuuid = read_mbr_partuuid(out_img, 2)
        log(f"boot PARTUUID: {boot_partuuid}")
        log(f"root PARTUUID: {root_partuuid}")

        # Stage 3: populate rootfs via jpkg, then boot/ via firmware tarball.
        mnt_root = Path(tempfile.mkdtemp(prefix="pi5-root-"))
        mnt_boot = Path(tempfile.mkdtemp(prefix="pi5-boot-"))
        try:
            with mount(root_part, mnt_root), mount(boot_part, mnt_boot):
                # jpkg first -- it creates /etc, /lib, etc.
                jpkg_install(mnt_root, packages, args.release_tag)
                enforce_pi5_boot_defaults(mnt_root)

                # CA trust store for TLS-using daemons (tailscale,
                # curl, ntpd). Must come AFTER jpkg_install so the
                # rootfs skeleton exists.
                fetch_ca_bundle(mnt_root)

                # Pi 5 firmware/kernel. CI passes --firmware-cache so
                # Raspi Imager artifacts are self-contained; local
                # permissive-only builds can omit both firmware options
                # and complete the boot partition later with:
                #   pi5-install.sh --firmware-only -d /dev/sdX
                if args.firmware_dir:
                    copy_local_firmware(Path(args.firmware_dir), mnt_boot)
                    log("firmware: included from --firmware-dir (override)")
                    require_boot_firmware(mnt_boot)
                elif args.firmware_cache:
                    firmware_cache = Path(args.firmware_cache)
                    fetch_firmware(firmware_cache)
                    extract_firmware_to_boot(firmware_cache, mnt_boot)
                    log("firmware: included from --firmware-cache")
                    require_boot_firmware(mnt_boot)
                else:
                    log("firmware: NOT included (run pi5-install.sh "
                        "--firmware-only after dd'ing this image)")
                    # Drop a marker in /boot so the user gets a clear
                    # error message at first boot if they forget to
                    # complete the install.
                    (mnt_boot / "FIRMWARE_MISSING").write_text(
                        "This jonerix-pi5.img was built without the\n"
                        "raspberrypi/firmware payload. To complete the\n"
                        "install:\n"
                        "\n"
                        "  sudo pi5-install.sh --firmware-only -d /dev/sdX\n"
                        "\n"
                        "(or `--firmware-dir LOCAL` at build time if you\n"
                        "want a self-contained image).\n"
                    )

                # config.txt + cmdline.txt in the boot partition.
                write_boot_config(mnt_boot)
                write_boot_cmdline(mnt_boot, root_partuuid)

                # /etc/fstab, /etc/hostname, /etc/hosts in the rootfs.
                write_fstab(mnt_root, boot_partuuid, root_partuuid)
                write_hostname(mnt_root, args.hostname)

                # /etc/passwd + /etc/shadow + /etc/group baseline. No
                # jpkg recipe owns these, so without this step the
                # image boots with an empty /etc/passwd and SSH /
                # console login both fail. Write before write_ssh_key
                # so the chmod chain (root:root on /root/.ssh/) has a
                # valid uid 0 to refer to.
                write_baseline_users(mnt_root)

                # /etc/init.d/sshd — see write_sshd_init() docstring
                # for why this lives in build-image.py rather than
                # the dropbear recipe.
                write_sshd_init(mnt_root)

                if args.ssh_key:
                    write_ssh_key(mnt_root, args.ssh_key)

                # Restricted-firmware opt-in path: drop the helper + the
                # motd/login nag. Nothing is fetched at build time; the
                # user runs jonerix-pi5-restricted after first boot and
                # explicitly accepts each license before any GPL kernel
                # module or Broadcom blob touches the filesystem.
                write_restricted_installer(mnt_root)
                write_restricted_motd(mnt_root)

                enable_default_services(mnt_root)
                validate_pi5_boot_defaults(mnt_root)

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
    p.add_argument("--ssh-key", default=None,
                   help="Public SSH key to drop into /root/.ssh/authorized_keys")
    p.add_argument("--firmware-dir", default=None,
                   help="Local directory to copy firmware from (skips download). "
                        "Must contain kernel_2712.img and the bcm2712 DTB.")
    p.add_argument("--firmware-cache", default=None,
                   help="Path to cache/download the raspberrypi/firmware "
                        "tarball and include Pi 5 boot firmware in the image.")
    p.add_argument("--release-tag", default=DEFAULT_RELEASE_TAG,
                   help=(
                       "GitHub release tag whose pinned package set to install "
                       "from. Default is computed from the source tree's VERSION_ID "
                       f"(currently {DEFAULT_RELEASE_TAG!r}). Pass 'packages' to "
                       "build against the rolling mirror, or 'v1.1.5' / etc. to "
                       "build a previous release's image. The booted Pi is always "
                       "left pointing at the rolling mirror — pinning is install-"
                       "time only, for reproducibility."
                   ))
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
