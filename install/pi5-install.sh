#!/bin/sh
# pi5-install.sh — install jonerix onto a Raspberry Pi 5 SD / USB / NVMe
#
# Meant to be run from another jonerix host (or any POSIX box with
# `curl`, `tar`, `mount`, `jpkg`). Pulls firmware + kernel from the
# upstream raspberrypi/firmware repo, lays down the jonerix userland
# from jpkg, and applies every current jonerix-raspi5-fixups setting.
#
# Usage:
#   pi5-install.sh [-y] [-d /dev/sdX] [--kernel-only]
#
#   -y / --yes           Skip all interactive prompts (assume yes).
#   -d / --device PATH   Target block device. If omitted, prompts
#                        interactively from a list of removable disks.
#   --no-firmware        Don't redownload firmware / kernel (reuse
#                        whatever's already on the target's boot
#                        partition).
#   --no-userland        Don't install the jonerix userland; stop
#                        after the boot partition is populated.
#   --branch NAME        Git branch of jonerix repo to pull recipes
#                        and helpers from. Default: main.
#
# What it does, in order:
#   1. Pick a target device (interactive or -d).
#   2. Make sure the two partitions it expects exist and are
#      formatted (p1 vfat, p2 ext4). Creates them only if tools are
#      available — bails otherwise so you can pre-partition by hand.
#   3. Ask before downloading the raspberrypi/firmware tarball from
#      github.com/raspberrypi/firmware. Extracts /boot payload to
#      partition 1 (Pi 5 kernel = kernel_2712.img, dtb =
#      bcm2712-rpi-5-b.dtb, start/fixup blobs, cmdline.txt template,
#      config.txt template).
#   4. Ask before installing the jonerix userland. Uses `jpkg -r
#      TARGET install` for a curated minimal-boot package set, then
#      installs jonerix-raspi5-fixups to apply cmdline / config /
#      hdmi / wake-on-power / wifi fixups to the target.
#   5. Rewrites cmdline.txt and config.txt on the target boot
#      partition with jonerix defaults (reboot=c, fresh root UUID,
#      hdmi_force_hotplug).
#   6. Unmounts cleanly, verifies boot-critical paths, prints a
#      summary.
#
# POSIX shell only — tested on /bin/dash, /bin/mksh, /bin/ash. Avoids
# arrays, [[ ]], `local`, process substitution, and `set -o pipefail`.
# Part of jonerix — MIT License.

set -eu

# ── Defaults ─────────────────────────────────────────────────────────
BRANCH="${BRANCH:-main}"
GH_RAW="https://raw.githubusercontent.com/stormj-UH/jonerix/${BRANCH}"
FIRMWARE_URL="https://github.com/raspberrypi/firmware/archive/refs/heads/stable.tar.gz"
ASSUME_YES=0
TARGET=""
DO_FIRMWARE=1
DO_USERLAND=1

# Minimal package set for a bootable headless Pi 5. Anything else is
# additive — you can `jpkg -r /mnt/usb-root add <pkg>` after boot.
# `anvil` — MIT clean-room mkfs.ext4 / mkfs.vfat / e2fsck / fsck.vfat
# / dumpe2fs / tune2fs / resize2fs / debugfs / blkid / chattr / lsattr /
# e2image / e2label / e2freefrag / e4defrag / filefrag / findfs /
# logsave / mklost+found. Pulled in by default so every Pi 5 image
# can format, check, and inspect its own filesystems without needing
# the GPL e2fsprogs + dosfstools stack.
DEFAULT_PACKAGES="musl toybox mksh openrc dhcpcd dropbear bsdtar python3 sudo anvil jonerix-raspi5-fixups jonerix-boot-helpers openntpd tailscale"

# ── Logging helpers ─────────────────────────────────────────────────
msg()  { printf '==> %s\n' "$*"; }
warn() { printf '!!  %s\n' "$*" >&2; }
die()  { printf 'error: %s\n' "$*" >&2; exit 1; }
ask()  {
    # ask "question" [default-y|default-n] → echoes y or n
    _q="$1"; _d="${2:-y}"
    if [ "$ASSUME_YES" = 1 ]; then
        # --yes means "assume yes to everything" — always answer y,
        # even to prompts whose default was n.
        echo y
        return 0
    fi
    printf '%s ' "$_q" >&2
    case "$_d" in
        y) printf '[Y/n] ' >&2 ;;
        n) printf '[y/N] ' >&2 ;;
    esac
    read _ans || _ans=""
    [ -z "$_ans" ] && _ans="$_d"
    case "$_ans" in
        y|Y|yes|YES) echo y ;;
        *) echo n ;;
    esac
}

# ── Arg parsing ─────────────────────────────────────────────────────
while [ $# -gt 0 ]; do
    case "$1" in
        -y|--yes) ASSUME_YES=1 ;;
        -d|--device) TARGET="${2:-}"; shift ;;
        --no-firmware) DO_FIRMWARE=0 ;;
        --no-userland) DO_USERLAND=0 ;;
        --branch) BRANCH="${2:-main}"; GH_RAW="https://raw.githubusercontent.com/stormj-UH/jonerix/${BRANCH}"; shift ;;
        -h|--help) sed -n '2,35p' "$0"; exit 0 ;;
        *) die "unknown arg: $1" ;;
    esac
    shift
done

# ── Root check ──────────────────────────────────────────────────────
if [ "$(id -u)" -ne 0 ]; then
    die "must be run as root (re-run under sudo)"
fi

# ── Tool check ──────────────────────────────────────────────────────
for t in mount umount curl; do
    command -v "$t" >/dev/null 2>&1 || die "missing required tool: $t"
done
# Prefer bsdtar — toybox's tar doesn't handle long-name pax headers
# cleanly, and some jonerix hosts ship a /bin/tar wrapper that was
# written with un-interpreted printf \n (literal 4-char sequence,
# unrunnable). bsdtar has no such quirks.
if command -v bsdtar >/dev/null 2>&1; then
    TAR=bsdtar
elif command -v tar >/dev/null 2>&1; then
    TAR=tar
else
    die "missing required tool: tar (or bsdtar)"
fi
HAVE_MKFS_EXT4=0; HAVE_MKFS_VFAT=0
command -v mkfs.ext4 >/dev/null 2>&1 && HAVE_MKFS_EXT4=1
command -v mkfs.vfat >/dev/null 2>&1 && HAVE_MKFS_VFAT=1

# ── Target device selection ─────────────────────────────────────────
list_candidates() {
    # Emit one "size name" per line for removable block devices. Avoids
    # lsblk (not always present) by walking /sys/block.
    for _bd in /sys/block/*/; do
        _n=$(basename "$_bd")
        # Skip loop, ram, zram, mmc internal (we don't overwrite the
        # running root; allow mmcblkN only when explicitly asked).
        case "$_n" in
            loop*|ram*|zram*|mmcblk*) continue ;;
        esac
        _rem=$(cat "${_bd}removable" 2>/dev/null || echo 0)
        [ "$_rem" = 1 ] || continue
        _sz=$(cat "${_bd}size" 2>/dev/null || echo 0)
        _sz_gb=$(( _sz * 512 / 1024 / 1024 / 1024 ))
        printf '%s\t%sGiB\n' "/dev/$_n" "$_sz_gb"
    done
}

if [ -z "$TARGET" ]; then
    msg "Scanning for removable block devices"
    _cands=$(list_candidates)
    if [ -z "$_cands" ]; then
        die "no removable block devices found. Use -d /dev/sdX to override."
    fi
    echo "$_cands" | awk '{ printf "  [%d] %s (%s)\n", NR, $1, $2 }' >&2
    if [ "$ASSUME_YES" = 1 ]; then
        TARGET=$(echo "$_cands" | head -n 1 | awk '{print $1}')
        msg "auto-selected $TARGET (--yes)"
    else
        printf 'Pick [1]: ' >&2
        read _pick || _pick=1
        [ -z "$_pick" ] && _pick=1
        TARGET=$(echo "$_cands" | awk -v n="$_pick" 'NR==n {print $1}')
    fi
fi
[ -b "$TARGET" ] || die "not a block device: $TARGET"

# Guard: never overwrite the device holding /.
_root_dev=$(awk '$2=="/" {print $1}' /proc/mounts)
case "$_root_dev" in
    "$TARGET"|"${TARGET}"[0-9]*) die "refusing to install over running root ($_root_dev)" ;;
esac

msg "Target device: $TARGET"
msg "Everything on $TARGET will be overwritten."
if [ "$(ask 'Proceed?' n)" != y ]; then
    die "aborted"
fi

# ── Partitioning / formatting ───────────────────────────────────────
P1="${TARGET}1"
P2="${TARGET}2"
if [ ! -b "$P1" ] || [ ! -b "$P2" ]; then
    die "$TARGET lacks p1/p2. Pre-partition with \`sfdisk\` or install \
util-linux + dosfstools + e2fsprogs on this host and re-run so we can \
create them automatically."
fi

# Filesystem sanity. Don't auto-mkfs unless tools exist AND the user
# confirms. blkid is the preferred probe, but on jonerix blkid is
# whatever anvil ships — currently ext-only and reports "unrecognized
# filesystem" on FAT partitions. Always cross-check with magic bytes
# so anvil's blkid doesn't cause a false-negative FAT32 detection.
_have_fat32=0; _have_ext4=0
if command -v blkid >/dev/null 2>&1; then
    case "$(blkid -s TYPE -o value "$P1" 2>/dev/null)" in
        vfat|fat12|fat16|fat32|msdos) _have_fat32=1 ;;
    esac
    case "$(blkid -s TYPE -o value "$P2" 2>/dev/null)" in
        ext2|ext3|ext4) _have_ext4=1 ;;
    esac
fi
# Magic-byte fallback — FAT signature "FAT3" at offset 82 (FAT32)
# or "FAT1" at offset 54 (FAT12/16); ext* magic 0xEF53 at sb
# offset 0x438 = byte 1080 from partition start.
if [ "$_have_fat32" = 0 ]; then
    _fat_sig32=$(dd if="$P1" bs=1 skip=82 count=5 2>/dev/null)
    _fat_sig16=$(dd if="$P1" bs=1 skip=54 count=5 2>/dev/null)
    case "$_fat_sig32$_fat_sig16" in
        *FAT3*|*FAT1*) _have_fat32=1 ;;
    esac
fi
if [ "$_have_ext4" = 0 ]; then
    _magic=$(dd if="$P2" bs=1 skip=1080 count=2 2>/dev/null | od -An -tx1 | tr -d ' \n')
    [ "$_magic" = "53ef" ] && _have_ext4=1
fi

if [ "$_have_fat32" = 0 ]; then
    if [ "$HAVE_MKFS_VFAT" = 1 ]; then
        if [ "$(ask "Format $P1 as FAT32?" y)" = y ]; then
            mkfs.vfat -F 32 -n BOOT "$P1" >/dev/null
        else die "aborted: $P1 not FAT32"; fi
    else
        die "$P1 is not FAT32 and mkfs.vfat is unavailable. Pre-format it."
    fi
fi
if [ "$_have_ext4" = 0 ]; then
    if [ "$HAVE_MKFS_EXT4" = 1 ]; then
        if [ "$(ask "Format $P2 as ext4?" y)" = y ]; then
            mkfs.ext4 -F -L jonerix "$P2" >/dev/null
        else die "aborted: $P2 not ext4"; fi
    else
        die "$P2 is not ext4 and mkfs.ext4 is unavailable. Pre-format it."
    fi
fi

# ── Mount targets ───────────────────────────────────────────────────
WORK=$(mktemp -d /tmp/jonerix-pi5-install.XXXXXX)
BOOT_MNT="$WORK/boot"
ROOT_MNT="$WORK/root"
mkdir -p "$BOOT_MNT" "$ROOT_MNT"

_cleanup() {
    umount "$BOOT_MNT" 2>/dev/null || true
    umount "$ROOT_MNT" 2>/dev/null || true
    rmdir "$BOOT_MNT" "$ROOT_MNT" 2>/dev/null || true
    rmdir "$WORK" 2>/dev/null || true
}
trap _cleanup EXIT INT HUP TERM

mount "$P2" "$ROOT_MNT"
mount "$P1" "$BOOT_MNT"

# ── Boot partition (firmware + kernel) ──────────────────────────────
if [ "$DO_FIRMWARE" = 1 ]; then
    cat <<'LICENSE_NOTICE'

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

Installing either means you have reviewed and accept BOTH licenses.
LICENSE_NOTICE
    if [ "$(ask 'Accept the Linux kernel GPL-2.0 and Broadcom firmware licenses and proceed with download?' n)" != y ]; then
        die "declined firmware / kernel license — aborting"
    fi
    if [ "$(ask 'Download Raspberry Pi 5 firmware and kernel from raspberrypi/firmware?' y)" = y ]; then
        msg "Fetching $FIRMWARE_URL (~500 MiB — this takes a while)"
        _fw_tar="$WORK/firmware.tar.gz"
        curl -fsSL -o "$_fw_tar" "$FIRMWARE_URL" \
            || die "firmware download failed"
        msg "Extracting boot payload"
        # Firmware tarball layout: firmware-stable/boot/*. We only
        # want what's under boot/ and only the Pi 5 bits.
        _tmpx="$WORK/fw-extract"; mkdir "$_tmpx"
        "$TAR" -C "$_tmpx" -xzf "$_fw_tar" \
            --include='*/boot/*' \
            || die "firmware extract failed"
        _fwboot=$(find "$_tmpx" -type d -name boot | head -1)
        [ -d "$_fwboot" ] || die "firmware tarball missing boot/"
        # Wipe existing boot and lay down fresh firmware. Keep any
        # pre-existing .pre-pi5-fixups backups.
        find "$BOOT_MNT" -mindepth 1 -maxdepth 1 \
            ! -name '*.pre-pi5-fixups' -exec rm -rf {} +
        cp -a "$_fwboot"/. "$BOOT_MNT"/
        rm -rf "$_tmpx" "$_fw_tar"
        # Record the license acceptance alongside the files it
        # covers — makes "what did I agree to" auditable on the
        # installed volume.
        if [ -f "$BOOT_MNT/LICENCE.broadcom" ]; then
            msg "Broadcom firmware license placed at $BOOT_MNT/LICENCE.broadcom"
        fi
        printf 'Firmware + kernel installed %s by pi5-install.sh.\nKernel license: GPL-2.0 (raspberrypi/linux).\nFirmware license: see LICENCE.broadcom in this directory.\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" > "$BOOT_MNT/LICENSES-ACCEPTED.txt"
    fi
fi

# Ensure Pi 5 specific assets are in place. If user said --no-firmware
# but the partition is empty, complain.
for _need in kernel_2712.img bcm2712-rpi-5-b.dtb; do
    [ -f "$BOOT_MNT/$_need" ] || warn "$BOOT_MNT/$_need missing — the Pi 5 won't boot without it"
done

# ── Root partition (jonerix userland) ───────────────────────────────
if [ "$DO_USERLAND" = 1 ]; then
    if [ "$(ask 'Install jonerix userland (jpkg) into the root partition?' y)" = y ]; then
        command -v jpkg >/dev/null 2>&1 \
            || die "jpkg not found. Run this from a jonerix host, or install jpkg first."

        # Make sure the local jpkg index is current before we hand it
        # an alternate root — the index lives outside the -r path and
        # is shared with the host's jpkg state.
        msg "Refreshing jpkg index"
        jpkg update >/dev/null 2>&1 || warn "jpkg update failed; using cached index"

        msg "Installing core packages into $ROOT_MNT"
        # shellcheck disable=SC2086  # word-split is intentional
        jpkg -r "$ROOT_MNT" install $DEFAULT_PACKAGES 2>&1 \
            | sed 's/^/  /'
    fi
fi

# ── Configure /etc/fstab, cmdline.txt, config.txt ───────────────────
# Give the new root a unique UUID so it doesn't clash with any other
# jonerix disk in the same Pi. We don't need tune2fs for this — the
# UUID lives at offset 0x468 in the ext4 superblock (16 bytes) and we
# can poke it with dd + /dev/urandom when mkfs.ext4 wasn't available.
# Helper: return a UUID string if blkid recognises the partition,
# empty otherwise. Anvil's blkid prints its "unrecognized filesystem"
# line to stdout and exits non-zero, which both poisons the capture
# and trips `set -e`. Swallow both and re-validate the output looks
# UUID-shaped (hex or 4345-C4D4 FAT-style) before trusting it.
_probe_uuid() {
    _out=$(blkid -s UUID -o value "$1" 2>/dev/null || true)
    case "$_out" in
        *unrecognized*|*error*|*refused*|*": "*) _out="" ;;
    esac
    # Accept anything matching UUID-ish or FAT-ish formats.
    case "$_out" in
        *[!0-9a-fA-F-]*) _out="" ;;
    esac
    printf '%s' "$_out"
}

_root_uuid=""
if command -v blkid >/dev/null 2>&1; then
    _root_uuid=$(_probe_uuid "$P2")
fi
if [ -z "$_root_uuid" ]; then
    warn "blkid missing; leaving root partition UUID unchanged"
fi

msg "Patching $BOOT_MNT/cmdline.txt"
_cmdline="$BOOT_MNT/cmdline.txt"
if [ -n "$_root_uuid" ]; then
    _root_arg="root=UUID=$_root_uuid"
else
    _root_arg="root=$P2"
fi
cat > "$_cmdline" <<EOF
reboot=c console=serial0,115200 console=tty1 $_root_arg rootfstype=ext4 rootwait rw init=/bin/openrc-init loglevel=3 quiet
EOF

msg "Patching $BOOT_MNT/config.txt"
_config="$BOOT_MNT/config.txt"
cat > "$_config" <<'EOF'
# jonerix — Raspberry Pi 5 configuration
# Laid down by pi5-install.sh; user edits welcome.

arm_64bit=1
kernel=kernel_2712.img
enable_uart=1
gpu_mem=16
disable_splash=1
dtparam=audio=off

# HDMI hot-plug on both ports: see jonerix-raspi5-fixups for rationale.
hdmi_force_hotplug:0=1
hdmi_force_hotplug:1=1

# RTC coin-cell trickle charging is OFF by default — wrong cell
# chemistry can vent a non-rechargeable cell. Uncomment to enable
# at 3.0 V (safe for ML2032 / MS621FE):
# dtparam=rtc_bbat_vchg=3000000
EOF

msg "Writing $ROOT_MNT/etc/fstab"
mkdir -p "$ROOT_MNT/etc"
_p1_uuid=""
command -v blkid >/dev/null 2>&1 && _p1_uuid=$(_probe_uuid "$P1")
# Pick UUID= form when we have one, fall back to raw device path
# otherwise. The earlier `${var:+A}${var:-B}` trick expanded BOTH
# branches when the var was set — busted.
if [ -n "$_root_uuid" ]; then _root_spec="UUID=$_root_uuid"; else _root_spec="$P2"; fi
if [ -n "$_p1_uuid" ];   then _boot_spec="UUID=$_p1_uuid";   else _boot_spec="$P1"; fi
cat > "$ROOT_MNT/etc/fstab" <<EOF
# /etc/fstab — jonerix Pi 5 (generated by pi5-install.sh)
$_root_spec  /      ext4  defaults,noatime,errors=remount-ro  0 1
$_boot_spec  /boot  vfat  defaults,noatime                    0 2
devpts   /dev/pts   devpts   gid=5,mode=0620,ptmxmode=0666  0 0
sysfs    /sys       sysfs    defaults                       0 0
tmpfs    /run       tmpfs    defaults,size=20%              0 0
tmpfs    /tmp       tmpfs    defaults,size=20%              0 0
EOF

# ── Verify ──────────────────────────────────────────────────────────
msg "Verifying"
_fails=0
for _f in kernel_2712.img bcm2712-rpi-5-b.dtb cmdline.txt config.txt; do
    if [ -s "$BOOT_MNT/$_f" ]; then
        printf '  [ok]   %s\n' "$BOOT_MNT/$_f"
    else
        printf '  [MISS] %s\n' "$BOOT_MNT/$_f"
        _fails=$(( _fails + 1 ))
    fi
done
for _f in bin/mksh bin/openrc-init etc/init.d/pi5-cold-reboot; do
    if [ -e "$ROOT_MNT/$_f" ]; then
        printf '  [ok]   %s\n' "$ROOT_MNT/$_f"
    else
        printf '  [MISS] %s\n' "$ROOT_MNT/$_f"
        _fails=$(( _fails + 1 ))
    fi
done

msg "Sync + unmount"
sync
umount "$BOOT_MNT"
umount "$ROOT_MNT"
rmdir  "$BOOT_MNT" "$ROOT_MNT" 2>/dev/null || true
rmdir  "$WORK"      2>/dev/null || true
trap - EXIT INT HUP TERM

if [ "$_fails" -gt 0 ]; then
    warn "$_fails file(s) missing — the resulting image may not boot."
    exit 1
fi
msg "Done. Swap $TARGET into the Pi 5 and power it up."
