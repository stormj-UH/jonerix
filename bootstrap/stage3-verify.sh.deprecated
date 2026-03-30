#!/bin/sh
# jonerix Stage 3 — Self-Hosting Verification
#
# This script is intended to run INSIDE a booted jonerix system (Stage 2 image).
# It rebuilds the entire system from source using only jonerix's own tools
# (no Alpine, no GPL tools) and compares the result to the Stage 2 rootfs
# for bit-for-bit reproducibility.
#
# If the rebuild matches, the system is proven:
#   1. Fully self-hosting (can rebuild itself)
#   2. Reproducible (deterministic output)
#   3. GPL-free at runtime (no GPL tool was needed to build it)
#
# SPDX-License-Identifier: MIT

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
. "${SCRIPT_DIR}/config.sh"

# =========================================================================
# Verification environment
# =========================================================================

VERIFY_SYSROOT="/jonerix-verify/sysroot"
VERIFY_ROOTFS="/jonerix-verify/rootfs"
VERIFY_BUILDDIR="/jonerix-verify/build"
VERIFY_OUTPUT="/jonerix-verify/output"
REFERENCE_ROOTFS="${DESTDIR:-/}"

msg "Stage 3: Self-hosting verification"
msg "Verify sysroot: ${VERIFY_SYSROOT}"
msg "Verify rootfs:  ${VERIFY_ROOTFS}"
msg "Reference:      ${REFERENCE_ROOTFS}"

# =========================================================================
# Pre-flight: verify we are running on jonerix
# =========================================================================

msg "Checking runtime environment..."

ERRORS=0

# Check for essential jonerix tools
for _tool in mksh clang ld.lld llvm-ar samu curl toybox; do
    if command -v "$_tool" >/dev/null 2>&1; then
        msg "  Found: $_tool ($(command -v "$_tool"))"
    else
        echo "ERROR: Required tool not found: $_tool" >&2
        echo "  Stage 3 must run inside a booted jonerix system." >&2
        ERRORS=$((ERRORS + 1))
    fi
done

# Check that no GPL build tools are present (they shouldn't be in jonerix)
for _gpl_tool in gcc g++ bash gawk busybox apt dpkg rpm yum; do
    if command -v "$_gpl_tool" >/dev/null 2>&1; then
        echo "WARNING: GPL tool found in PATH: $_gpl_tool" >&2
        echo "  This suggests we are NOT running on a clean jonerix system." >&2
    fi
done

# Check musl (should be the only libc)
if [ -f /lib/ld-musl-*.so.1 ] 2>/dev/null; then
    msg "  C library: musl (OK)"
elif [ -f /lib/libc.musl-*.so.1 ] 2>/dev/null; then
    msg "  C library: musl (OK)"
else
    echo "WARNING: musl dynamic linker not found — are we on jonerix?" >&2
fi

if [ "${ERRORS}" -gt 0 ]; then
    die "Pre-flight failed: ${ERRORS} missing tools. Cannot proceed."
fi

# =========================================================================
# Check for jpkg
# =========================================================================

HAVE_JPKG=0
if command -v jpkg >/dev/null 2>&1; then
    HAVE_JPKG=1
    msg "jpkg found — will use 'jpkg build-world' for rebuild"
else
    msg "jpkg not found — will use manual bootstrap for rebuild"
fi

# =========================================================================
# Prepare clean build environment
# =========================================================================

msg "Preparing verification build environment..."

rm -rf /jonerix-verify
mkdir -p "${VERIFY_SYSROOT}" "${VERIFY_ROOTFS}" "${VERIFY_BUILDDIR}" "${VERIFY_OUTPUT}"

# Override paths so config.sh helpers write to the verification dirs
export SYSROOT="${VERIFY_SYSROOT}"
export DESTDIR="${VERIFY_ROOTFS}"
export SRCDIR="${VERIFY_BUILDDIR}/src"
export BUILDDIR="${VERIFY_BUILDDIR}/obj"
export OUTPUT="${VERIFY_OUTPUT}"

mkdir -p "${SRCDIR}" "${BUILDDIR}"

# =========================================================================
# Rebuild the world
# =========================================================================

if [ "${HAVE_JPKG}" -eq 1 ]; then
    # ---------------------------------------------------------------
    # Path A: Use jpkg build-world (the intended self-hosting path)
    # ---------------------------------------------------------------
    msg "Rebuilding system via jpkg build-world..."

    jpkg build-world \
        --sysroot="${VERIFY_SYSROOT}" \
        --destdir="${VERIFY_ROOTFS}" \
        || die "jpkg build-world failed"

else
    # ---------------------------------------------------------------
    # Path B: Manual rebuild (before jpkg is fully implemented)
    # ---------------------------------------------------------------
    msg "Rebuilding system manually (jpkg not yet available)..."

    REPO_ROOT="${SCRIPT_DIR}/.."

    # Reuse Stage 1 logic but building with jonerix's own tools
    # The key difference: CC, LD, etc. point to jonerix's LLVM, not Alpine's
    export CC="/bin/clang"
    export LD="/bin/ld.lld"
    export AR="/bin/llvm-ar"
    export RANLIB="/bin/llvm-ranlib"
    export STRIP="/bin/llvm-strip"
    export NM="/bin/llvm-nm"
    export OBJCOPY="/bin/llvm-objcopy"
    export CFLAGS="-Os -pipe -fstack-protector-strong -fPIE -D_FORTIFY_SOURCE=2"
    export LDFLAGS="-Wl,-z,relro,-z,now -pie"

    NPROC=$(nproc 2>/dev/null || echo 1)

    # The build order and steps mirror stage1.sh exactly.
    # For brevity, we source stage1.sh with the overridden environment
    # variables pointing to the verification directories.

    msg "Executing stage1.sh with verification environment..."
    (
        # Re-source config to pick up new SYSROOT/DESTDIR
        . "${SCRIPT_DIR}/config.sh"
        . "${SCRIPT_DIR}/stage1.sh"
    ) || die "Verification stage1 rebuild failed"

    msg "Executing stage2.sh with verification environment..."
    (
        . "${SCRIPT_DIR}/config.sh"
        . "${SCRIPT_DIR}/stage2.sh"
    ) || die "Verification stage2 rebuild failed"
fi

# =========================================================================
# Compare: reference rootfs vs verification rootfs
# =========================================================================

msg "Comparing reference rootfs to verification rootfs..."

VERIFY_TARBALL="${VERIFY_OUTPUT}/jonerix-rootfs-${JONERIX_VERSION}.tar.zst"
REFERENCE_TARBALL="${SCRIPT_DIR}/../output/jonerix-rootfs-${JONERIX_VERSION}.tar.zst"

MATCH=1

# --- Method 1: Compare tarballs byte-for-byte (strongest test) ---
if [ -f "${REFERENCE_TARBALL}" ] && [ -f "${VERIFY_TARBALL}" ]; then
    msg "Comparing tarballs byte-for-byte..."

    REF_SHA256=$(sha256sum "${REFERENCE_TARBALL}" | cut -d' ' -f1)
    VER_SHA256=$(sha256sum "${VERIFY_TARBALL}" | cut -d' ' -f1)

    msg "  Reference: ${REF_SHA256}"
    msg "  Verify:    ${VER_SHA256}"

    if [ "${REF_SHA256}" = "${VER_SHA256}" ]; then
        msg "TARBALL MATCH: Bit-for-bit identical!"
    else
        echo "TARBALL MISMATCH: Tarballs differ." >&2
        MATCH=0
    fi
else
    msg "Tarball comparison skipped (one or both tarballs missing)"
    MATCH=0
fi

# --- Method 2: File-by-file comparison ---
msg "Running file-by-file comparison..."

DIFF_COUNT=0
COMPARED=0

# Generate sorted file lists
find "${REFERENCE_ROOTFS}" -type f -o -type l 2>/dev/null | \
    sed "s|^${REFERENCE_ROOTFS}||" | sort > /tmp/jonerix-ref-files.txt

find "${VERIFY_ROOTFS}" -type f -o -type l 2>/dev/null | \
    sed "s|^${VERIFY_ROOTFS}||" | sort > /tmp/jonerix-ver-files.txt

# Check for files present in reference but missing from verify
while IFS= read -r _file; do
    if ! grep -qxF "$_file" /tmp/jonerix-ver-files.txt; then
        echo "  MISSING in verify: $_file" >&2
        DIFF_COUNT=$((DIFF_COUNT + 1))
    fi
done < /tmp/jonerix-ref-files.txt

# Check for extra files in verify
while IFS= read -r _file; do
    if ! grep -qxF "$_file" /tmp/jonerix-ref-files.txt; then
        echo "  EXTRA in verify: $_file" >&2
        DIFF_COUNT=$((DIFF_COUNT + 1))
    fi
done < /tmp/jonerix-ver-files.txt

# Compare common files by SHA256
while IFS= read -r _file; do
    if grep -qxF "$_file" /tmp/jonerix-ver-files.txt; then
        _ref="${REFERENCE_ROOTFS}${_file}"
        _ver="${VERIFY_ROOTFS}${_file}"

        # Skip symlinks — compare targets
        if [ -L "$_ref" ] && [ -L "$_ver" ]; then
            _ref_target=$(readlink "$_ref")
            _ver_target=$(readlink "$_ver")
            if [ "$_ref_target" != "$_ver_target" ]; then
                echo "  SYMLINK DIFF: $_file -> ref:${_ref_target} ver:${_ver_target}" >&2
                DIFF_COUNT=$((DIFF_COUNT + 1))
            fi
        elif [ -f "$_ref" ] && [ -f "$_ver" ]; then
            _ref_hash=$(sha256sum "$_ref" | cut -d' ' -f1)
            _ver_hash=$(sha256sum "$_ver" | cut -d' ' -f1)
            if [ "$_ref_hash" != "$_ver_hash" ]; then
                echo "  CONTENT DIFF: $_file" >&2
                DIFF_COUNT=$((DIFF_COUNT + 1))
            fi
        fi
        COMPARED=$((COMPARED + 1))
    fi
done < /tmp/jonerix-ref-files.txt

# Clean up temp files
rm -f /tmp/jonerix-ref-files.txt /tmp/jonerix-ver-files.txt

msg "Compared ${COMPARED} files, found ${DIFF_COUNT} differences"

if [ "${DIFF_COUNT}" -gt 0 ]; then
    MATCH=0
fi

# =========================================================================
# Final license audit
# =========================================================================

msg "Running final license audit on verification rootfs..."

AUDIT_FAIL=0

# Scan all ELF binaries
for _bin in "${VERIFY_ROOTFS}/bin/"*; do
    [ -f "$_bin" ] || continue
    _basename="$(basename "$_bin")"

    # Skip kernel (sole GPL exception)
    [ "$_basename" = "vmlinuz" ] && continue

    if file "$_bin" 2>/dev/null | grep -q "dynamically linked"; then
        _interp=$(readelf -l "$_bin" 2>/dev/null | grep "interpreter" | sed 's/.*: \(.*\)]/\1/' || true)
        if echo "$_interp" | grep -q "ld-linux"; then
            echo "  AUDIT FAIL: ${_basename} linked against glibc" >&2
            AUDIT_FAIL=1
        fi
    fi
done

# Check package database
if [ -f "${VERIFY_ROOTFS}/var/db/jpkg/INDEX" ]; then
    while IFS= read -r _line; do
        _pkg=$(echo "$_line" | awk '{print $1}')
        _lic=$(echo "$_line" | awk '{print $3}')
        [ -z "$_pkg" ] && continue
        echo "$_pkg" | grep -q '^#' && continue

        case "$_lic" in
            GPL*|LGPL*|AGPL*)
                if [ "$_pkg" != "linux" ]; then
                    echo "  AUDIT FAIL: ${_pkg} (${_lic})" >&2
                    AUDIT_FAIL=1
                fi
                ;;
        esac
    done < "${VERIFY_ROOTFS}/var/db/jpkg/INDEX"
fi

if [ "${AUDIT_FAIL}" -ne 0 ]; then
    echo "LICENSE AUDIT: FAILED" >&2
else
    msg "LICENSE AUDIT: PASSED"
fi

# =========================================================================
# Report
# =========================================================================

echo ""
echo "================================================================"
echo "  Stage 3 Verification Report"
echo "================================================================"
echo ""

if [ "${MATCH}" -eq 1 ] && [ "${AUDIT_FAIL}" -eq 0 ]; then
    msg "RESULT: PASS"
    msg ""
    msg "The jonerix system is:"
    msg "  1. SELF-HOSTING: Rebuilt itself using only its own tools"
    msg "  2. REPRODUCIBLE: Output is bit-for-bit identical to Stage 2"
    msg "  3. GPL-FREE:     No GPL binaries in the runtime image"
    msg ""
    msg "Bootstrap chain proven. jonerix is ready for distribution."
    exit 0
else
    msg "RESULT: FAIL"
    msg ""
    if [ "${MATCH}" -eq 0 ]; then
        msg "  REPRODUCIBILITY: FAILED (${DIFF_COUNT} differences found)"
        msg "  This may indicate non-deterministic build steps."
        msg "  Review timestamps, randomized hashes, or path differences."
    fi
    if [ "${AUDIT_FAIL}" -ne 0 ]; then
        msg "  LICENSE AUDIT: FAILED"
        msg "  GPL-licensed code was found in the runtime image."
    fi
    msg ""
    msg "Verification artifacts preserved at /jonerix-verify/ for debugging."
    exit 1
fi
