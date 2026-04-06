#!/bin/zsh
# ci-build-aarch64.sh â€” Run inside ghcr.io/stormj-uh/jonerix:all container
# Mounts: /workspace (repo), /var/cache/jpkg (output), /var/cache/jpkg-published, /jpkg-bin
# Env: PKG_INPUT (optional package name to force-build)
set -e

# Get jpkg binary: prefer host cache, then container pre-installed, then compile.
# The jonerix:all container COPYs packages/jpkg/jpkg â†’ /bin/jpkg at build time,
# so the pre-installed path is always available for containers built that way.
if [ -f /jpkg-bin/jpkg ]; then
    install -m 755 /jpkg-bin/jpkg /bin/jpkg
    echo "jpkg: using cached binary"
elif /bin/jpkg --version >/dev/null 2>&1; then
    echo "jpkg: using container pre-installed binary"
    cp /bin/jpkg /jpkg-bin/jpkg 2>/dev/null || true
else
    cd /workspace/packages/jpkg
    # Use clang directly â€” jonerix has bmake, not GNU make, so Makefile pattern
    # rules (%.o: %.c) don't work; compile all sources in one shot instead.
    # --rtlib=compiler-rt avoids GCC CRT (crtbeginS.o/libgcc) path dependency.
    clang -std=c11 -Os -fuse-ld=lld \
      -Wall -Wextra -Wpedantic -Werror=implicit-function-declaration \
      -Wno-unused-parameter -Wshadow -Wstrict-prototypes \
      -fstack-protector-strong \
      --rtlib=compiler-rt --unwindlib=none \
      -D_POSIX_C_SOURCE=200809L -D_DEFAULT_SOURCE \
      -o jpkg src/*.c
    install -m 755 jpkg /bin/jpkg
    cp jpkg /jpkg-bin/jpkg
fi

# Ensure the clang cfg exists (written by LLVM recipe; recreate if absent).
# NOTE: Alpine's clang 21 does NOT auto-load /etc/clang/<triple>.cfg â€”
# CLANG_CONFIG_FILE_SYSTEM_DIR is unset. cmake-based recipes must pass
# -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY and explicit linker flags
# to avoid the missing crtbeginS.o / libgcc failure.
CLANG_TRIPLE=$(clang -dumpmachine 2>/dev/null || echo "aarch64-jonerix-linux-musl")
CLANG_CFG="/etc/clang/${CLANG_TRIPLE}.cfg"
if [ ! -f "$CLANG_CFG" ]; then
    mkdir -p /etc/clang
    printf -- '--rtlib=compiler-rt\n--unwindlib=libunwind\n-fuse-ld=lld\n' > "$CLANG_CFG"
    echo "clang: created $CLANG_CFG"
fi

[ -f /lib/libssp_nonshared.a ] || printf '!<arch>\n' > /lib/libssp_nonshared.a

# Ensure bsdtar/tar is functional. Some published libarchive artifacts were built
# against OpenSSL 3 and require libcrypto.so.3, while jonerix ships LibreSSL. If the
# container's dynamic bsdtar is broken, restore the static fallback before jpkg tries
# to extract sources.
if ! bsdtar --version >/dev/null 2>&1; then
    if [ -x /workspace/tools/bsdtar-static-aarch64 ]; then
        install -m 755 /workspace/tools/bsdtar-static-aarch64 /bin/bsdtar
        echo "bsdtar: restored static fallback (dynamic libarchive artifact expects OpenSSL 3)"
    fi
fi

# Update package index
jpkg update

if [ -n "$PKG_INPUT" ]; then
    recipe_dir=""
    for d in core develop extra; do
      [ -f "/workspace/packages/$d/${PKG_INPUT}/recipe.toml" ] && recipe_dir="/workspace/packages/$d/${PKG_INPUT}" && break
    done
    [ -z "$recipe_dir" ] && { echo "ERROR: no recipe for ${PKG_INPUT} in packages/{core,develop,extra}"; exit 1; }
    echo "=== Building ${PKG_INPUT} (forced) ==="
    timeout 3600 jpkg build "${recipe_dir}" --build-jpkg --output /var/cache/jpkg || echo "FAILED: ${PKG_INPUT}"
else
    for recipe in /workspace/packages/core/*/recipe.toml /workspace/packages/develop/*/recipe.toml /workspace/packages/extra/*/recipe.toml; do
        [ -f "$recipe" ] || continue
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
