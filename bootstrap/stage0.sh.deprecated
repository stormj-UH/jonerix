#!/bin/sh
# jonerix Stage 0 — Alpine Build Host Setup
#
# This script prepares an Alpine Linux host with all the tools needed to
# cross-compile the jonerix permissive userland.  It is intended to run
# inside an Alpine container (Docker) or on a bare Alpine installation.
#
# Nothing produced by this stage enters the final jonerix image.
#
# SPDX-License-Identifier: MIT

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "${SCRIPT_DIR}/config.sh"

# =========================================================================
# Verify we are running on Alpine Linux
# =========================================================================

msg "Stage 0: Alpine build host setup"

if [ ! -f /etc/alpine-release ]; then
    die "Stage 0 must run on Alpine Linux (no /etc/alpine-release found).
If you are on another distro, run this inside an Alpine container:
  docker run --rm -v \$(pwd):/jonerix -w /jonerix alpine:latest sh bootstrap/stage0.sh"
fi

ALPINE_VERSION="$(cat /etc/alpine-release)"
msg "Detected Alpine Linux ${ALPINE_VERSION}"

# =========================================================================
# Install build dependencies
# =========================================================================

msg "Updating package index..."
apk update || die "apk update failed"

msg "Installing build dependencies..."
apk add --no-cache \
    clang \
    lld \
    llvm \
    llvm-dev \
    musl-dev \
    cmake \
    samurai \
    meson \
    git \
    curl \
    patch \
    tar \
    xz \
    bzip2 \
    zstd \
    zstd-dev \
    zstd-static \
    make \
    pkgconf \
    linux-headers \
    perl \
    python3 \
    bison \
    flex \
    bc \
    cpio \
    openssl-dev \
    ncurses-dev \
    || die "apk add failed — are you running as root?"

# =========================================================================
# Verify critical tools are present and functional
# =========================================================================

msg "Verifying tool availability..."

for tool in clang ld.lld llvm-ar llvm-nm llvm-strip llvm-objcopy cmake samu git curl patch tar sha256sum meson; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        die "Required tool not found: $tool"
    fi
done

# Verify clang can compile a trivial program
_test_c=$(mktemp /tmp/jonerix-test-XXXXXX.c)
_test_bin=$(mktemp /tmp/jonerix-test-XXXXXX)
cat > "$_test_c" <<'TESTEOF'
#include <stdio.h>
int main(void) { puts("jonerix stage0 ok"); return 0; }
TESTEOF

if ! clang -o "$_test_bin" "$_test_c" 2>/dev/null; then
    rm -f "$_test_c" "$_test_bin"
    die "clang cannot compile a trivial C program. Check musl-dev installation."
fi

rm -f "$_test_c" "$_test_bin"
msg "clang+musl compilation: OK"

# =========================================================================
# Create build directories
# =========================================================================

msg "Creating build directories..."
ensure_dir \
    "${SYSROOT}" \
    "${SYSROOT}/bin" \
    "${SYSROOT}/lib" \
    "${SYSROOT}/include" \
    "${SYSROOT}/share" \
    "${SRCDIR}" \
    "${BUILDDIR}" \
    "${OUTPUT}"

msg "Directory layout:"
msg "  SYSROOT  = ${SYSROOT}"
msg "  SRCDIR   = ${SRCDIR}"
msg "  BUILDDIR = ${BUILDDIR}"
msg "  OUTPUT   = ${OUTPUT}"

# =========================================================================
# Record build environment metadata
# =========================================================================

_meta="${BUILDDIR}/stage0-metadata.txt"
cat > "$_meta" <<EOF
jonerix Stage 0 Build Environment
==================================
Date:           $(date -u +"%Y-%m-%dT%H:%M:%SZ")
Alpine version: ${ALPINE_VERSION}
Architecture:   ${JONERIX_ARCH}
Clang version:  $(clang --version | head -1)
LLD version:    $(ld.lld --version | head -1)
CMake version:  $(cmake --version | head -1)
Kernel:         $(uname -r)
Host:           $(uname -n)
EOF

msg "Build metadata written to ${_meta}"

# =========================================================================
# Done
# =========================================================================

msg "Stage 0 complete. Build host is ready."
msg "Next step: sh bootstrap/stage1.sh"
