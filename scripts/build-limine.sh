#!/bin/sh
# build-limine.sh — build the Limine jpkg package inside an Alpine container.
#
# Usage:
#   ./scripts/build-limine.sh
#
# Output:
#   /tmp/jpkg-output/limine-*.jpkg
#
# Why Alpine? Limine's build requires GNU make and nasm. Both are GPL/BSD
# build-time tools that are acceptable in the Alpine build environment but
# must never appear in the jonerix rootfs.

set -e

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

docker run --rm \
    -v "$REPO_ROOT:/workspace" \
    -v /tmp/jpkg-output:/output \
    alpine:latest sh -c '
        set -e

        # Install build-time dependencies (all GPL/BSD, build-time only)
        apk add --no-cache clang lld musl-dev make nasm samurai zstd-dev mtools

        # Build jpkg from source so we can use it to build the Limine package
        cd /workspace/packages/jpkg
        samu clean 2>/dev/null || true
        samu

        install -m755 jpkg /usr/local/bin/jpkg

        # Build the Limine recipe
        jpkg build /workspace/packages/extra/limine --build-jpkg --output /output
    '

echo "Build complete. Package written to /tmp/jpkg-output/"
ls -lh /tmp/jpkg-output/limine-*.jpkg 2>/dev/null || true
