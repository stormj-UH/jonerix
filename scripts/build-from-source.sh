#!/bin/sh
# build-from-source.sh — Build all jpkg packages from source
#
# Runs inside a jonerix:builder container. Uses jpkg build to compile
# every recipe in packages/core/ and output .jpkg files.
#
# The builder image has everything needed: clang, go, rust, python3,
# cmake, bmake, samu — so it can rebuild itself and all other images.
#
# Usage (inside builder container):
#   sh /workspace/scripts/build-from-source.sh
#
# Usage (from host):
#   docker run --rm -v "$PWD:/workspace" -v "$PWD/.build/pkgs:/output" \
#     jonerix:builder -c 'sh /workspace/scripts/build-from-source.sh'
#
# Environment:
#   OUTPUT      Directory for .jpkg files (default: /output or /var/cache/jpkg)
#   PKG_INPUT   Build only this package (optional)
#   JOBS        Parallelism hint (default: nproc)
#
# After building, the .jpkg files can be used to assemble any image:
#   minimal:  musl zlib toybox dropbear openrc openssl curl zstd
#   core:     minimal + mksh ncurses micro ripgrep gitoxide ...
#   builder:  core + llvm rust go cmake bmake samurai perl python3 nodejs ...
#   router:   core + hostapd wpa_supplicant btop
#
# SPDX-License-Identifier: MIT

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RECIPE_DIR="${REPO_ROOT}/packages/core"

# Output directory
if [ -d /output ]; then
    OUTPUT="${OUTPUT:-/output}"
else
    OUTPUT="${OUTPUT:-/var/cache/jpkg}"
fi
mkdir -p "$OUTPUT"

ARCH=$(uname -m)
JOBS="${JOBS:-$(nproc 2>/dev/null || echo 1)}"

BUILT=0
FAILED=0
SKIPPED=0
ERRORS=""

log() {
    printf "[build-from-source] %s\n" "$*"
}

# Ensure jpkg index is current
jpkg update 2>/dev/null || true

# Single package mode
if [ -n "$PKG_INPUT" ]; then
    recipe_dir="${RECIPE_DIR}/${PKG_INPUT}"
    if [ ! -f "${recipe_dir}/recipe.toml" ]; then
        log "ERROR: no recipe at ${recipe_dir}/recipe.toml"
        exit 1
    fi
    log "Building: ${PKG_INPUT}"
    jpkg build "${recipe_dir}" --build-jpkg --output "$OUTPUT"
    log "Done: $(ls -lh "$OUTPUT/${PKG_INPUT}"*".jpkg" 2>/dev/null)"
    exit 0
fi

# Build all packages
log "Building all recipes from source"
log "Output: $OUTPUT"
log "Arch: $ARCH"
log ""

for recipe in "${RECIPE_DIR}"/*/recipe.toml; do
    pkg_dir="$(dirname "$recipe")"
    pkg_name="$(basename "$pkg_dir")"
    pkg_ver=$(grep "^version" "$recipe" | head -1 | sed 's/.*= *"\(.*\)"/\1/')

    # Skip if already built
    if [ -f "${OUTPUT}/${pkg_name}-${pkg_ver}-${ARCH}.jpkg" ]; then
        log "SKIP  ${pkg_name}-${pkg_ver} (exists)"
        SKIPPED=$((SKIPPED + 1))
        continue
    fi

    log "BUILD ${pkg_name}-${pkg_ver}"
    if timeout 3600 jpkg build "${pkg_dir}" --build-jpkg --output "$OUTPUT" 2>&1; then
        log "OK    ${pkg_name}-${pkg_ver}"
        BUILT=$((BUILT + 1))
    else
        log "FAIL  ${pkg_name}-${pkg_ver}"
        ERRORS="$ERRORS $pkg_name"
        FAILED=$((FAILED + 1))
    fi
done

log ""
log "========================================"
log "  Build complete"
log "========================================"
log "  Built:   $BUILT"
log "  Skipped: $SKIPPED"
log "  Failed:  $FAILED"
if [ -n "$ERRORS" ]; then
    log "  Errors: $ERRORS"
fi
log ""
log "Packages:"
ls -lhS "$OUTPUT"/*.jpkg 2>/dev/null || log "(none)"

[ "$FAILED" -eq 0 ]
