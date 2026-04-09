#!/bin/sh
# Build LLVM from source with libc++ on jonerix (aarch64)
#
# Produces: .build/packages-aarch64/llvm-21.1.2-aarch64.jpkg
# Time: ~45-60 min on M4 Mac
# Memory: needs 12GB+ Docker memory
#
# Prerequisites:
#   - jonerix:all image (ghcr.io/stormj-uh/jonerix:all)
#   - libcxx jpkg uploaded to GitHub
#   - packages/jpkg/ source in repo
#
# Usage:
#   sh scripts/build-llvm-libcxx.sh

set -e

REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
OUTPUT_DIR="$REPO_DIR/.build/packages-aarch64"
IMAGE="ghcr.io/stormj-uh/jonerix:all"

mkdir -p "$OUTPUT_DIR"

echo "=== Building LLVM 21.1.2 with libc++ on jonerix (aarch64) ==="
echo "Image: $IMAGE"
echo "Output: $OUTPUT_DIR"
echo "This will take 45-60 minutes..."
echo ""

docker run --rm \
    --entrypoint /bin/zsh \
    --memory=12g \
    --platform linux/arm64 \
    -v "$REPO_DIR:/workspace:ro" \
    -v "$OUTPUT_DIR:/output" \
    "$IMAGE" \
    -c '
set -e

echo "Building jpkg from source..."
cd /workspace/packages/jpkg
samu clean 2>/dev/null || true
samu && install -m 755 jpkg /bin/jpkg
cd /

echo "Installing libc++..."
jpkg update
jpkg install libcxx || {
    echo "jpkg install failed, trying direct download..."
    curl -fsSL "https://github.com/stormj-UH/jonerix/releases/download/packages/libcxx-21.1.2-aarch64.jpkg" -o /tmp/libcxx.jpkg
    jpkg install /tmp/libcxx.jpkg 2>/dev/null || {
        # Manual extract
        cd /tmp
        python3 -c "d=open(\"libcxx.jpkg\",\"rb\").read();i=d.find(b\"\\x28\\xb5\\x2f\\xfd\");open(\"lc.zst\",\"wb\").write(d[i:])"
        zstd -d lc.zst -o lc.tar 2>/dev/null
        cd / && bsdtar xf /tmp/lc.tar 2>/dev/null
        echo "libc++ manually extracted"
    }
}

echo "Verifying libc++..."
ls /lib/libc++.so* /lib/libunwind.so* || { echo "FATAL: libc++ not found"; exit 1; }

echo "Building LLVM..."
jpkg build /workspace/packages/develop/llvm --build-jpkg --output /output 2>&1

echo ""
echo "=== LLVM build complete ==="
ls -lh /output/llvm-*.jpkg
'

echo ""
echo "=== Done ==="
echo "Upload with:"
echo "  gh release upload packages $OUTPUT_DIR/llvm-21.1.2-aarch64.jpkg --repo stormj-UH/jonerix --clobber"
