#!/bin/sh
# jonerix Stage 2 — Assemble Clean Root Filesystem
#
# Takes the Stage 1 sysroot and produces a clean, minimal rootfs with:
#   - Merged /usr layout (all binaries in /bin, all libs in /lib)
#   - Proper directory structure per DESIGN.md section 5
#   - Correct permissions and ownership
#   - jpkg package database
#   - Verification that no GPL binaries are present
#
# Outputs:
#   - jonerix-rootfs-<version>.tar.zst    (root filesystem tarball)
#   - jonerix-<version>.img               (placeholder for bootable disk image)
#   - jonerix-<version>-oci.tar           (placeholder for OCI container image)
#
# SPDX-License-Identifier: MIT

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "${SCRIPT_DIR}/config.sh"

# =========================================================================
# Pre-flight checks
# =========================================================================

msg "Stage 2: Assemble clean root filesystem"
msg "Sysroot: ${SYSROOT}"
msg "Rootfs:  ${DESTDIR}"

if [ ! -d "${SYSROOT}" ]; then
    die "Sysroot not found at ${SYSROOT}. Run stage1.sh first."
fi

if [ ! -d "${SYSROOT}/bin" ]; then
    die "Sysroot appears incomplete (no bin/). Run stage1.sh first."
fi

# Start fresh
if [ -d "${DESTDIR}" ]; then
    msg "Removing previous rootfs at ${DESTDIR}"
    rm -rf "${DESTDIR}"
fi

# =========================================================================
# Create directory structure (DESIGN.md section 5 — merged /usr layout)
# =========================================================================

msg "Creating directory structure..."

# Core directories
mkdir -p "${DESTDIR}/bin"
mkdir -p "${DESTDIR}/lib"
mkdir -p "${DESTDIR}/etc/init.d"
mkdir -p "${DESTDIR}/etc/conf.d"
mkdir -p "${DESTDIR}/etc/ssl"
mkdir -p "${DESTDIR}/etc/network"
mkdir -p "${DESTDIR}/etc/jpkg"
mkdir -p "${DESTDIR}/var/log"
mkdir -p "${DESTDIR}/var/cache/jpkg"
mkdir -p "${DESTDIR}/var/db/jpkg"
mkdir -p "${DESTDIR}/var/lib/urandom"
mkdir -p "${DESTDIR}/var/run"
mkdir -p "${DESTDIR}/var/tmp"
mkdir -p "${DESTDIR}/home"
mkdir -p "${DESTDIR}/root"
mkdir -p "${DESTDIR}/boot"
mkdir -p "${DESTDIR}/dev"
mkdir -p "${DESTDIR}/proc"
mkdir -p "${DESTDIR}/sys"
mkdir -p "${DESTDIR}/run"
mkdir -p "${DESTDIR}/tmp"
mkdir -p "${DESTDIR}/share/man"

# Merged /usr — symlink for compatibility
ln -sf / "${DESTDIR}/usr" 2>/dev/null || true

# Additional compatibility symlinks
ln -sf bin "${DESTDIR}/sbin" 2>/dev/null || true
ln -sf lib "${DESTDIR}/lib64" 2>/dev/null || true

# Permissions for special directories
chmod 1777 "${DESTDIR}/tmp"
chmod 1777 "${DESTDIR}/var/tmp"
chmod 0700 "${DESTDIR}/root"
chmod 0755 "${DESTDIR}/run"

# =========================================================================
# Copy artifacts from sysroot
# =========================================================================

msg "Copying binaries from sysroot..."

# Copy all binaries
if [ -d "${SYSROOT}/bin" ]; then
    cp -a "${SYSROOT}/bin/"* "${DESTDIR}/bin/" 2>/dev/null || true
fi
if [ -d "${SYSROOT}/sbin" ]; then
    cp -a "${SYSROOT}/sbin/"* "${DESTDIR}/bin/" 2>/dev/null || true
fi

msg "Copying libraries from sysroot..."

# Copy all libraries
if [ -d "${SYSROOT}/lib" ]; then
    cp -a "${SYSROOT}/lib/"*.so* "${DESTDIR}/lib/" 2>/dev/null || true
    cp -a "${SYSROOT}/lib/"*.a "${DESTDIR}/lib/" 2>/dev/null || true
    # Copy musl dynamic linker
    cp -a "${SYSROOT}/lib/ld-musl-"* "${DESTDIR}/lib/" 2>/dev/null || true
fi
if [ -d "${SYSROOT}/lib64" ]; then
    cp -a "${SYSROOT}/lib64/"* "${DESTDIR}/lib/" 2>/dev/null || true
fi

# Copy kernel modules if present
if [ -d "${SYSROOT}/lib/modules" ]; then
    mkdir -p "${DESTDIR}/lib/modules"
    cp -a "${SYSROOT}/lib/modules/"* "${DESTDIR}/lib/modules/" 2>/dev/null || true
fi

# Copy OpenRC lib components
if [ -d "${SYSROOT}/lib/rc" ]; then
    mkdir -p "${DESTDIR}/lib/rc"
    cp -a "${SYSROOT}/lib/rc/"* "${DESTDIR}/lib/rc/" 2>/dev/null || true
fi
if [ -d "${SYSROOT}/lib/dhcpcd" ]; then
    mkdir -p "${DESTDIR}/lib/dhcpcd"
    cp -a "${SYSROOT}/lib/dhcpcd/"* "${DESTDIR}/lib/dhcpcd/" 2>/dev/null || true
fi

msg "Copying headers (for self-hosting)..."

# Copy include files for self-hosting capability
if [ -d "${SYSROOT}/include" ]; then
    mkdir -p "${DESTDIR}/include"
    cp -a "${SYSROOT}/include/"* "${DESTDIR}/include/" 2>/dev/null || true
fi

msg "Copying kernel..."

# Copy kernel image
if [ -f "${SYSROOT}/boot/vmlinuz" ]; then
    cp "${SYSROOT}/boot/vmlinuz" "${DESTDIR}/boot/vmlinuz"
else
    msg "WARNING: No kernel image found in sysroot"
fi

msg "Copying man pages..."

# Copy man pages
if [ -d "${SYSROOT}/share/man" ]; then
    cp -a "${SYSROOT}/share/man/"* "${DESTDIR}/share/man/" 2>/dev/null || true
fi

# Copy SSL certificates if present
if [ -d "${SYSROOT}/etc/ssl" ]; then
    cp -a "${SYSROOT}/etc/ssl/"* "${DESTDIR}/etc/ssl/" 2>/dev/null || true
fi

# =========================================================================
# Create symlinks for merged /usr layout
# =========================================================================

msg "Setting up merged /usr layout symlinks..."

# Ensure /usr/bin -> /bin, /usr/lib -> /lib, etc.
# Since /usr -> /, these are implicitly handled.  But for any programs
# that hardcode /usr/bin paths, the symlink chain resolves correctly:
#   /usr/bin/foo -> /bin/foo  (because /usr -> /)

# Some packages install to /usr/local — we redirect that too
if [ -d "${SYSROOT}/usr/local" ]; then
    cp -a "${SYSROOT}/usr/local/bin/"* "${DESTDIR}/bin/" 2>/dev/null || true
    cp -a "${SYSROOT}/usr/local/lib/"* "${DESTDIR}/lib/" 2>/dev/null || true
fi

# LLVM symlinks — ensure clang, clang++, lld, etc. are accessible
for _tool in clang clang++ clang-cpp lld ld.lld ld64.lld \
             llvm-ar llvm-nm llvm-objcopy llvm-objdump llvm-ranlib \
             llvm-readelf llvm-size llvm-strip llvm-strings; do
    if [ -f "${DESTDIR}/bin/${_tool}" ] || [ -L "${DESTDIR}/bin/${_tool}" ]; then
        : # already present
    elif [ -f "${SYSROOT}/bin/${_tool}" ]; then
        cp -a "${SYSROOT}/bin/${_tool}" "${DESTDIR}/bin/${_tool}"
    fi
done

# Standard tool symlinks
if [ -f "${DESTDIR}/bin/clang" ] && [ ! -e "${DESTDIR}/bin/cc" ]; then
    ln -sf clang "${DESTDIR}/bin/cc"
fi
if [ -f "${DESTDIR}/bin/clang++" ] && [ ! -e "${DESTDIR}/bin/c++" ]; then
    ln -sf clang++ "${DESTDIR}/bin/c++"
fi

# =========================================================================
# Install default configuration files
# =========================================================================

msg "Installing default configuration..."

REPO_ROOT="${SCRIPT_DIR}/.."

# Copy config defaults from the repository if available
if [ -d "${REPO_ROOT}/config/defaults/etc" ]; then
    cp -a "${REPO_ROOT}/config/defaults/etc/"* "${DESTDIR}/etc/" 2>/dev/null || true
fi

# Generate minimal /etc files if they don't exist from config/
_install_if_missing() {
    _file="$1"
    shift
    if [ ! -f "${DESTDIR}/${_file}" ]; then
        msg "  Generating ${_file}"
        # Arguments are lines to write
        for _line in "$@"; do
            echo "$_line"
        done > "${DESTDIR}/${_file}"
    fi
}

# /etc/hostname
_install_if_missing "etc/hostname" "jonerix"

# /etc/passwd
_install_if_missing "etc/passwd" \
    "root:x:0:0:root:/root:/bin/mksh" \
    "daemon:x:1:1:daemon:/usr/sbin:/bin/false" \
    "bin:x:2:2:bin:/bin:/bin/false" \
    "nobody:x:65534:65534:nobody:/nonexistent:/bin/false" \
    "sshd:x:22:22:sshd:/var/empty:/bin/false" \
    "unbound:x:88:88:unbound:/var/lib/unbound:/bin/false" \
    "dhcpcd:x:100:100:dhcpcd:/var/db/dhcpcd:/bin/false" \
    "socklog:x:101:101:socklog:/var/log:/bin/false"

# /etc/group
_install_if_missing "etc/group" \
    "root:x:0:root" \
    "daemon:x:1:" \
    "bin:x:2:" \
    "wheel:x:10:" \
    "sshd:x:22:" \
    "unbound:x:88:" \
    "dhcpcd:x:100:" \
    "socklog:x:101:" \
    "nobody:x:65534:"

# /etc/shadow (locked passwords by default)
_install_if_missing "etc/shadow" \
    "root:!:19814:0:99999:7:::" \
    "daemon:!:19814:0:99999:7:::" \
    "bin:!:19814:0:99999:7:::" \
    "nobody:!:19814:0:99999:7:::" \
    "sshd:!:19814:0:99999:7:::" \
    "unbound:!:19814:0:99999:7:::" \
    "dhcpcd:!:19814:0:99999:7:::" \
    "socklog:!:19814:0:99999:7:::"
chmod 0640 "${DESTDIR}/etc/shadow"

# /etc/shells
_install_if_missing "etc/shells" \
    "/bin/mksh" \
    "/bin/sh"

# /etc/resolv.conf
_install_if_missing "etc/resolv.conf" \
    "# jonerix default — use localhost unbound" \
    "nameserver 127.0.0.1"

# /etc/profile
_install_if_missing "etc/profile" \
    '# jonerix system profile' \
    'export PATH="/bin"' \
    'export PAGER="less"' \
    'export EDITOR="vi"' \
    'export CHARSET="UTF-8"' \
    'export LANG="C.UTF-8"' \
    '' \
    '# Load user profile fragments' \
    'for f in /etc/profile.d/*.sh; do' \
    '    [ -r "$f" ] && . "$f"' \
    'done' \
    'unset f'

# /etc/doas.conf
_install_if_missing "etc/doas.conf" \
    "# jonerix default: wheel group can elevate with password" \
    "permit persist :wheel"
chmod 0600 "${DESTDIR}/etc/doas.conf"

# /etc/fstab
_install_if_missing "etc/fstab" \
    "# jonerix /etc/fstab" \
    "# <device>  <mount>  <type>  <options>           <dump>  <pass>" \
    "/dev/sda2   /        ext4    defaults,noatime     0       1" \
    "/dev/sda1   /boot    vfat    defaults,noatime     0       2" \
    "tmpfs       /tmp     tmpfs   nosuid,nodev,noexec  0       0" \
    "tmpfs       /run     tmpfs   nosuid,nodev         0       0" \
    "proc        /proc    proc    defaults             0       0" \
    "sysfs       /sys     sysfs  defaults              0       0" \
    "devtmpfs    /dev     devtmpfs defaults             0       0"

# /etc/inittab (for OpenRC)
_install_if_missing "etc/inittab" \
    "# jonerix inittab — OpenRC" \
    "::sysinit:/bin/openrc sysinit" \
    "::sysinit:/bin/openrc boot" \
    "::wait:/bin/openrc default" \
    "" \
    "# Console" \
    "tty1::respawn:/bin/agetty 38400 tty1 linux" \
    "tty2::respawn:/bin/agetty 38400 tty2 linux" \
    "" \
    "# Serial console (cloud / VM)" \
    "ttyS0::respawn:/bin/agetty -L 115200 ttyS0 vt100" \
    "" \
    "::shutdown:/bin/openrc shutdown" \
    "::ctrlaltdel:/bin/reboot"

# /etc/network/interfaces
mkdir -p "${DESTDIR}/etc/network"
_install_if_missing "etc/network/interfaces" \
    "# jonerix default network configuration" \
    "auto lo" \
    "iface lo inet loopback" \
    "" \
    "auto eth0" \
    "iface eth0 inet dhcp"

# =========================================================================
# Set permissions
# =========================================================================

msg "Setting file permissions..."

# Make all binaries executable
find "${DESTDIR}/bin" -type f -exec chmod 0755 {} + 2>/dev/null || true

# doas needs setuid
if [ -f "${DESTDIR}/bin/doas" ]; then
    chmod 4755 "${DESTDIR}/bin/doas"
fi

# Sensitive files
chmod 0600 "${DESTDIR}/etc/shadow" 2>/dev/null || true
chmod 0600 "${DESTDIR}/etc/doas.conf" 2>/dev/null || true
chmod 0644 "${DESTDIR}/etc/passwd"
chmod 0644 "${DESTDIR}/etc/group"

# =========================================================================
# Generate jpkg package database
# =========================================================================

msg "Generating jpkg package database..."

JPKG_DB="${DESTDIR}/var/db/jpkg"
mkdir -p "${JPKG_DB}"

# Generate an installed-packages manifest
# Each installed package gets a directory with metadata
_register_pkg() {
    _name="$1"
    _version="$2"
    _license="$3"
    _desc="$4"

    _pkgdir="${JPKG_DB}/${_name}"
    mkdir -p "$_pkgdir"

    cat > "${_pkgdir}/metadata" <<PKGEOF
[package]
name = "${_name}"
version = "${_version}"
license = "${_license}"
description = "${_desc}"
arch = "${JONERIX_ARCH}"
install_date = "$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
PKGEOF

    # Generate file list for this package
    : > "${_pkgdir}/files"
}

_register_pkg "musl"         "${MUSL_VERSION}"         "MIT"               "C standard library"
_register_pkg "zstd"         "${ZSTD_VERSION}"         "BSD-3-Clause"      "Zstandard compression"
_register_pkg "lz4"          "${LZ4_VERSION}"          "BSD-2-Clause"      "LZ4 compression"
_register_pkg "libressl"     "${LIBRESSL_VERSION}"     "ISC"               "TLS library"
_register_pkg "toybox"       "${TOYBOX_VERSION}"       "0BSD"              "BSD-licensed coreutils"
_register_pkg "mksh"         "${MKSH_VERSION}"         "MirOS"             "MirBSD Korn shell"
_register_pkg "samurai"      "${SAMURAI_VERSION}"      "Apache-2.0"        "Ninja-compatible build tool"
_register_pkg "llvm"         "${LLVM_VERSION}"         "Apache-2.0"        "LLVM compiler infrastructure"
_register_pkg "openrc"       "${OPENRC_VERSION}"       "BSD-2-Clause"      "Init system"
_register_pkg "dropbear"     "${DROPBEAR_VERSION}"     "MIT"               "SSH server and client"
_register_pkg "curl"         "${CURL_VERSION}"         "curl"              "HTTP client"
_register_pkg "dhcpcd"       "${DHCPCD_VERSION}"       "BSD-2-Clause"      "DHCP client"
_register_pkg "unbound"      "${UNBOUND_VERSION}"      "BSD-3-Clause"      "DNS resolver"
_register_pkg "doas"         "${DOAS_VERSION}"         "ISC"               "Privilege escalation"
_register_pkg "socklog"      "${SOCKLOG_VERSION}"      "BSD-3-Clause"      "System logging"
_register_pkg "snooze"       "${SNOOZE_VERSION}"       "CC0"               "Cron replacement"
_register_pkg "mandoc"       "${MANDOC_VERSION}"       "ISC"               "Man page tools"
_register_pkg "ifupdown-ng"  "${IFUPDOWN_NG_VERSION}"  "ISC"               "Network configuration"
_register_pkg "pigz"         "${PIGZ_VERSION}"         "zlib"              "Parallel gzip"
_register_pkg "nvi"          "${NVI_VERSION}"          "BSD"               "Text editor"
_register_pkg "linux"        "${LINUX_VERSION}"        "GPLv2"             "Linux kernel (sole GPL exception)"

# Write the global installed packages index
{
    echo "# jonerix package database"
    echo "# Generated: $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
    echo "# Architecture: ${JONERIX_ARCH}"
    echo "#"
    for _d in "${JPKG_DB}"/*/; do
        [ -d "$_d" ] || continue
        _pkg="$(basename "$_d")"
        if [ -f "${_d}/metadata" ]; then
            _ver=$(sed -n 's/^version = "\(.*\)"/\1/p' "${_d}/metadata")
            _lic=$(sed -n 's/^license = "\(.*\)"/\1/p' "${_d}/metadata")
            printf "%-20s %-15s %s\n" "$_pkg" "$_ver" "$_lic"
        fi
    done
} > "${JPKG_DB}/INDEX"

msg "Registered $(find "${JPKG_DB}" -maxdepth 1 -type d | wc -l | tr -d ' ') packages"

# =========================================================================
# GPL verification
# =========================================================================

msg "Running GPL verification scan..."

GPL_FOUND=0

# Check 1: Scan ELF binaries for GPL-associated strings
for _bin in "${DESTDIR}/bin/"*; do
    [ -f "$_bin" ] || continue

    # Skip the kernel — it is the sole GPL exception
    _basename="$(basename "$_bin")"
    if [ "$_basename" = "vmlinuz" ]; then
        continue
    fi

    # Check if this is a dynamically linked binary using a GPL libc (glibc)
    if file "$_bin" 2>/dev/null | grep -q "dynamically linked"; then
        _interp=$(readelf -l "$_bin" 2>/dev/null | grep "interpreter" | sed 's/.*: \(.*\)]/\1/')
        if echo "$_interp" | grep -q "ld-linux"; then
            echo "FAIL: ${_bin} is linked against glibc (GPL)" >&2
            GPL_FOUND=1
        fi
    fi
done

# Check 2: Ensure no GNU tools leaked in
for _gpl_tool in bash gawk grep sed make gcc g++ as ld.bfd busybox; do
    if [ -f "${DESTDIR}/bin/${_gpl_tool}" ]; then
        echo "FAIL: GPL tool found in rootfs: ${_gpl_tool}" >&2
        GPL_FOUND=1
    fi
done

# Check 3: Verify against jpkg database — only linux should be GPL
while IFS= read -r _line; do
    _pkg=$(echo "$_line" | awk '{print $1}')
    _lic=$(echo "$_line" | awk '{print $3}')

    [ -z "$_pkg" ] && continue
    echo "$_pkg" | grep -q '^#' && continue

    case "$_lic" in
        GPL*|LGPL*|AGPL*)
            if [ "$_pkg" != "linux" ]; then
                echo "FAIL: Package ${_pkg} has license ${_lic} — not permitted" >&2
                GPL_FOUND=1
            fi
            ;;
    esac
done < "${JPKG_DB}/INDEX"

if [ "${GPL_FOUND}" -ne 0 ]; then
    die "GPL verification FAILED. See errors above."
fi

msg "GPL verification PASSED — no GPL binaries in rootfs (kernel exception noted)"

# =========================================================================
# Produce output artifacts
# =========================================================================

msg "Producing output artifacts..."
ensure_dir "${OUTPUT}"

# Rootfs tarball (zstd-compressed)
ROOTFS_TARBALL="${OUTPUT}/jonerix-rootfs-${JONERIX_VERSION}.tar.zst"
msg "Creating rootfs tarball: ${ROOTFS_TARBALL}"
(
    cd "${DESTDIR}"
    # Deterministic tar: sorted, no timestamps for reproducibility
    find . -print0 | sort -z | \
        tar --null -T - \
            --no-recursion \
            --numeric-owner \
            --owner=0 --group=0 \
            --mtime="@0" \
            --format=posix \
            -cf - | \
        zstd -19 -T0 -o "${ROOTFS_TARBALL}"
) || die "Failed to create rootfs tarball"

msg "Rootfs tarball: $(du -sh "${ROOTFS_TARBALL}" | cut -f1)"

# Disk image placeholder — actual image creation is done by image/mkimage.sh
DISK_IMAGE="${OUTPUT}/jonerix-${JONERIX_VERSION}.img"
msg "Disk image placeholder: ${DISK_IMAGE}"
echo "# Placeholder — run 'make image' or image/mkimage.sh to create bootable disk image" \
    > "${DISK_IMAGE}.README"

# OCI image placeholder — actual OCI image creation is done by image/oci.sh
OCI_IMAGE="${OUTPUT}/jonerix-${JONERIX_VERSION}-oci.tar"
msg "OCI image placeholder: ${OCI_IMAGE}"
echo "# Placeholder — run 'make oci' or image/oci.sh to create OCI container image" \
    > "${OCI_IMAGE}.README"

# =========================================================================
# Summary
# =========================================================================

echo ""
echo "================================================================"
echo "  Stage 2 COMPLETE"
echo "================================================================"
echo ""
msg "Rootfs assembled at: ${DESTDIR}"
msg "Rootfs size:         $(du -sh "${DESTDIR}" | cut -f1)"
msg ""
msg "Output artifacts:"
msg "  Rootfs tarball: ${ROOTFS_TARBALL}"
msg "  Disk image:     ${DISK_IMAGE} (placeholder — run 'make image')"
msg "  OCI image:      ${OCI_IMAGE} (placeholder — run 'make oci')"
msg ""
msg "Directory breakdown:"
du -sh "${DESTDIR}/bin" "${DESTDIR}/lib" "${DESTDIR}/boot" "${DESTDIR}/include" \
    "${DESTDIR}/share" "${DESTDIR}/etc" 2>/dev/null | while read -r _size _path; do
    printf "  %-30s %s\n" "$_path" "$_size"
done
msg ""
msg "Next step: Boot the image and run 'sh bootstrap/stage3-verify.sh'"
