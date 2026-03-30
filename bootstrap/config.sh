#!/bin/sh
# DEPRECATED: This file is kept for reference only.
# The old stage0/stage1/stage2 bootstrap pipeline has been replaced by:
#   - bootstrap/build-all.sh (builds packages via jpkg)
#   - packages/bootstrap/*/recipe.toml (per-package from-source recipes)
#   - Dockerfile.minimal + Dockerfile.develop (jpkg-based image assembly)
#
# Original description:
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
MUSL_SHA256="a9a118bbe84d8764da0ea0d28b3ab3fae8477fc7e4085d90102b8596fc7c75e4"

# Compression
ZSTD_VERSION="1.5.6"
ZSTD_SHA256="8c29e06cf42aacc1eafc4077ae2ec6c6fcb96a626157e0593d5e82a34fd403c1"

LZ4_VERSION="1.10.0"
LZ4_SHA256="537512904744b35e232912055ccf8ec66d768639ff3abe5788d90d792ec5f48b"

ZLIB_VERSION="1.3.1"
ZLIB_SHA256="9a93b2b7dfdac77ceba5a558a580e74667dd6fede4585b91eefb60f03b72df23"

# TLS
LIBRESSL_VERSION="4.0.0"
LIBRESSL_SHA256="4d841955f0acc3dfc71d0e3dd35f283af461222350e26843fea9731c0246a1e4"

# Coreutils
TOYBOX_VERSION="0.8.11"
TOYBOX_SHA256="15aa3f832f4ec1874db761b9950617f99e1e38144c22da39a71311093bfe67dc"

# Shell
MKSH_VERSION="R59c"
MKSH_SHA256="77ae1665a337f1c48c61d6b961db3e52119b38e58884d1c89684af31f87bc506"

# Build tool
SAMURAI_VERSION="1.2"
SAMURAI_SHA256="3b8cf51548dfc49b7efe035e191ff5e1963ebc4fe8f6064a5eefc5343eaf78a5"

# Compiler suite
LLVM_VERSION="19.1.5"
LLVM_SHA256="bd8445f554aae33d50d3212a15e993a667c0ad1b694ac1977f3463db3338e542"

# Init system
OPENRC_VERSION="0.54"
OPENRC_SHA256="c84ff1d8e468c043fe136d11d3d34d6bb28328267d1352526a5d18cdf4c60fb0"

# SSH
DROPBEAR_VERSION="2024.86"
DROPBEAR_SHA256="e78936dffc395f2e0db099321d6be659190966b99712b55c530dd0a1822e0a5e"

# HTTP client
CURL_VERSION="8.11.1"
CURL_SHA256="c7ca7db48b0909743eaef34250da02c19bc61d4f1dcedd6603f109409536ab56"

# DHCP
DHCPCD_VERSION="10.1.0"
DHCPCD_SHA256="abc307c63853da3199baa5c1e15fd5ede9d68d068b2a59ca14c5a6768e9cc3b7"

# DNS resolver
UNBOUND_VERSION="1.22.0"
UNBOUND_SHA256="c5dd1bdef5d5685b2cedb749158dd152c52d44f65529a34ac15cd88d4b1b3d43"

# Privilege escalation
DOAS_VERSION="6.8.2"
DOAS_SHA256="4e98828056d6266bd8f2c93e6ecf12a63a71dbfd70a5ea99ccd4ab6d0745adf0"

# Logging
SOCKLOG_VERSION="2.2.3"
SOCKLOG_SHA256="960410d2b54b165e636afc9fb201effdbf48aa032cbfefd544adf0b0656a91d3"

# Cron
SNOOZE_VERSION="0.5"
SNOOZE_SHA256="d63fde85d9333188bed5996baabd833eaa00842ce117443ffbf8719c094be414"

# Man pages
MANDOC_VERSION="1.14.6"
MANDOC_SHA256="8bf0d570f01e70a6e124884088870cbed7537f36328d512909eb10cd53179d9c"

# Network configuration
IFUPDOWN_NG_VERSION="0.12.1"
IFUPDOWN_NG_SHA256="d42c8c18222efbce0087b92a14ea206de4e865d5c9dde6c0864dcbb2b45f2d85"

# Parallel gzip
PIGZ_VERSION="2.8"
PIGZ_SHA256="eb872b4f0e1f0ebe59c9f7bd8c506c4204893ba6a8492de31df416f0d5170fd0"

# Text editor
NVI_VERSION="2.2.1"
NVI_SHA256="9f7c9aef3924c0e39ef96e1aadb8f5d396825b8251addab1290aa866cf3d5af4"

# Python interpreter (build dependency for Node.js + scripting)
PYTHON_VERSION="3.12.8"
PYTHON_SHA256="c909157bb25ec114e5869124cc2a9c4a4d4c1e957ca4ff553f1edc692101154e"

# JavaScript runtime
NODEJS_VERSION="22.12.0"
NODEJS_SHA256="3157e7c002b6e964bdbefb331ec38db1e2dceb064ab11c038275155461b22ce3"

# Linux kernel
LINUX_VERSION="6.12.5"
LINUX_SHA256="39207fce1ce42838e085261bae0af5ce4a0843aa777cfc0f5c49bc7729602bcd"

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
SAMURAI_SOURCE="https://github.com/michaelforney/samurai/releases/download/${SAMURAI_VERSION}/samurai-${SAMURAI_VERSION}.tar.gz"
LLVM_SOURCE="https://github.com/llvm/llvm-project/releases/download/llvmorg-${LLVM_VERSION}/llvm-project-${LLVM_VERSION}.src.tar.xz"
OPENRC_SOURCE="https://github.com/OpenRC/openrc/archive/refs/tags/${OPENRC_VERSION}.tar.gz"
DROPBEAR_SOURCE="https://matt.ucc.asn.au/dropbear/releases/dropbear-${DROPBEAR_VERSION}.tar.bz2"
CURL_SOURCE="https://curl.se/download/curl-${CURL_VERSION}.tar.xz"
DHCPCD_SOURCE="https://github.com/NetworkConfiguration/dhcpcd/releases/download/v${DHCPCD_VERSION}/dhcpcd-${DHCPCD_VERSION}.tar.xz"
UNBOUND_SOURCE="https://nlnetlabs.nl/downloads/unbound/unbound-${UNBOUND_VERSION}.tar.gz"
DOAS_SOURCE="https://github.com/Duncaen/OpenDoas/releases/download/v${DOAS_VERSION}/opendoas-${DOAS_VERSION}.tar.xz"
# Note: Doas version bumped from 6.3p12 (never released) to 6.8.2
SOCKLOG_SOURCE="https://smarden.org/socklog/socklog-${SOCKLOG_VERSION}.tar.gz"
SNOOZE_SOURCE="https://github.com/leahneukirchen/snooze/archive/v${SNOOZE_VERSION}.tar.gz"
MANDOC_SOURCE="https://mandoc.bsd.lv/snapshots/mandoc-${MANDOC_VERSION}.tar.gz"
IFUPDOWN_NG_SOURCE="https://github.com/ifupdown-ng/ifupdown-ng/archive/refs/tags/ifupdown-ng-${IFUPDOWN_NG_VERSION}.tar.gz"
PIGZ_SOURCE="https://zlib.net/pigz/pigz-${PIGZ_VERSION}.tar.gz"
NVI_SOURCE="https://github.com/lichray/nvi2/archive/refs/tags/v${NVI_VERSION}.tar.gz"
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
