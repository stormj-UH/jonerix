#!/bin/sh
# build-local.sh — Build jonerix images locally
#
# Mirrors the CI image chain: minimal -> core -> builder -> router
# Also supports building packages from source inside a builder container.
#
# Usage:
#   ./scripts/build-local.sh [TARGET...]
#
# Targets (built in dependency order):
#   minimal   Base rootfs (toybox, dropbear, openrc, musl)
#   core      Core packages (micro, ripgrep, gitoxide, mksh, ...)
#   builder   Compilers + build tools (clang, go, rust, cmake, ...)
#   router    Networking appliance (hostapd, unbound, dhcpcd, ...)
#   all       Build minimal + core + builder (default)
#   packages  Build all jpkg packages from source inside builder
#
# Environment:
#   PLATFORM    Target platform (default: auto-detect)
#   TAG_PREFIX  Image tag prefix (default: jonerix)
#   NO_CACHE    Set to 1 to disable Docker layer cache
#   PKG_OUTPUT  Output dir for built .jpkg files (default: .build/pkgs)
#   PKG_INPUT   Build only this package (with 'packages' target)
#   JOBS        Parallelism for package builds (default: auto)
#
# Examples:
#   ./scripts/build-local.sh                    # Build minimal + core + builder
#   ./scripts/build-local.sh minimal core       # Build minimal and core only
#   ./scripts/build-local.sh builder            # Build builder (assumes core exists)
#   ./scripts/build-local.sh packages           # Build all jpkg packages from source
#   PKG_INPUT=ruby ./scripts/build-local.sh packages  # Build just ruby
#
# SPDX-License-Identifier: MIT

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
PLATFORM="${PLATFORM:-$(docker info --format '{{.Architecture}}' 2>/dev/null || uname -m)}"
TAG_PREFIX="${TAG_PREFIX:-jonerix}"
PKG_OUTPUT="${PKG_OUTPUT:-$REPO_ROOT/.build/pkgs}"
JOBS="${JOBS:-$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo 4)}"
CACHE_FLAG=""
[ "${NO_CACHE}" = "1" ] && CACHE_FLAG="--no-cache"

# Normalize platform for Docker
case "$PLATFORM" in
    x86_64|amd64)  DOCKER_PLATFORM="linux/amd64"; ARCH="x86_64" ;;
    aarch64|arm64) DOCKER_PLATFORM="linux/arm64"; ARCH="aarch64" ;;
    *) echo "Unknown platform: $PLATFORM"; exit 1 ;;
esac

log() {
    printf "\033[1;34m[build-local]\033[0m %s\n" "$*"
}

err() {
    printf "\033[1;31m[build-local] ERROR:\033[0m %s\n" "$*" >&2
    exit 1
}

require_image() {
    if ! docker image inspect "$1" >/dev/null 2>&1; then
        err "$1 not found. Build it first: $0 $(echo "$1" | sed "s/${TAG_PREFIX}://")"
    fi
}

# ── Image build functions ────────────────────────────────────────────────

build_minimal() {
    log "Building ${TAG_PREFIX}:minimal (${DOCKER_PLATFORM})"
    docker build \
        --platform "$DOCKER_PLATFORM" \
        -f "$REPO_ROOT/Dockerfile.minimal" \
        --build-arg BUILDER=alpine:latest \
        --tag "${TAG_PREFIX}:minimal" \
        $CACHE_FLAG \
        "$REPO_ROOT"
    log "Done: ${TAG_PREFIX}:minimal"
}

build_core() {
    require_image "${TAG_PREFIX}:minimal"
    log "Building ${TAG_PREFIX}:core (${DOCKER_PLATFORM})"
    docker build \
        --platform "$DOCKER_PLATFORM" \
        -f "$REPO_ROOT/Dockerfile.core" \
        --build-arg MINIMAL_IMAGE="${TAG_PREFIX}:minimal" \
        --tag "${TAG_PREFIX}:core" \
        $CACHE_FLAG \
        "$REPO_ROOT"
    log "Done: ${TAG_PREFIX}:core"
}

build_builder() {
    require_image "${TAG_PREFIX}:core"
    log "Building ${TAG_PREFIX}:builder (${DOCKER_PLATFORM})"
    docker build \
        --platform "$DOCKER_PLATFORM" \
        -f "$REPO_ROOT/Dockerfile.builder" \
        --build-arg CORE_IMAGE="${TAG_PREFIX}:core" \
        --tag "${TAG_PREFIX}:builder" \
        $CACHE_FLAG \
        "$REPO_ROOT"
    log "Done: ${TAG_PREFIX}:builder"
}

build_router() {
    require_image "${TAG_PREFIX}:core"
    log "Building ${TAG_PREFIX}:router (${DOCKER_PLATFORM})"
    docker build \
        --platform "$DOCKER_PLATFORM" \
        -f "$REPO_ROOT/Dockerfile.router" \
        --build-arg CORE_IMAGE="${TAG_PREFIX}:core" \
        --tag "${TAG_PREFIX}:router" \
        $CACHE_FLAG \
        "$REPO_ROOT"
    log "Done: ${TAG_PREFIX}:router"
}

# ── Package build (from source) ─────────────────────────────────────────

build_packages() {
    require_image "${TAG_PREFIX}:builder"
    mkdir -p "$PKG_OUTPUT"

    log "Building packages from source inside ${TAG_PREFIX}:builder"
    log "Output: $PKG_OUTPUT"

    if [ -n "$PKG_INPUT" ]; then
        log "Package: $PKG_INPUT"
    else
        log "Building all recipes in packages/core/"
    fi

    # Run builder container with repo mounted, output to host
    docker run --rm \
        --platform "$DOCKER_PLATFORM" \
        -v "$REPO_ROOT:/workspace:ro" \
        -v "$PKG_OUTPUT:/output" \
        -e "PKG_INPUT=${PKG_INPUT}" \
        -e "JOBS=${JOBS}" \
        -w /workspace \
        "${TAG_PREFIX}:builder" \
        -c '
set -e

# Configure jpkg
jpkg update 2>/dev/null || true

if [ -n "$PKG_INPUT" ]; then
    recipe_dir="/workspace/packages/core/${PKG_INPUT}"
    [ -f "${recipe_dir}/recipe.toml" ] || { echo "ERROR: no recipe at ${recipe_dir}"; exit 1; }
    echo "=== Building ${PKG_INPUT} ==="
    jpkg build "${recipe_dir}" --build-jpkg --output /output
else
    BUILT=0
    FAILED=0
    SKIPPED=0
    for recipe in /workspace/packages/core/*/recipe.toml; do
        pkg_dir="$(dirname "$recipe")"
        pkg_name="$(basename "$pkg_dir")"
        pkg_ver=$(grep "^version" "$recipe" | head -1 | sed "s/.*= *\"\(.*\)\"/\1/")
        arch=$(uname -m)
        if [ -f "/output/${pkg_name}-${pkg_ver}-${arch}.jpkg" ]; then
            echo "SKIP  ${pkg_name}-${pkg_ver} (already built)"
            SKIPPED=$((SKIPPED + 1))
            continue
        fi
        echo "=== Building ${pkg_name}-${pkg_ver} ==="
        if timeout 3600 jpkg build "${pkg_dir}" --build-jpkg --output /output 2>&1; then
            echo "OK    ${pkg_name}-${pkg_ver}"
            BUILT=$((BUILT + 1))
        else
            echo "FAIL  ${pkg_name}-${pkg_ver}"
            FAILED=$((FAILED + 1))
        fi
    done
    echo ""
    echo "=== Summary ==="
    echo "  Built:   $BUILT"
    echo "  Skipped: $SKIPPED"
    echo "  Failed:  $FAILED"
fi

echo ""
echo "Packages:"
ls -lhS /output/*.jpkg 2>/dev/null || echo "(none)"
'

    log "Packages written to: $PKG_OUTPUT"
    ls -lhS "$PKG_OUTPUT"/*.jpkg 2>/dev/null || log "(no .jpkg files)"
}

# ── Main ─────────────────────────────────────────────────────────────────

TARGETS="$*"
[ -z "$TARGETS" ] && TARGETS="all"

for target in $TARGETS; do
    case "$target" in
        minimal)  build_minimal ;;
        core)     build_core ;;
        builder)  build_builder ;;
        router)   build_router ;;
        packages) build_packages ;;
        all)
            build_minimal
            build_core
            build_builder
            ;;
        *)
            echo "Usage: $0 [minimal|core|builder|router|packages|all]"
            echo ""
            echo "Image chain:  minimal -> core -> builder"
            echo "                              -> router"
            echo ""
            echo "Build packages from source:"
            echo "  $0 packages                        # all recipes"
            echo "  PKG_INPUT=ruby $0 packages         # single package"
            exit 1
            ;;
    esac
done

log ""
log "Images built:"
for img in minimal core builder router; do
    if docker image inspect "${TAG_PREFIX}:${img}" >/dev/null 2>&1; then
        size=$(docker image inspect "${TAG_PREFIX}:${img}" --format '{{.Size}}' | awk '{printf "%.0fMB", $1/1048576}')
        log "  ${TAG_PREFIX}:${img}  (${size})"
    fi
done
log ""
log "Run:  docker run --rm -it ${TAG_PREFIX}:core"
