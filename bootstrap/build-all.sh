#!/bin/sh
# jonerix bootstrap — Build all packages from source in dependency order
#
# This script reads bootstrap/build-order.txt and builds each package
# using jpkg. Designed to run inside the jonerix-develop container or
# an Alpine CI environment with jpkg available.
#
# Usage:
#   sh bootstrap/build-all.sh [--output DIR] [--package NAME] [--continue-on-error]
#
# Options:
#   --output DIR          Write .jpkg files here (default: /var/cache/jpkg)
#   --package NAME        Build only this package (must exist in packages/bootstrap/)
#   --continue-on-error   Don't stop on build failures
#
# SPDX-License-Identifier: MIT

set -eu

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
BUILD_ORDER="${SCRIPT_DIR}/build-order.txt"
OUTPUT_DIR="${OUTPUT_DIR:-/var/cache/jpkg}"
SINGLE_PKG=""
CONTINUE_ON_ERROR=false
TIMEOUT_SECS=3600

# Parse arguments
while [ $# -gt 0 ]; do
    case "$1" in
        --output)      OUTPUT_DIR="$2"; shift 2 ;;
        --package)     SINGLE_PKG="$2"; shift 2 ;;
        --continue-on-error) CONTINUE_ON_ERROR=true; shift ;;
        --timeout)     TIMEOUT_SECS="$2"; shift 2 ;;
        -h|--help)
            sed -n '2,/^$/p' "$0" | sed 's/^# \?//'
            exit 0
            ;;
        *) echo "Unknown option: $1" >&2; exit 1 ;;
    esac
done

# Verify jpkg is available
if ! command -v jpkg >/dev/null 2>&1; then
    echo "ERROR: jpkg not found in PATH. Build it first or run inside jonerix-develop." >&2
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

# Read build order (skip comments and blank lines)
read_build_order() {
    grep -v '^\s*#' "$BUILD_ORDER" | grep -v '^\s*$' | sed 's/^\s*//;s/\s*$//'
}

# Build a single package, return 0 on success, 1 on failure
build_package() {
    _pkg="$1"
    _recipe_dir="${REPO_ROOT}/packages/bootstrap/${_pkg}"

    if [ ! -f "${_recipe_dir}/recipe.toml" ]; then
        echo "WARNING: No bootstrap recipe for ${_pkg}, skipping"
        return 0
    fi

    echo ""
    echo "================================================================"
    echo "  Building: ${_pkg}"
    echo "================================================================"
    echo ""

    if timeout "$TIMEOUT_SECS" jpkg build "${_recipe_dir}" --build-jpkg --output "$OUTPUT_DIR"; then
        echo "SUCCESS: ${_pkg}"
        return 0
    else
        echo "FAILED: ${_pkg}"
        return 1
    fi
}

# Track results
BUILT=""
FAILED=""
SKIPPED=""

if [ -n "$SINGLE_PKG" ]; then
    # Build single package
    if build_package "$SINGLE_PKG"; then
        BUILT="$SINGLE_PKG"
    else
        FAILED="$SINGLE_PKG"
    fi
else
    # Build all in order
    for pkg in $(read_build_order); do
        if build_package "$pkg"; then
            BUILT="${BUILT:+${BUILT} }${pkg}"
        else
            FAILED="${FAILED:+${FAILED} }${pkg}"
            if [ "$CONTINUE_ON_ERROR" = "false" ]; then
                echo "Stopping due to build failure. Use --continue-on-error to keep going."
                break
            fi
        fi
    done
fi

# Summary
echo ""
echo "================================================================"
echo "  Build Summary"
echo "================================================================"
echo ""

built_count=0
failed_count=0
for p in $BUILT; do built_count=$((built_count + 1)); done
for p in $FAILED; do failed_count=$((failed_count + 1)); done

echo "Built:  ${built_count} package(s)"
[ -n "$BUILT" ] && echo "  ${BUILT}"

if [ -n "$FAILED" ]; then
    echo "Failed: ${failed_count} package(s)"
    echo "  ${FAILED}"
fi

echo ""
echo "Output: ${OUTPUT_DIR}"
ls -lh "${OUTPUT_DIR}"/*.jpkg 2>/dev/null || echo "(no packages)"

# Exit with failure if any package failed
[ -z "$FAILED" ]
