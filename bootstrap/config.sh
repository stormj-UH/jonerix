#!/bin/sh
# jonerix bootstrap — shared configuration
# Sourced by stage0.sh, stage1.sh, stage2.sh, stage3-verify.sh
#
# All variables used across bootstrap stages are defined here:
#   - Package versions and SHA256 checksums
#   - Compiler flags (hardened, per DESIGN.md section 2 & 8)
#   - Directory paths for sysroot, source, and output
#   - Architecture detection
#
# SPDX-License-Identifier: MIT

set -eu

# =========================================================================
# Architecture detection
# =========================================================================

detect_arch() {
    case "$(uname -m)" in
        x86_64)  echo "x86_64"  ;;
        aarch64) echo "aarch64" ;;
        arm64)   echo "aarch64" ;;  # macOS reports arm64
        *)
            echo "ERROR: Unsupported architecture: $(uname -m)" >&2
            exit 1
            ;;
    esac
}

JONERIX_ARCH="${JONERIX_ARCH:-$(detect_arch)}"

# =========================================================================
# Directory layout
# =========================================================================

# Where Stage 1 installs cross-compiled artifacts
SYSROOT="${SYSROOT:-/jonerix-sysroot}"

# Where Stage 2 assembles the final rootfs
DESTDIR="${DESTDIR:-/jonerix-rootfs}"

# Where source tarballs are downloaded and extracted
SRCDIR="${SRCDIR:-/jonerix-build/src}"

# Where build artifacts go during compilation
BUILDDIR="${BUILDDIR:-/jonerix-build/obj}"

# Where patches live (relative to repo root)
PATCHDIR="${PATCHDIR:-$(cd "$(dirname "$0")/.." && pwd)/packages/core}"

# Final output directory for tarballs and images
OUTPUT="${OUTPUT:-$(cd "$(dirname "$0")/.." && pwd)/output}"

# =========================================================================
# Compiler and linker flags (DESIGN.md section 2 & 8)
# =========================================================================

CC="${CC:-clang}"
LD="${LD:-ld.lld}"
AR="${AR:-llvm-ar}"
RANLIB="${RANLIB:-llvm-ranlib}"
STRIP="${STRIP:-llvm-strip}"
NM="${NM:-llvm-nm}"
OBJCOPY="${OBJCOPY:-llvm-objcopy}"

CFLAGS="${CFLAGS:--Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2}"
LDFLAGS="${LDFLAGS:--Wl,-z,relro,-z,now -pie}"

# Export so child processes (make, configure) pick them up
export CC LD AR RANLIB STRIP NM OBJCOPY CFLAGS LDFLAGS
export SYSROOT DESTDIR SRCDIR BUILDDIR OUTPUT JONERIX_ARCH

# =========================================================================
# Package versions
# =========================================================================

# Core C library
MUSL_VERSION="1.2.5"
MUSL_SHA256="FIXME"  # FIXME: fill in real SHA256 after downloading

# Compression
ZSTD_VERSION="1.5.6"
ZSTD_SHA256="FIXME"  # FIXME: fill in real SHA256

LZ4_VERSION="1.10.0"
LZ4_SHA256="FIXME"  # FIXME: fill in real SHA256

ZLIB_VERSION="1.3.1"
ZLIB_SHA256="FIXME"  # FIXME: fill in real SHA256

# TLS
LIBRESSL_VERSION="4.0.0"
LIBRESSL_SHA256="FIXME"  # FIXME: fill in real SHA256

# Coreutils
TOYBOX_VERSION="0.8.11"
TOYBOX_SHA256="FIXME"  # FIXME: fill in real SHA256

# Shell
MKSH_VERSION="R59c"
MKSH_SHA256="FIXME"  # FIXME: fill in real SHA256

# Build tool
SAMURAI_VERSION="1.2"
SAMURAI_SHA256="FIXME"  # FIXME: fill in real SHA256

# Compiler suite
LLVM_VERSION="19.1.5"
LLVM_SHA256="FIXME"  # FIXME: fill in real SHA256

# Init system
OPENRC_VERSION="0.54"
OPENRC_SHA256="FIXME"  # FIXME: fill in real SHA256

# SSH
DROPBEAR_VERSION="2024.86"
DROPBEAR_SHA256="FIXME"  # FIXME: fill in real SHA256

# HTTP client
CURL_VERSION="8.11.1"
CURL_SHA256="FIXME"  # FIXME: fill in real SHA256

# DHCP
DHCPCD_VERSION="10.1.0"
DHCPCD_SHA256="FIXME"  # FIXME: fill in real SHA256

# DNS resolver
UNBOUND_VERSION="1.22.0"
UNBOUND_SHA256="FIXME"  # FIXME: fill in real SHA256

# Privilege escalation
DOAS_VERSION="6.3p12"
DOAS_SHA256="FIXME"  # FIXME: fill in real SHA256

# Logging
SOCKLOG_VERSION="2.2.3"
SOCKLOG_SHA256="FIXME"  # FIXME: fill in real SHA256

# Cron
SNOOZE_VERSION="0.5"
SNOOZE_SHA256="FIXME"  # FIXME: fill in real SHA256

# Man pages
MANDOC_VERSION="1.14.6"
MANDOC_SHA256="FIXME"  # FIXME: fill in real SHA256

# Network configuration
IFUPDOWN_NG_VERSION="0.12.1"
IFUPDOWN_NG_SHA256="FIXME"  # FIXME: fill in real SHA256

# Parallel gzip
PIGZ_VERSION="2.8"
PIGZ_SHA256="FIXME"  # FIXME: fill in real SHA256

# Text editor
NVI_VERSION="2.2.1"
NVI_SHA256="FIXME"  # FIXME: fill in real SHA256

# Python interpreter (build dependency for Node.js + scripting)
PYTHON_VERSION="3.12.8"
PYTHON_SHA256="FIXME"  # FIXME: fill in real SHA256

# JavaScript runtime
NODEJS_VERSION="22.12.0"
NODEJS_SHA256="FIXME"  # FIXME: fill in real SHA256

# Linux kernel
LINUX_VERSION="6.12.5"
LINUX_SHA256="FIXME"  # FIXME: fill in real SHA256

# =========================================================================
# Source URLs
# =========================================================================

MUSL_SOURCE="https://musl.libc.org/releases/musl-${MUSL_VERSION}.tar.gz"
ZSTD_SOURCE="https://github.com/facebook/zstd/releases/download/v${ZSTD_VERSION}/zstd-${ZSTD_VERSION}.tar.gz"
LZ4_SOURCE="https://github.com/lz4/lz4/releases/download/v${LZ4_VERSION}/lz4-${LZ4_VERSION}.tar.gz"
ZLIB_SOURCE="https://github.com/madler/zlib/releases/download/v${ZLIB_VERSION}/zlib-${ZLIB_VERSION}.tar.gz"
LIBRESSL_SOURCE="https://ftp.openbsd.org/pub/OpenBSD/LibreSSL/libressl-${LIBRESSL_VERSION}.tar.gz"
TOYBOX_SOURCE="https://landley.net/toybox/downloads/toybox-${TOYBOX_VERSION}.tar.gz"
MKSH_SOURCE="https://www.mirbsd.org/MirOS/dist/mir/mksh/mksh-${MKSH_VERSION}.tgz"
SAMURAI_SOURCE="https://github.com/nicknamenamenick/samurai/releases/download/${SAMURAI_VERSION}/samurai-${SAMURAI_VERSION}.tar.gz"
LLVM_SOURCE="https://github.com/llvm/llvm-project/releases/download/llvmorg-${LLVM_VERSION}/llvm-project-${LLVM_VERSION}.src.tar.xz"
OPENRC_SOURCE="https://github.com/OpenRC/openrc/releases/download/${OPENRC_VERSION}/openrc-${OPENRC_VERSION}.tar.gz"
DROPBEAR_SOURCE="https://matt.ucc.asn.au/dropbear/releases/dropbear-${DROPBEAR_VERSION}.tar.bz2"
CURL_SOURCE="https://curl.se/download/curl-${CURL_VERSION}.tar.xz"
DHCPCD_SOURCE="https://github.com/NetworkConfiguration/dhcpcd/releases/download/v${DHCPCD_VERSION}/dhcpcd-${DHCPCD_VERSION}.tar.xz"
UNBOUND_SOURCE="https://nlnetlabs.nl/downloads/unbound/unbound-${UNBOUND_VERSION}.tar.gz"
DOAS_SOURCE="https://github.com/Duncaen/OpenDoas/releases/download/v${DOAS_VERSION}/opendoas-${DOAS_VERSION}.tar.xz"
SOCKLOG_SOURCE="https://smarden.org/socklog/socklog-${SOCKLOG_VERSION}.tar.gz"
SNOOZE_SOURCE="https://github.com/leahneukirchen/snooze/archive/v${SNOOZE_VERSION}.tar.gz"
MANDOC_SOURCE="https://mandoc.bsd.lv/snapshots/mandoc-${MANDOC_VERSION}.tar.gz"
IFUPDOWN_NG_SOURCE="https://github.com/ifupdown-ng/ifupdown-ng/releases/download/ifupdown-ng-${IFUPDOWN_NG_VERSION}/ifupdown-ng-${IFUPDOWN_NG_VERSION}.tar.xz"
PIGZ_SOURCE="https://zlib.net/pigz/pigz-${PIGZ_VERSION}.tar.gz"
NVI_SOURCE="https://github.com/lichray/nvi2/releases/download/v${NVI_VERSION}/nvi2-${NVI_VERSION}.tar.gz"
PYTHON_SOURCE="https://www.python.org/ftp/python/${PYTHON_VERSION}/Python-${PYTHON_VERSION}.tar.xz"
NODEJS_SOURCE="https://nodejs.org/dist/v${NODEJS_VERSION}/node-v${NODEJS_VERSION}.tar.gz"
LINUX_SOURCE="https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-${LINUX_VERSION}.tar.xz"

# =========================================================================
# jonerix version
# =========================================================================

JONERIX_VERSION="${JONERIX_VERSION:-0.1.0}"

# =========================================================================
# Helper functions
# =========================================================================

# Print a section header
msg() {
    echo ">>> $*"
}

# Print an error and exit
die() {
    echo "ERROR: $*" >&2
    exit 1
}

# Download a file if it does not already exist
fetch_source() {
    _url="$1"
    _dest="$2"
    _sha256="$3"

    if [ -f "$_dest" ]; then
        msg "Already downloaded: $_dest"
    else
        msg "Downloading: $_url"
        curl -fSL -o "$_dest" "$_url" || die "Failed to download $_url"
    fi

    verify_sha256 "$_dest" "$_sha256"
}

# Verify SHA256 checksum (skip if FIXME placeholder)
verify_sha256() {
    _file="$1"
    _expected="$2"

    if [ "$_expected" = "FIXME" ]; then
        echo "WARNING: SHA256 not set for $_file (FIXME placeholder)" >&2
        return 0
    fi

    _actual=$(sha256sum "$_file" | cut -d' ' -f1)
    if [ "$_actual" != "$_expected" ]; then
        die "SHA256 mismatch for $_file: expected $_expected, got $_actual"
    fi
    msg "SHA256 OK: $_file"
}

# Extract a tarball into SRCDIR
extract_source() {
    _tarball="$1"
    _name="$2"

    if [ -d "${SRCDIR}/${_name}" ]; then
        msg "Already extracted: ${SRCDIR}/${_name}"
        return 0
    fi

    msg "Extracting: $_tarball"
    mkdir -p "${SRCDIR}"

    case "$_tarball" in
        *.tar.gz|*.tgz)   tar xzf "$_tarball" -C "${SRCDIR}" ;;
        *.tar.xz)         tar xJf "$_tarball" -C "${SRCDIR}" ;;
        *.tar.bz2)        tar xjf "$_tarball" -C "${SRCDIR}" ;;
        *.tar.zst)        zstd -d "$_tarball" --stdout | tar xf - -C "${SRCDIR}" ;;
        *)                die "Unknown archive format: $_tarball" ;;
    esac
}

# Apply patches from a directory if it exists
apply_patches() {
    _srcdir="$1"
    _patchdir="$2"

    if [ -d "$_patchdir/patches" ]; then
        for _p in "$_patchdir"/patches/*.patch; do
            [ -f "$_p" ] || continue
            msg "Applying patch: $_p"
            patch -d "$_srcdir" -p1 < "$_p" || die "Patch failed: $_p"
        done
    fi
}

# Ensure a directory exists
ensure_dir() {
    for _d in "$@"; do
        mkdir -p "$_d"
    done
}

# Count of total packages for progress display
TOTAL_PACKAGES=23
