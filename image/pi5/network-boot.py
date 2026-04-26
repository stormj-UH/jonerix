#!/usr/bin/env python3
# network-boot.py — build a network-boot (TFTP) payload tree for the
# Raspberry Pi 5, suitable for dropping behind dnsmasq / in.tftpd /
# any RFC-1350 TFTP server.
#
# The Pi 5 bootloader (the EEPROM on the RP1 side of the board —
# no bootcode.bin is involved on Pi 5) can fetch its firmware +
# kernel + device tree over TFTP when the EEPROM config has
# BOOT_ORDER containing `0x2` and the unit is in a position in
# that order. This script does not mutate the Pi's EEPROM — it
# only prepares the payload the TFTP server needs to hand out.
#
# Output tree (default --output=./tftp):
#   tftp/
#     <serial>/                    # one per Pi, populated in CI
#       config.txt                 # configurable via --uart / --cmdline
#       cmdline.txt
#       kernel_2712.img
#       bcm2712-rpi-5-b.dtb
#       bcm2712-d-rpi-5-b.dtb
#       bcm2712-rpi-cm5-*.dtb
#       start4.elf
#       start4db.elf
#       start4cd.elf
#       start4x.elf
#       fixup4.dat
#       fixup4cd.dat
#       fixup4db.dat
#       fixup4x.dat
#       LICENCE.broadcom
#       LICENSES-ACCEPTED.txt
#       overlays/*.dtbo
#     dnsmasq.conf.sample          # drop-in config for the TFTP server
#     README.md                    # "how to actually boot a Pi 5 with this"
#
# POSIX-safe: Python 3.9+, stdlib only (urllib, tarfile, hashlib,
# argparse, pathlib, shutil, json, datetime). Designed to run both on
# a jonerix builder and on a stock macOS / Ubuntu CI runner —
# everything the script does is filesystem-level, no jpkg / losetup /
# privileged mounts.
#
# Usage:
#   network-boot.py --output tftp --serial 10000000abcd
#   network-boot.py --output tftp --from-firmware-tarball ./firmware.tar.gz
#   network-boot.py --yes         # skip the kernel+firmware license gate
#
# License: MIT — part of jonerix.

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request


DEFAULT_FIRMWARE_TAG = "stable"
FIRMWARE_URL = (
    "https://github.com/raspberrypi/firmware/archive/refs/heads/"
    f"{DEFAULT_FIRMWARE_TAG}.tar.gz"
)

# Files we want from the raspberrypi/firmware tarball's `boot/` dir.
# The Pi 5 bootloader fetches these verbatim over TFTP when the unit
# is serial-dir gated (per `tftp_prefix` in its config).
PI5_FILES = [
    "kernel_2712.img",
    "bcm2712-rpi-5-b.dtb",
    "bcm2712-d-rpi-5-b.dtb",
    "bcm2712-rpi-cm5-cm4io.dtb",
    "bcm2712-rpi-cm5-cm5io.dtb",
    "bcm2712-rpi-cm5l-cm4io.dtb",
    "bcm2712-rpi-cm5l-cm5io.dtb",
    "bcm2712-rpi-500.dtb",
    "start4.elf",
    "start4cd.elf",
    "start4db.elf",
    "start4x.elf",
    "fixup4.dat",
    "fixup4cd.dat",
    "fixup4db.dat",
    "fixup4x.dat",
    "LICENCE.broadcom",
]

# Overlay blobs the Pi 5 commonly asks for at boot. Missing entries
# are fine (some depend on hats that aren't attached); present ones
# are copied verbatim.
COMMON_OVERLAYS = [
    "disable-bt.dtbo",
    "disable-wifi.dtbo",
    "miniuart-bt.dtbo",
    "pi3-miniuart-bt.dtbo",
    "vc4-kms-v3d.dtbo",
    "vc4-kms-v3d-pi5.dtbo",
]


# ─────────────────────────────────────────────────────────────────
# Logging + argparse
# ─────────────────────────────────────────────────────────────────

def info(msg: str) -> None:
    print(f"==> {msg}", flush=True)


def warn(msg: str) -> None:
    print(f"!!  {msg}", file=sys.stderr, flush=True)


def die(msg: str, code: int = 1) -> None:
    print(f"error: {msg}", file=sys.stderr, flush=True)
    sys.exit(code)


def parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(
        description="Build a Pi 5 TFTP network-boot payload.",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "Example (CI, fresh runner):\n"
            "  network-boot.py --output out/tftp \\\n"
            "      --serial 10000000deadbeef \\\n"
            "      --root-source nfs --nfs-server 10.0.0.4 --nfs-path /srv/jonerix \\\n"
            "      --yes\n"
        ),
    )
    ap.add_argument(
        "--output", "-o", default="tftp",
        help="Output directory for the TFTP tree (default: ./tftp)",
    )
    ap.add_argument(
        "--serial", default="",
        help=(
            "Board serial (64-bit hex, lower-case). The Pi 5 requests"
            " files from <serial>/. Use 'any' to drop files at the"
            " root (serves every Pi on the wire)."
        ),
    )
    ap.add_argument(
        "--from-firmware-tarball", default=None,
        help=(
            "Path to an already-downloaded raspberrypi/firmware tarball"
            " (.tar.gz). Skips the network fetch — useful for offline"
            " CI and for reproducibility."
        ),
    )
    ap.add_argument(
        "--firmware-tag", default=DEFAULT_FIRMWARE_TAG,
        help="raspberrypi/firmware branch/tag to download (default: stable)",
    )
    ap.add_argument(
        "--uart", action=argparse.BooleanOptionalAction, default=True,
        help="Enable UART serial console (default: on; --no-uart to disable)",
    )
    ap.add_argument(
        "--root-source", choices=("nfs", "http", "local", "none"),
        default="nfs",
        help=(
            "How the booted kernel mounts root. `nfs` (default) writes"
            " cmdline.txt with root=/dev/nfs and nfsroot=. `http` writes"
            " a cmdline for an initramfs that expects an HTTP-served"
            " rootfs. `local` leaves root= pointing at a block dev the"
            " caller sets via --root-device. `none` writes no root=."
        ),
    )
    ap.add_argument("--nfs-server", default="", help="NFS server IP (with --root-source nfs)")
    ap.add_argument("--nfs-path", default="", help="NFS exported path (with --root-source nfs)")
    ap.add_argument("--root-device", default="", help="Block device (with --root-source local)")
    ap.add_argument("--hostname", default="jonerix-netboot", help="Initial hostname")
    ap.add_argument(
        "--dnsmasq-sample", action=argparse.BooleanOptionalAction, default=True,
        help="Emit a dnsmasq.conf.sample next to the TFTP tree (default: on)",
    )
    ap.add_argument(
        "--yes", "-y", action="store_true",
        help=(
            "Accept the Linux kernel GPL-2.0 and Broadcom firmware"
            " licenses non-interactively. Required for unattended CI."
        ),
    )
    ap.add_argument(
        "--release-tag", default="",
        help=(
            "GitHub release tag the netboot payload pins to (e.g. v1.1.6)."
            " The booted Pi runs `jpkg conform <ver>` against this tag if"
            " the rootfs is later configured for it. Default: read"
            " VERSION_ID from config/defaults/etc/os-release."
        ),
    )
    ap.add_argument(
        "--include-firmware", action="store_true",
        help=(
            "Bake the raspberrypi/firmware payload (kernel_2712.img, DTBs,"
            " start4.elf, fixup4.dat — GPL-2.0 + Broadcom Redistributable)"
            " into the output tarball. Default OFF: jonerix's permissive-"
            " license policy keeps non-permissive bits OUT of CI artifacts."
            " The companion install/jonerix-pi5-netboot.sh server bootstrap"
            " fetches them on the user's machine at runtime with explicit"
            " license acceptance. Use --include-firmware only for air-"
            " gapped scenarios where you can't pull from raspberrypi/"
            " firmware at deploy time."
        ),
    )
    return ap.parse_args()


# ─────────────────────────────────────────────────────────────────
# License gate — mirrors install/pi5-install.sh
# ─────────────────────────────────────────────────────────────────

LICENSE_NOTICE = """
------------------------------------------------------------------------
The Pi 5 firmware tarball from raspberrypi/firmware contains two
categories of third-party software, each under its own license:

  1. Linux kernel (kernel_2712.img, device-tree blobs)
     License: GNU General Public License v2.0
     Source:  https://github.com/raspberrypi/linux

  2. VideoCore / Broadcom firmware blobs (start4.elf, fixup4.dat, etc.)
     License: proprietary Broadcom binary — see LICENCE.broadcom in
              the tarball. Free to redistribute with Raspberry Pi
              hardware; may NOT be modified or used outside Pi boards.
     Source:  closed-source

Building this payload means you have reviewed and accept BOTH licenses.
"""


def license_gate(accept: bool) -> None:
    print(LICENSE_NOTICE, file=sys.stderr)
    if accept:
        info("License gate auto-accepted via --yes")
        return
    resp = input("Accept GPL-2.0 (kernel) and Broadcom firmware licenses? [y/N]: ")
    if resp.strip().lower() not in ("y", "yes"):
        die("declined firmware / kernel license — aborting")


# ─────────────────────────────────────────────────────────────────
# Firmware fetch + extract
# ─────────────────────────────────────────────────────────────────

def fetch_firmware(args: argparse.Namespace, work: pathlib.Path) -> pathlib.Path:
    if args.from_firmware_tarball:
        src = pathlib.Path(args.from_firmware_tarball).resolve()
        if not src.is_file():
            die(f"--from-firmware-tarball not found: {src}")
        info(f"Using provided firmware tarball: {src}")
        return src
    tgz = work / "firmware.tar.gz"
    url = (
        "https://github.com/raspberrypi/firmware/archive/refs/heads/"
        f"{args.firmware_tag}.tar.gz"
    )
    info(f"Fetching {url}")
    with urllib.request.urlopen(url) as r, open(tgz, "wb") as w:
        shutil.copyfileobj(r, w)
    sha = hashlib.sha256(tgz.read_bytes()).hexdigest()
    info(f"Downloaded {tgz.stat().st_size:,} bytes (sha256={sha})")
    return tgz


def extract_boot_dir(tgz: pathlib.Path, work: pathlib.Path) -> pathlib.Path:
    info(f"Extracting boot/ from {tgz.name}")
    # We only want the `boot/` subtree from the tarball. extractall
    # with a filter saves ~1.2 GiB of kernel sources + docs we don't
    # need. Python 3.12+ deprecated the default filter; request the
    # data filter explicitly when available.
    extract_dir = work / "fw-extract"
    extract_dir.mkdir(exist_ok=True)
    with tarfile.open(tgz, "r:gz") as tf:
        def want(m: tarfile.TarInfo) -> bool:
            parts = m.name.split("/", 2)
            return len(parts) >= 2 and parts[1] == "boot"
        members = [m for m in tf.getmembers() if want(m)]
        if not members:
            die("firmware tarball missing boot/")
        try:
            tf.extractall(extract_dir, members=members, filter="data")
        except TypeError:
            tf.extractall(extract_dir, members=members)
    # Find the boot/ inside the extracted tree.
    for root, dirs, _files in os.walk(extract_dir):
        if "boot" in dirs:
            return pathlib.Path(root) / "boot"
    die("boot/ not found in extracted tarball")


# ─────────────────────────────────────────────────────────────────
# Payload assembly
# ─────────────────────────────────────────────────────────────────

def valid_serial(s: str) -> bool:
    # Pi 5 serial on a TFTP path is 16 hex chars, lower-case
    # (the bootloader calls it the Unique ID).
    return bool(re.fullmatch(r"[0-9a-f]{16}", s))


def build_cmdline(args: argparse.Namespace) -> str:
    # Keep the same reboot=c + console defaults as pi5-install.sh
    # so warm-reboot doesn't hang once the netbooted kernel runs.
    parts = ["reboot=c"]
    if args.uart:
        parts.append("console=serial0,115200")
    parts.append("console=tty1")
    if args.root_source == "nfs":
        if not args.nfs_server or not args.nfs_path:
            die("--root-source nfs needs --nfs-server and --nfs-path")
        parts.extend([
            "root=/dev/nfs",
            f"nfsroot={args.nfs_server}:{args.nfs_path},vers=4,tcp",
            "ip=dhcp",
            "rootwait",
        ])
    elif args.root_source == "http":
        parts.extend([
            # Many netboot initramfs tools (pxelinux, u-boot)
            # look for a `netboot_url=` that they hand to wget.
            # Leave root= off so the initramfs is in charge.
            "ip=dhcp",
            "boot=netboot",
        ])
    elif args.root_source == "local":
        if not args.root_device:
            die("--root-source local needs --root-device")
        parts.extend([f"root={args.root_device}", "rootfstype=ext4", "rootwait"])
    parts.extend(["rw", "init=/bin/openrc-init", "loglevel=3", "quiet"])
    return " ".join(parts) + "\n"


def build_config(args: argparse.Namespace) -> str:
    lines = [
        "# jonerix — Raspberry Pi 5 netboot",
        "# Generated by image/pi5/network-boot.py",
        "",
        "arm_64bit=1",
        "kernel=kernel_2712.img",
        "gpu_mem=16",
        "disable_splash=1",
        "dtparam=audio=off",
        "hdmi_force_hotplug:0=1",
        "hdmi_force_hotplug:1=1",
    ]
    if args.uart:
        lines.insert(4, "enable_uart=1")
    return "\n".join(lines) + "\n"


def build_dnsmasq_sample(args: argparse.Namespace) -> str:
    # A drop-in fragment that pairs with `dnsmasq --dhcp-range=...`.
    # The Pi 5 bootloader sends vendor class "PXEClient:Arch:00000"
    # for legacy and "PXEClient:Arch:00011:UNDI:003000" for AArch64;
    # we match the latter so we don't poison x86 PXE on the same
    # network.
    tftp_root = os.path.abspath(args.output)
    return (
        "# /etc/dnsmasq.d/pi5-tftp.conf — drop-in sample generated by\n"
        "# image/pi5/network-boot.py. Edit to taste.\n\n"
        "enable-tftp\n"
        f"tftp-root={tftp_root}\n"
        "# Pi 5 UEFI-arch identifier. If you also serve x86 PXE,\n"
        "# leave this filter in so the Pi-only payload stays Pi-only.\n"
        "dhcp-match=set:pi5,option:client-arch,11\n"
        "dhcp-boot=tag:pi5,\n"
        "# Faster option-66/67 path for boards that skip class filtering.\n"
        "# dhcp-option=66,tftp.example.com\n"
    )


def build_readme(args: argparse.Namespace, fw_ref: dict) -> str:
    ts = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    return (
        "# jonerix Pi 5 network-boot payload\n\n"
        f"Generated {ts} by image/pi5/network-boot.py.\n\n"
        f"- Firmware tag: `{fw_ref['tag']}`\n"
        f"- Firmware sha256: `{fw_ref['sha256']}`\n"
        f"- Serial dir: `{args.serial or '(root)'}`\n"
        f"- Root source: `{args.root_source}`\n\n"
        "## Serving it\n\n"
        "Any RFC-1350 TFTP server works. dnsmasq is the shortest path:\n\n"
        "```sh\n"
        "sudo cp dnsmasq.conf.sample /etc/dnsmasq.d/pi5-tftp.conf\n"
        "sudo systemctl reload dnsmasq   # or: sudo rc-service dnsmasq restart\n"
        "```\n\n"
        "Then on the Pi, set the EEPROM BOOT_ORDER to include the\n"
        "`NETWORK` byte (`0x2`). From an already-booted jonerix image:\n\n"
        "```sh\n"
        "sudo pi5-wake-on-power enable    # ensure the board auto-resumes\n"
        "# rpi-eeprom-config equivalent via vcmailbox (config change\n"
        "# persists in EEPROM across reboots):\n"
        "printf '[all]\\nBOOT_ORDER=0xf241\\n' | sudo tee /tmp/boot.conf\n"
        "# upload /tmp/boot.conf as EEPROM config (TODO: anvil or\n"
        "# upstream rpi-eeprom-config; both unavailable in jonerix\n"
        "# today — file against jonerix issue tracker).\n"
        "```\n\n"
        "## Licenses\n\n"
        "See LICENSES-ACCEPTED.txt next to this file. Copied verbatim:\n"
        "- GPL-2.0 (Linux kernel)\n"
        "- Broadcom proprietary binary firmware (LICENCE.broadcom)\n"
    )


def copy_boot_tree(boot: pathlib.Path, dest: pathlib.Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    for name in PI5_FILES:
        src = boot / name
        if src.is_file():
            shutil.copy2(src, dest / name)
        else:
            warn(f"missing from tarball: {name}")
    overlays_src = boot / "overlays"
    overlays_dst = dest / "overlays"
    if overlays_src.is_dir():
        overlays_dst.mkdir(exist_ok=True)
        for name in COMMON_OVERLAYS:
            src = overlays_src / name
            if src.is_file():
                shutil.copy2(src, overlays_dst / name)
        # Also copy README and a handful of Pi-5-specific overlays if
        # present — we don't know exactly which ones are relevant per
        # hat config, so ship them all.
        for src in overlays_src.iterdir():
            if src.is_file() and src.suffix in (".dtbo", ".dtbo.gz"):
                dst = overlays_dst / src.name
                if not dst.exists():
                    shutil.copy2(src, dst)


def _resolve_release_tag(tag: str) -> str:
    """Default --release-tag to v<VERSION_ID> from os-release if unset.

    Reads `config/defaults/etc/os-release` relative to the repo root
    (this file lives at image/pi5/network-boot.py, so two parents up).
    Mirrors build-image.py's _default_release_tag logic for symmetry.
    """
    if tag:
        return tag
    try:
        repo_root = pathlib.Path(__file__).resolve().parents[2]
        osr = (repo_root / "config" / "defaults" / "etc" / "os-release").read_text()
        for line in osr.splitlines():
            if line.startswith("VERSION_ID="):
                return "v" + line.split("=", 1)[1].strip().strip('"')
    except Exception:
        pass
    return "packages"  # rolling fallback


def main() -> None:
    args = parse_args()

    if args.serial and args.serial != "any" and not valid_serial(args.serial):
        die(f"--serial must be 16 hex chars or 'any' (got {args.serial!r})")

    args.release_tag = _resolve_release_tag(args.release_tag)
    info(f"Pinning netboot payload to jonerix release: {args.release_tag}")

    out = pathlib.Path(args.output).resolve()
    out.mkdir(parents=True, exist_ok=True)

    # License gate only fires when the build will actually pull
    # raspberrypi/firmware. With --include-firmware off (the default),
    # we never touch GPL/Broadcom-licensed bits — the install-side
    # bootstrap (install/jonerix-pi5-netboot.sh) handles license
    # acceptance + download at deploy time.
    if args.include_firmware:
        license_gate(args.yes)

    # Work dir for fetch / extract. Cleaned up automatically.
    with tempfile.TemporaryDirectory(prefix="pi5-netboot-") as tmp:
        work = pathlib.Path(tmp)

        # Target subdir (serial or 'any' → root of tftp)
        payload = out / args.serial if args.serial and args.serial != "any" else out
        info(f"Assembling payload under {payload}")
        payload.mkdir(parents=True, exist_ok=True)

        if args.include_firmware:
            tgz = fetch_firmware(args, work)
            fw_sha = hashlib.sha256(tgz.read_bytes()).hexdigest()
            boot = extract_boot_dir(tgz, work)
            copy_boot_tree(boot, payload)
            info("firmware: included (--include-firmware)")
        else:
            fw_sha = ""
            (payload / "FIRMWARE_MISSING").write_text(
                "This jonerix Pi 5 netboot payload was built without the\n"
                "raspberrypi/firmware tree (Linux kernel + Broadcom blobs).\n"
                "jonerix's userland-only policy keeps non-permissive bits\n"
                "OUT of CI artifacts.\n"
                "\n"
                "To complete the netboot tree, run install/jonerix-pi5-\n"
                "netboot.sh on your Mac or Linux host: it downloads the\n"
                "firmware tarball directly from raspberrypi/firmware (with\n"
                "explicit GPL-2.0 + Broadcom Redistributable license\n"
                "acceptance) and stages it next to these files before\n"
                "starting the TFTP/HTTP server the Pi boots from.\n"
            )
            info("firmware: NOT included (companion script downloads at deploy)")

        (payload / "cmdline.txt").write_text(build_cmdline(args))
        (payload / "config.txt").write_text(build_config(args))

        (payload / "LICENSES-ACCEPTED.txt").write_text(
            f"TFTP payload built {datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')}"
            f" by image/pi5/network-boot.py.\n"
            "Kernel license: GPL-2.0 (raspberrypi/linux).\n"
            "Firmware license: see LICENCE.broadcom in this directory.\n"
        )

        # Top-level helpers
        if args.dnsmasq_sample:
            (out / "dnsmasq.conf.sample").write_text(build_dnsmasq_sample(args))
        fw_ref = {"tag": args.firmware_tag, "sha256": fw_sha}
        (out / "README.md").write_text(build_readme(args, fw_ref))
        (out / "build-info.json").write_text(json.dumps({
            "generator": "image/pi5/network-boot.py",
            "generated_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "firmware": fw_ref,
            "serial": args.serial,
            "hostname": args.hostname,
            "root_source": args.root_source,
            "release_tag": args.release_tag,
        }, indent=2) + "\n")

    info(f"Done — payload at {out}")
    if args.serial and args.serial != "any":
        info(f"Pi with unique-id {args.serial} will fetch from {out}/{args.serial}/")
    else:
        info(f"Serving all Pis from {out}/ (no serial filter)")


if __name__ == "__main__":
    main()
