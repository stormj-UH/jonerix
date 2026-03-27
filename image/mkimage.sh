#!/bin/sh
# mkimage.sh — Create a bootable jonerix disk image
#
# Produces a GPT-partitioned disk image with:
#   Partition 1: ESP (FAT32, ~64MB) — Linux kernel as EFI stub
#   Partition 2: Root (ext4) — jonerix root filesystem
#
# Usage: mkimage.sh <rootfs-tarball> [output-image] [image-size]
#
# Requires: losetup, mkfs.vfat, mkfs.ext4, mount, tar, efibootmgr (optional)
# Must be run as root.
#
# Part of jonerix — MIT License

set -eu

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

ROOTFS_TAR="${1:?Usage: mkimage.sh <rootfs-tarball> [output-image] [image-size]}"
OUTPUT="${2:-jonerix.img}"
IMAGE_SIZE="${3:-512M}"

ESP_SIZE_MB=64
LABEL_ESP="JONERIX-ESP"
LABEL_ROOT="jonerix-root"
KERNEL_CMDLINE="root=PARTLABEL=${LABEL_ROOT} rootfstype=ext4 init=/bin/openrc-init ro quiet"

WORK_DIR=""
LOOP_DEV=""

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

die() {
    printf "mkimage: error: %s\n" "$1" >&2
    exit 1
}

info() {
    printf "mkimage: %s\n" "$1"
}

cleanup() {
    set +e
    if [ -n "$WORK_DIR" ]; then
        umount "$WORK_DIR/rootfs/boot/efi" 2>/dev/null
        umount "$WORK_DIR/rootfs" 2>/dev/null
        umount "$WORK_DIR/esp" 2>/dev/null
    fi
    if [ -n "$LOOP_DEV" ]; then
        losetup -d "$LOOP_DEV" 2>/dev/null
    fi
    if [ -n "$WORK_DIR" ]; then
        rm -rf "$WORK_DIR"
    fi
}

trap cleanup EXIT INT TERM

require_root() {
    if [ "$(id -u)" -ne 0 ]; then
        die "must be run as root"
    fi
}

require_cmd() {
    for cmd in "$@"; do
        command -v "$cmd" >/dev/null 2>&1 || die "required command not found: $cmd"
    done
}

# ---------------------------------------------------------------------------
# Validate inputs
# ---------------------------------------------------------------------------

require_root
require_cmd losetup sgdisk mkfs.vfat mkfs.ext4 mount umount tar

[ -f "$ROOTFS_TAR" ] || die "rootfs tarball not found: $ROOTFS_TAR"

info "Creating jonerix disk image"
info "  Rootfs: $ROOTFS_TAR"
info "  Output: $OUTPUT"
info "  Size:   $IMAGE_SIZE"

# ---------------------------------------------------------------------------
# Create work directory
# ---------------------------------------------------------------------------

WORK_DIR="$(mktemp -d /tmp/jonerix-mkimage.XXXXXX)"

# ---------------------------------------------------------------------------
# Step 1: Create blank disk image
# ---------------------------------------------------------------------------

info "Allocating disk image ($IMAGE_SIZE)..."
truncate -s "$IMAGE_SIZE" "$OUTPUT"

# ---------------------------------------------------------------------------
# Step 2: Create GPT partition table
# ---------------------------------------------------------------------------

info "Creating GPT partition table..."

# Partition 1: EFI System Partition
# Partition 2: Linux root filesystem
sgdisk \
    --clear \
    --new=1:2048:+${ESP_SIZE_MB}M --typecode=1:EF00 --change-name=1:"$LABEL_ESP" \
    --new=2:0:0                   --typecode=2:8300 --change-name=2:"$LABEL_ROOT" \
    "$OUTPUT"

# ---------------------------------------------------------------------------
# Step 3: Attach loop device
# ---------------------------------------------------------------------------

info "Attaching loop device..."
LOOP_DEV="$(losetup --find --show --partscan "$OUTPUT")"
info "  Loop device: $LOOP_DEV"

# Wait for partition devices to appear
sleep 1

PART_ESP="${LOOP_DEV}p1"
PART_ROOT="${LOOP_DEV}p2"

[ -b "$PART_ESP" ]  || die "ESP partition device not found: $PART_ESP"
[ -b "$PART_ROOT" ] || die "Root partition device not found: $PART_ROOT"

# ---------------------------------------------------------------------------
# Step 4: Format partitions
# ---------------------------------------------------------------------------

info "Formatting ESP as FAT32..."
mkfs.vfat -F 32 -n "$LABEL_ESP" "$PART_ESP"

info "Formatting root as ext4..."
mkfs.ext4 -L "$LABEL_ROOT" -O ^has_journal -m 1 -q "$PART_ROOT"

# ---------------------------------------------------------------------------
# Step 5: Mount partitions
# ---------------------------------------------------------------------------

info "Mounting partitions..."

mkdir -p "$WORK_DIR/rootfs" "$WORK_DIR/esp"

mount "$PART_ROOT" "$WORK_DIR/rootfs"
mkdir -p "$WORK_DIR/rootfs/boot/efi"
mount "$PART_ESP" "$WORK_DIR/rootfs/boot/efi"

# ---------------------------------------------------------------------------
# Step 6: Extract rootfs
# ---------------------------------------------------------------------------

info "Extracting rootfs tarball..."

# Detect compression
case "$ROOTFS_TAR" in
    *.tar.zst|*.tar.zstd)
        zstd -dc "$ROOTFS_TAR" | tar -xf - -C "$WORK_DIR/rootfs"
        ;;
    *.tar.gz|*.tgz)
        tar -xzf "$ROOTFS_TAR" -C "$WORK_DIR/rootfs"
        ;;
    *.tar.xz)
        tar -xJf "$ROOTFS_TAR" -C "$WORK_DIR/rootfs"
        ;;
    *.tar)
        tar -xf "$ROOTFS_TAR" -C "$WORK_DIR/rootfs"
        ;;
    *)
        die "unsupported tarball format: $ROOTFS_TAR"
        ;;
esac

# ---------------------------------------------------------------------------
# Step 7: Install kernel as EFI stub
# ---------------------------------------------------------------------------

info "Installing kernel to ESP..."

KERNEL_SRC="$WORK_DIR/rootfs/boot/vmlinuz"
if [ ! -f "$KERNEL_SRC" ]; then
    # Try common kernel names
    for k in vmlinuz vmlinuz-jonerix bzImage; do
        if [ -f "$WORK_DIR/rootfs/boot/$k" ]; then
            KERNEL_SRC="$WORK_DIR/rootfs/boot/$k"
            break
        fi
    done
fi

if [ ! -f "$KERNEL_SRC" ]; then
    die "no kernel found in rootfs /boot/"
fi

# Create EFI directory structure
mkdir -p "$WORK_DIR/rootfs/boot/efi/EFI/jonerix"
mkdir -p "$WORK_DIR/rootfs/boot/efi/EFI/BOOT"

# Copy kernel as EFI application
cp "$KERNEL_SRC" "$WORK_DIR/rootfs/boot/efi/EFI/jonerix/vmlinuz.efi"
cp "$KERNEL_SRC" "$WORK_DIR/rootfs/boot/efi/EFI/BOOT/BOOTX64.EFI"

# ---------------------------------------------------------------------------
# Step 8: Create startup.nsh for UEFI shell fallback
# ---------------------------------------------------------------------------

info "Writing UEFI startup script..."
cat > "$WORK_DIR/rootfs/boot/efi/startup.nsh" <<EOF
\\EFI\\jonerix\\vmlinuz.efi ${KERNEL_CMDLINE}
EOF

# ---------------------------------------------------------------------------
# Step 9: Write kernel command line config
# ---------------------------------------------------------------------------

# Some EFISTUB implementations read /etc/kernel/cmdline
mkdir -p "$WORK_DIR/rootfs/etc/kernel"
printf '%s\n' "$KERNEL_CMDLINE" > "$WORK_DIR/rootfs/etc/kernel/cmdline"

# ---------------------------------------------------------------------------
# Step 10: Set filesystem permissions
# ---------------------------------------------------------------------------

info "Setting filesystem permissions..."

# Ensure critical directories exist with correct permissions
chmod 0755 "$WORK_DIR/rootfs"
chmod 0700 "$WORK_DIR/rootfs/root" 2>/dev/null || true
chmod 0600 "$WORK_DIR/rootfs/etc/shadow" 2>/dev/null || true
chmod 1777 "$WORK_DIR/rootfs/tmp" 2>/dev/null || true

# ---------------------------------------------------------------------------
# Step 11: Write fstab
# ---------------------------------------------------------------------------

info "Generating /etc/fstab..."
cat > "$WORK_DIR/rootfs/etc/fstab" <<EOF
# /etc/fstab — jonerix filesystem table
# <device>                  <mount>     <type>  <options>               <dump> <pass>
PARTLABEL=${LABEL_ROOT}     /           ext4    defaults,noatime        0      1
PARTLABEL=${LABEL_ESP}      /boot/efi   vfat    defaults,noatime        0      2
tmpfs                       /tmp        tmpfs   defaults,nosuid,nodev   0      0
tmpfs                       /run        tmpfs   defaults,nosuid,nodev   0      0
devtmpfs                    /dev        devtmpfs defaults               0      0
proc                        /proc       proc    defaults                0      0
sysfs                       /sys        sysfs   defaults                0      0
EOF

# ---------------------------------------------------------------------------
# Step 12: Unmount and detach
# ---------------------------------------------------------------------------

info "Syncing and unmounting..."
sync
umount "$WORK_DIR/rootfs/boot/efi"
umount "$WORK_DIR/rootfs"
losetup -d "$LOOP_DEV"
LOOP_DEV=""

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------

IMAGE_BYTES="$(wc -c < "$OUTPUT")"
IMAGE_MB="$((IMAGE_BYTES / 1048576))"

info "Done. Image created: $OUTPUT (${IMAGE_MB} MB)"
info "Boot with: qemu-system-x86_64 -bios /usr/share/OVMF/OVMF_CODE.fd -drive file=$OUTPUT,format=raw"
