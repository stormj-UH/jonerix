#!/bin/sh
# bootstrap/build-all.sh — Build all bootstrap packages from source
#
# Builds packages in dependency+size order using jpkg build.
# Supports resuming: skips packages whose .jpkg already exists in OUTPUT.
# On failure: logs the error, marks the package as failed, and skips
# any packages that depend on it.
#
# Usage:
#   sh bootstrap/build-all.sh [--output DIR] [--force PKG] [--dry-run]
#
# Intended to run inside a jonerix-develop container where bmake (as make),
# clang, cmake, meson, and jpkg are available.
#
# SPDX-License-Identifier: MIT

# Note: no set -eu — toybox sh does not support it.
# Error handling is done manually in the build loop.

# =========================================================================
# Configuration
# =========================================================================

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# Recipe directories: core (runtime), develop (compilers), extra (apps)
RECIPE_DIRS="${REPO_ROOT}/packages/core ${REPO_ROOT}/packages/develop ${REPO_ROOT}/packages/extra"
ORDER_FILE="${SCRIPT_DIR}/build-order.txt"
OUTPUT="${OUTPUT:-/var/cache/jpkg}"
# Auto-set source cache if sources/ directory exists in repo
if [ -z "$JPKG_SOURCE_CACHE" ] && [ -d "${REPO_ROOT}/sources" ]; then
    JPKG_SOURCE_CACHE="${REPO_ROOT}/sources"
    export JPKG_SOURCE_CACHE
fi
FORCE_PKG=""
SKIP_PKGS=""
DRY_RUN=0
NPROC=$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 1)

# Parse arguments
while [ $# -gt 0 ]; do
    case "$1" in
        --output)  OUTPUT="$2"; shift 2 ;;
        --force)   FORCE_PKG="$2"; shift 2 ;;
        --skip)    SKIP_PKGS="$SKIP_PKGS $2"; shift 2 ;;
        --dry-run) DRY_RUN=1; shift ;;
        --continue-on-error) shift ;;  # default behavior, accepted for compat
        -h|--help)
            echo "Usage: $0 [--output DIR] [--force PKG] [--skip PKG] [--dry-run]"
            echo ""
            echo "  --output DIR   Directory for built .jpkg files (default: /var/cache/jpkg)"
            echo "  --force PKG    Force rebuild of a specific package"
            echo "  --skip PKG     Skip a package (repeatable, also skips dependents)"
            echo "  --dry-run      Show what would be built without building"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

mkdir -p "$OUTPUT"

# =========================================================================
# State tracking
# =========================================================================

FAILED=""       # space-separated list of failed package names
BUILT=0
SKIPPED=0
ERRORS=0
TOTAL=0

log() {
    printf "[build-all] %s\n" "$*"
}

log_err() {
    printf "[build-all] ERROR: %s\n" "$*" >&2
}

# Check if a package name is in the failed/skipped list
is_failed() {
    case " $FAILED " in
        *" $1 "*) return 0 ;;
        *)        return 1 ;;
    esac
}

is_skipped() {
    case " $SKIP_PKGS " in
        *" $1 "*) return 0 ;;
        *)        return 1 ;;
    esac
}

# Get runtime dependencies for a package from its recipe.toml
get_deps() {
    for _rd in $RECIPE_DIRS; do
        _recipe="${_rd}/$1/recipe.toml"
        if [ -f "$_recipe" ]; then
            grep '^runtime ' "$_recipe" 2>/dev/null | head -1 | \
                sed 's/runtime = //; s/[][]//g; s/"//g; s/,/ /g'
            return
        fi
    done
}

# Check if a .jpkg for this package already exists in OUTPUT
is_built() {
    _pkg="$1"
    _recipe=""
    for _rd in $RECIPE_DIRS; do
        [ -f "${_rd}/$_pkg/recipe.toml" ] && _recipe="${_rd}/$_pkg/recipe.toml" && break
    done
    [ -z "$_recipe" ] && return 1
    _ver=$(grep '^version' "$_recipe" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
    _arch=$(uname -m)
    [ -f "${OUTPUT}/${_pkg}-${_ver}-${_arch}.jpkg" ] && return 0
    # Also check legacy format without arch
    [ -f "${OUTPUT}/${_pkg}-${_ver}.jpkg" ] && return 0
    return 1
}

# =========================================================================
# Read build order
# =========================================================================

if [ ! -f "$ORDER_FILE" ]; then
    log_err "Build order file not found: $ORDER_FILE"
    exit 1
fi

# Read packages from build-order.txt (skip comments and blank lines).
# Uses sed+grep instead of while-read because toybox sh lacks 'read'.
PACKAGES=$(sed 's/#.*//; s/[[:space:]]//g' "$ORDER_FILE" | grep -v '^$' | tr '\n' ' ')

# =========================================================================
# Build loop
# =========================================================================

log "Bootstrap build starting"
log "Output directory: $OUTPUT"
log "Parallelism: $NPROC"
log ""

for pkg in $PACKAGES; do
    TOTAL=$((TOTAL + 1))
    # Find recipe in any package directory
    pkg_dir=""
    for _rd in $RECIPE_DIRS; do
        [ -f "${_rd}/${pkg}/recipe.toml" ] && pkg_dir="${_rd}/${pkg}" && break
    done

    # Check recipe exists
    if [ -z "$pkg_dir" ] || [ ! -f "${pkg_dir}/recipe.toml" ]; then
        log_err "No recipe found for '$pkg' — skipping"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    pkg_ver=$(grep '^version' "${pkg_dir}/recipe.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')

    # Check if package is in skip list
    if is_skipped "$pkg"; then
        log "SKIP  ${pkg}-${pkg_ver} (--skip)"
        FAILED="$FAILED $pkg"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    # Check if any dependency has failed
    dep_failed=""
    for dep in $(get_deps "$pkg"); do
        if is_failed "$dep"; then
            dep_failed="$dep"
            break
        fi
    done
    if [ -n "$dep_failed" ]; then
        log "SKIP  ${pkg}-${pkg_ver} (dependency '$dep_failed' failed)"
        FAILED="$FAILED $pkg"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    # Resume support: skip already-built packages
    if [ "$pkg" != "$FORCE_PKG" ] && is_built "$pkg"; then
        log "SKIP  ${pkg}-${pkg_ver} (already built)"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    # Dry run: just show what would be built
    if [ "$DRY_RUN" -eq 1 ]; then
        log "BUILD ${pkg}-${pkg_ver} (would build)"
        continue
    fi

    # Build
    log ""
    log "================================================================"
    log "  [${TOTAL}] Building: ${pkg}-${pkg_ver}"
    log "================================================================"

    if timeout 3600 jpkg build "$pkg_dir" --build-jpkg --output "$OUTPUT" 2>&1; then
        log "OK    ${pkg}-${pkg_ver}"
        BUILT=$((BUILT + 1))
    else
        log_err "FAIL  ${pkg}-${pkg_ver}"
        FAILED="$FAILED $pkg"
        ERRORS=$((ERRORS + 1))
    fi
done

# =========================================================================
# Summary
# =========================================================================

log ""
log "================================================================"
log "  Bootstrap build complete"
log "================================================================"
log "  Built:   $BUILT"
log "  Skipped: $SKIPPED"
log "  Failed:  $ERRORS"
log "  Total:   $TOTAL"

if [ -n "$FAILED" ]; then
    log ""
    log "  Failed packages:$FAILED"
    log ""
    log "  Re-run with --force <pkg> to retry a specific package."
fi

log ""
log "Output: $OUTPUT"
ls -lhS "$OUTPUT"/*.jpkg 2>/dev/null || log "(no .jpkg files in output)"

# Exit non-zero if anything failed
[ "$ERRORS" -eq 0 ]
