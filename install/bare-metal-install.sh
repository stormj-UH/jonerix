#!/bin/sh
# bare-metal-install.sh — jonerix bare-metal installer skeleton
#
# DRAFT / WIP — This script is a skeleton. Review and adapt it before use.
# Many sections contain TODO markers for steps that need completion.
#
# Usage (run from a live environment — Alpine USB, rescue shell, etc.):
#   ./bare-metal-install.sh /dev/sdX
#
# What this script does:
#   1. Creates a GPT partition table on the target disk
#   2. Creates a 256 MiB EFI System Partition (FAT32)
#   3. Creates the remaining space as an ext4 root partition
#   4. Mounts both partitions
#   5. Installs Limine EFI binary to the ESP
#   6. Extracts the jonerix rootfs to the root partition
#   7. Copies the kernel to the ESP
#   8. Writes limine.conf with the correct root UUID
#   9. Unmounts and exits
#
# REQUIREMENTS (live environment must have):
#   sfdisk or sgdisk  — GPT partitioning (GPL, installer-only, not in jonerix rootfs)
#   mkfs.fat          — FAT32 formatting (GPL, installer-only)
#   mkfs.ext4         — ext4 formatting (GPL, installer-only)
#   blkid             — UUID detection (toybox 0BSD, or util-linux)
#   mount / umount    — filesystem mounting
#
# LICENSE NOTE:
#   The GPL tools (sfdisk, mkfs.fat, mkfs.ext4) are used HERE in the installer
#   environment only. They do not ship in the installed jonerix rootfs.
#   Limine (BSD-2-Clause) is the only bootloader binary that ends up on the disk.

set -e

# ── Configurable variables ────────────────────────────────────────────────
# TODO: Accept these as arguments or a config file for unattended installs.
JONERIX_ROOTFS="${JONERIX_ROOTFS:-}"           # Path to jonerix rootfs tarball
JONERIX_KERNEL="${JONERIX_KERNEL:-}"           # Path to vmlinuz
LIMINE_EFI="${LIMINE_EFI:-}"                   # Path to BOOTX64.EFI
LIMINE_CONF_EXTRA="${LIMINE_CONF_EXTRA:-}"     # Extra limine.conf content (optional)
ESP_SIZE_MIB="${ESP_SIZE_MIB:-256}"            # EFI System Partition size in MiB
MOUNT_ROOT="${MOUNT_ROOT:-/mnt/jonerix-install}"

# ── Argument parsing ──────────────────────────────────────────────────────
if [ -z "$1" ]; then
    echo "Usage: $0 <target-disk>"
    echo "  Example: $0 /dev/sda"
    echo ""
    echo "Environment variables (all required unless noted):"
    echo "  JONERIX_ROOTFS   — path to jonerix rootfs tarball (.tar.gz or .tar.zst)"
    echo "  JONERIX_KERNEL   — path to vmlinuz kernel image"
    echo "  LIMINE_EFI       — path to BOOTX64.EFI (from limine jpkg: /share/limine/BOOTX64.EFI)"
    echo "  ESP_SIZE_MIB     — EFI partition size in MiB (default: 256)"
    exit 1
fi

DISK="$1"

# ── Preflight checks ──────────────────────────────────────────────────────
die() { echo "ERROR: $*" >&2; exit 1; }
warn() { echo "WARNING: $*" >&2; }
need_cmd() { command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1"; }

[ "$(id -u)" = "0" ] || die "Must be run as root"
[ -b "$DISK" ] || die "Not a block device: $DISK"

need_cmd blkid
need_cmd mount
need_cmd umount
need_cmd mkfs.fat    # dosfstools — GPL, installer-only
need_cmd mkfs.ext4   # e2fsprogs — GPL, installer-only

# Prefer sfdisk for partitioning (util-linux), fall back to sgdisk (gptfdisk)
if command -v sfdisk >/dev/null 2>&1; then
    PART_TOOL=sfdisk
elif command -v sgdisk >/dev/null 2>&1; then
    PART_TOOL=sgdisk
else
    die "No partitioning tool found. Install sfdisk (util-linux) or sgdisk (gptfdisk)."
fi

# Check required files
[ -n "$JONERIX_ROOTFS" ] || die "JONERIX_ROOTFS not set"
[ -f "$JONERIX_ROOTFS" ] || die "Rootfs tarball not found: $JONERIX_ROOTFS"
[ -n "$JONERIX_KERNEL" ] || die "JONERIX_KERNEL not set"
[ -f "$JONERIX_KERNEL" ] || die "Kernel not found: $JONERIX_KERNEL"
[ -n "$LIMINE_EFI" ] || die "LIMINE_EFI not set"
[ -f "$LIMINE_EFI" ] || die "Limine EFI binary not found: $LIMINE_EFI"

# ── Confirm ───────────────────────────────────────────────────────────────
echo ""
echo "=== jonerix bare-metal installer ==="
echo ""
echo "  Target disk    : $DISK"
echo "  Rootfs tarball : $JONERIX_ROOTFS"
echo "  Kernel         : $JONERIX_KERNEL"
echo "  Limine EFI     : $LIMINE_EFI"
echo "  ESP size       : ${ESP_SIZE_MIB} MiB"
echo ""
echo "WARNING: ALL DATA ON $DISK WILL BE DESTROYED."
echo ""
printf "Type 'yes' to continue: "
read CONFIRM
[ "$CONFIRM" = "yes" ] || { echo "Aborted."; exit 0; }

# ── Step 1: Partition the disk ────────────────────────────────────────────
echo "==> Partitioning $DISK with GPT..."

# Determine partition device name convention (/dev/sda1 vs /dev/nvme0n1p1)
case "$DISK" in
    *nvme*|*mmcblk*) PART_PREFIX="${DISK}p" ;;
    *)               PART_PREFIX="${DISK}"   ;;
esac

ESP_PART="${PART_PREFIX}1"
ROOT_PART="${PART_PREFIX}2"

if [ "$PART_TOOL" = "sfdisk" ]; then
    # sfdisk: create GPT with ESP (EFI System) + Linux root
    sfdisk "$DISK" << EOF
label: gpt
1 : size=${ESP_SIZE_MIB}MiB, type=uefi
2 : type=linux
EOF
else
    # sgdisk fallback
    sgdisk --zap-all "$DISK"
    sgdisk --new=1:0:+${ESP_SIZE_MIB}M --typecode=1:ef00 --change-name=1:"EFI System" "$DISK"
    sgdisk --new=2:0:0 --typecode=2:8300 --change-name=2:"jonerix root" "$DISK"
fi

# Re-read partition table
partprobe "$DISK" 2>/dev/null || true
sleep 1

[ -b "$ESP_PART" ]  || die "ESP partition not found: $ESP_PART"
[ -b "$ROOT_PART" ] || die "Root partition not found: $ROOT_PART"

# ── Step 2: Format partitions ─────────────────────────────────────────────
echo "==> Formatting ESP ($ESP_PART) as FAT32..."
mkfs.fat -F32 -n "EFI" "$ESP_PART"

echo "==> Formatting root ($ROOT_PART) as ext4..."
mkfs.ext4 -L "jonerix" "$ROOT_PART"

# ── Step 3: Mount partitions ──────────────────────────────────────────────
echo "==> Mounting filesystems under $MOUNT_ROOT..."
mkdir -p "$MOUNT_ROOT"
mount "$ROOT_PART" "$MOUNT_ROOT"

mkdir -p "$MOUNT_ROOT/boot/efi"
mount "$ESP_PART" "$MOUNT_ROOT/boot/efi"

# Ensure unmount on exit
cleanup() {
    echo "==> Cleaning up mounts..."
    umount "$MOUNT_ROOT/boot/efi" 2>/dev/null || true
    umount "$MOUNT_ROOT"           2>/dev/null || true
}
trap cleanup EXIT

# ── Step 4: Extract jonerix rootfs ────────────────────────────────────────
echo "==> Extracting jonerix rootfs to $MOUNT_ROOT..."
case "$JONERIX_ROOTFS" in
    *.tar.gz|*.tgz)
        tar -xzf "$JONERIX_ROOTFS" -C "$MOUNT_ROOT"
        ;;
    *.tar.zst)
        # TODO: ensure zstd is available in the live environment
        tar --use-compress-program=zstd -xf "$JONERIX_ROOTFS" -C "$MOUNT_ROOT"
        ;;
    *.tar)
        tar -xf "$JONERIX_ROOTFS" -C "$MOUNT_ROOT"
        ;;
    *)
        die "Unrecognised rootfs archive format: $JONERIX_ROOTFS"
        ;;
esac

# ── Step 5: Install kernel to ESP ─────────────────────────────────────────
echo "==> Installing kernel to ESP..."
# TODO: If building a jonerix ISO, the kernel path may differ.
# For a disk install, the kernel lives on the ESP so Limine can load it
# directly from FAT32 without needing to read the ext4 root partition.
mkdir -p "$MOUNT_ROOT/boot/efi/boot"
cp "$JONERIX_KERNEL" "$MOUNT_ROOT/boot/efi/boot/vmlinuz"

# ── Step 6: Install Limine EFI binary ────────────────────────────────────
echo "==> Installing Limine EFI binary..."
mkdir -p "$MOUNT_ROOT/boot/efi/EFI/BOOT"
cp "$LIMINE_EFI" "$MOUNT_ROOT/boot/efi/EFI/BOOT/BOOTX64.EFI"

# TODO: For aarch64 support, also copy BOOTAA64.EFI.
# LIMINE_EFI_AA64="${LIMINE_EFI_AA64:-}"
# [ -f "$LIMINE_EFI_AA64" ] && cp "$LIMINE_EFI_AA64" "$MOUNT_ROOT/boot/efi/EFI/BOOT/BOOTAA64.EFI"

# ── Step 7: Get root partition UUID ──────────────────────────────────────
echo "==> Detecting root partition UUID..."
ROOT_UUID=$(blkid -s UUID -o value "$ROOT_PART")
[ -n "$ROOT_UUID" ] || die "Failed to detect UUID for $ROOT_PART"
echo "    Root UUID: $ROOT_UUID"

# ── Step 8: Write limine.conf ─────────────────────────────────────────────
echo "==> Writing limine.conf to ESP..."
mkdir -p "$MOUNT_ROOT/boot/efi/boot/limine"
cat > "$MOUNT_ROOT/boot/efi/boot/limine/limine.conf" << EOF
# Limine bootloader configuration — generated by bare-metal-install.sh
# Edit this file on the EFI System Partition to change boot options.
timeout: 5
default_entry: 1

/ jonerix
    protocol: linux
    path: boot():/boot/vmlinuz
    cmdline: root=UUID=${ROOT_UUID} rootfstype=ext4 init=/bin/openrc-init console=tty0 quiet
EOF

# TODO: If using an initramfs, add:
#     module_path: boot():/boot/initramfs.cpio.gz

echo ""
echo "==> Installation complete."
echo ""
echo "    Disk layout:"
echo "      ${ESP_PART}  — EFI System Partition (FAT32, ${ESP_SIZE_MIB} MiB)"
echo "      ${ROOT_PART} — jonerix root (ext4, UUID=${ROOT_UUID})"
echo ""
echo "    Boot files on ESP:"
echo "      /EFI/BOOT/BOOTX64.EFI         — Limine UEFI bootloader"
echo "      /boot/vmlinuz                  — jonerix kernel"
echo "      /boot/limine/limine.conf       — Limine configuration"
echo ""
echo "    Next steps:"
echo "      1. Verify your UEFI firmware has Secure Boot disabled (or sign the EFI binary)."
echo "      2. Set the boot order in UEFI setup to boot from $DISK."
echo "      3. Reboot."
echo ""
echo "TODO items before this script is production-ready:"
echo "  - Add aarch64 EFI binary support (BOOTAA64.EFI)"
echo "  - Add BIOS legacy install path (limine bios-install)"
echo "  - Add optional efibootmgr call to register UEFI boot entry"
echo "  - Add initramfs support (mkinitfs from Alpine, or jmkinitfs)"
echo "  - Add swap partition/file option"
echo "  - Add disk image mode (dd + losetup) for VM image creation"
echo "  - Verify all required commands before starting (preflight)"
echo "  - Add dry-run mode"
