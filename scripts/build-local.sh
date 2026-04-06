#!/bin/sh
# build-local.sh — Build jonerix images locally from source
#
# Builds minimal and/or core images using Docker. All packages are built
# from source via jpkg inside the container — no prebuilt binaries pulled.
#
# Usage:
#   ./scripts/build-local.sh [minimal|core|all]
#
# Options:
#   minimal   Build jonerix:minimal only
#   core      Build jonerix:core only (requires minimal)
#   all       Build both (default)
#
# Environment:
#   PLATFORM    Target platform (default: auto-detect)
#   TAG_PREFIX  Image tag prefix (default: jonerix)
#   NO_CACHE    Set to 1 to disable Docker layer cache
#
# SPDX-License-Identifier: MIT

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Defaults
TARGET="${1:-all}"
PLATFORM="${PLATFORM:-$(docker info --format '{{.Architecture}}' 2>/dev/null || uname -m)}"
TAG_PREFIX="${TAG_PREFIX:-jonerix}"
CACHE_FLAG=""
[ "${NO_CACHE}" = "1" ] && CACHE_FLAG="--no-cache"

# Normalize platform for Docker
case "$PLATFORM" in
    x86_64|amd64)  DOCKER_PLATFORM="linux/amd64" ;;
    aarch64|arm64) DOCKER_PLATFORM="linux/arm64" ;;
    *) echo "Unknown platform: $PLATFORM"; exit 1 ;;
esac

log() {
    printf "\033[1;34m[build-local]\033[0m %s\n" "$*"
}

err() {
    printf "\033[1;31m[build-local] ERROR:\033[0m %s\n" "$*" >&2
    exit 1
}

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
    # Verify minimal exists
    if ! docker image inspect "${TAG_PREFIX}:minimal" >/dev/null 2>&1; then
        err "${TAG_PREFIX}:minimal not found. Build minimal first: $0 minimal"
    fi

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

case "$TARGET" in
    minimal)
        build_minimal
        ;;
    core)
        build_core
        ;;
    all)
        build_minimal
        build_core
        ;;
    *)
        echo "Usage: $0 [minimal|core|all]"
        exit 1
        ;;
esac

log "Build complete."
log "  ${TAG_PREFIX}:minimal — base rootfs (toybox, dropbear, openrc)"
log "  ${TAG_PREFIX}:core    — core packages (micro, ripgrep, gitoxide, ...)"
log ""
log "Run:  docker run --rm -it ${TAG_PREFIX}:core"
