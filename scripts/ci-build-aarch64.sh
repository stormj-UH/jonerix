#!/bin/zsh
# ci-build-aarch64.sh — Run inside ghcr.io/stormj-uh/jonerix:all container
# Mounts: /workspace (repo), /var/cache/jpkg (output), /var/cache/jpkg-published, /jpkg-bin
# Env: PKG_INPUT (optional package name to force-build)
set -e

# Build jpkg from source (source of truth — do not trust container binary)
if [ -f /jpkg-bin/jpkg ]; then
    install -m 755 /jpkg-bin/jpkg /bin/jpkg
    echo "jpkg: using cached binary"
else
    cd /workspace/packages/jpkg
    # Use clang directly — jonerix has bmake, not GNU make, so Makefile pattern
    # rules (%.o: %.c) don't work; compile all sources in one shot instead.
    # Build dynamically — jonerix's LLVM ships crtbeginS.o (shared) but not
    # crtbeginT.o (static-PIE), so -static fails. Dynamic is fine inside the
    # container since musl is present.
    clang -std=c11 -Os -fuse-ld=lld \
      -Wall -Wextra -Wpedantic -Werror=implicit-function-declaration \
      -Wno-unused-parameter -Wshadow -Wstrict-prototypes \
      -fstack-protector-strong \
      -D_POSIX_C_SOURCE=200809L -D_DEFAULT_SOURCE \
      -o jpkg src/*.c
    install -m 755 jpkg /bin/jpkg
    cp jpkg /jpkg-bin/jpkg
fi

# Update package index
jpkg update

if [ -n "$PKG_INPUT" ]; then
    recipe_dir="/workspace/packages/core/${PKG_INPUT}"
    [ -f "${recipe_dir}/recipe.toml" ] || { echo "ERROR: no recipe at ${recipe_dir}/recipe.toml"; exit 1; }
    echo "=== Building ${PKG_INPUT} (forced) ==="
    timeout 1800 jpkg build "${recipe_dir}" --build-jpkg --output /var/cache/jpkg || echo "FAILED: ${PKG_INPUT}"
else
    for recipe in /workspace/packages/core/*/recipe.toml; do
        pkg_dir="$(dirname "$recipe")"
        pkg_name="$(basename "$pkg_dir")"
        pkg_ver=$(grep "^version" "${pkg_dir}/recipe.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
        expected="/var/cache/jpkg-published/${pkg_name}-${pkg_ver}-aarch64.jpkg"
        legacy="/var/cache/jpkg-published/${pkg_name}-${pkg_ver}.jpkg"
        if [ -f "$expected" ] || [ -f "$legacy" ]; then
            echo "=== Skipping ${pkg_name}-${pkg_ver} (already published) ==="
            continue
        fi
        echo "=== Building ${pkg_name} ==="
        timeout 1200 jpkg build "${pkg_dir}" --build-jpkg --output /var/cache/jpkg || echo "FAILED: ${pkg_name}"
    done
fi

echo "Built/cached packages (aarch64):"
ls -lh /var/cache/jpkg/*.jpkg 2>/dev/null || echo "(none)"
