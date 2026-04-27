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

        # Install build-time dependencies (all permissive, build-time only).
        # nasm + mtools are for limine itself; cargo/rust/python3 are for
        # jpkg 2.0 (Rust port).
        apk add --no-cache clang lld musl-dev nasm mtools cargo rust python3

        # Build jpkg 2.0 from source so we can use it to build the Limine package
        cd /workspace/packages/core/jpkg
        TRIPLE=$(rustc -vV | sed -n "s/^host: //p")
        RUSTFLAGS="-C strip=symbols -C target-feature=+crt-static" \
            cargo build --release --locked --target "$TRIPLE" --bin jpkg --bin jpkg-local
        install -m 755 "target/$TRIPLE/release/jpkg" /usr/local/bin/jpkg

        # Build the Limine recipe
        jpkg build /workspace/packages/extra/limine --build-jpkg --output /output
    '

echo "Build complete. Package written to /tmp/jpkg-output/"
ls -lh /tmp/jpkg-output/limine-*.jpkg 2>/dev/null || true
