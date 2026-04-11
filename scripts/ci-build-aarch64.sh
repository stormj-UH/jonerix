#!/bin/sh
# ci-build-aarch64.sh — Run inside ghcr.io/stormj-uh/jonerix:builder-arm64 container
# Mounts: /workspace (repo), /var/cache/jpkg (output), /var/cache/jpkg-published, /jpkg-bin
# Env: PKG_INPUT (optional package name to target), REBUILD_INPUT (optional boolean)
set -e

# Get jpkg binary: prefer host cache, then compile from source.
# Always build from source on cache miss so the binary matches the current
# repo checkout (important for new features like RECIPE_DIR).
if [ -f /jpkg-bin/jpkg ]; then
    install -m 755 /jpkg-bin/jpkg /bin/jpkg
    echo “jpkg: using cached binary”
else
    cd /workspace/packages/jpkg
    clang -std=c11 -Os -fuse-ld=lld \
      -Wall -Wextra -Wpedantic -Werror=implicit-function-declaration \
      -Wno-unused-parameter -Wshadow -Wstrict-prototypes \
      -fstack-protector-strong \
      --rtlib=compiler-rt --unwindlib=none \
      -D_POSIX_C_SOURCE=200809L -D_DEFAULT_SOURCE \
      -o jpkg src/*.c
    install -m 755 jpkg /bin/jpkg
    cp jpkg /jpkg-bin/jpkg
    cd /workspace
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

# Meson is bootstrapped from its upstream source tarball to avoid depending
# on pip/SSL support in older builder images.
/workspace/scripts/bootstrap-meson.sh

ensure_bootstrap_llvm() {
    if [ -x /bin/clang-21 ] && { [ -x /bin/clang++-21 ] || [ -x /bin/clang++ ]; } && [ -x /bin/ld.lld ]; then
        return 0
    fi
    echo "cmake: repairing stale LLVM toolchain from published packages"
    jpkg install --force llvm
    if [ ! -x /bin/clang++-21 ] && [ -x /bin/clang++ ]; then
        ln -sf clang++ /bin/clang++-21
    fi
    if [ ! -x /bin/clang-21 ] || { [ ! -x /bin/clang++-21 ] && [ ! -x /bin/clang++ ]; } || [ ! -x /bin/ld.lld ]; then
        echo "FATAL: llvm package repair did not restore clang-21/clang++/ld.lld"
        exit 1
    fi
}

ensure_bootstrap_cmake_pkg() {
    if cmake --version >/dev/null 2>&1; then
        return 0
    fi
    echo "cmake: repairing stale cmake from published package"
    if jpkg install --force cmake >/dev/null 2>&1 && cmake --version >/dev/null 2>&1; then
        return 0
    fi
    return 1
}

# Some older published builder images have a broken packaged cmake and no apk.
# Build a temporary bootstrap cmake from the vendored source tarball, then use
# it to rebuild the final cmake package cleanly.
if [ "${PKG_INPUT:-}" = "cmake" ]; then
    if ensure_bootstrap_cmake_pkg; then
        export BOOTSTRAP_CMAKE=cmake
        echo "cmake: using published package bootstrap"
    else
        ensure_bootstrap_llvm
        export BOOTSTRAP_CMAKE=$(/workspace/scripts/bootstrap-cmake.sh)
        "$BOOTSTRAP_CMAKE" --version >/dev/null 2>&1 || {
            echo "FATAL: bootstrap cmake is unusable"
            exit 1
        }
        echo "cmake: using local bootstrap binary at $BOOTSTRAP_CMAKE"
    fi
elif ! cmake --version >/dev/null 2>&1; then
    if command -v apk >/dev/null 2>&1; then
        broken_cmake=$(command -v cmake 2>/dev/null || true)
        if [ -n "$broken_cmake" ] && [ -e "$broken_cmake" ]; then
            mv "$broken_cmake" "${broken_cmake}.broken" 2>/dev/null || true
        fi
        apk add --no-cache cmake >/dev/null
        cmake --version >/dev/null 2>&1 || {
            echo "FATAL: bootstrap cmake is still unusable"
            exit 1
        }
        echo "cmake: installed temporary bootstrap copy from Alpine packages"
    fi
fi

failures=0

build_one() {
    recipe="$1"
    pkg_dir="$(dirname "$recipe")"
    pkg_name="$(basename "$pkg_dir")"
    pkg_ver=$(grep "^version" "${pkg_dir}/recipe.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
    expected="/var/cache/jpkg-published/${pkg_name}-${pkg_ver}-aarch64.jpkg"
    legacy="/var/cache/jpkg-published/${pkg_name}-${pkg_ver}.jpkg"

    if [ "${REBUILD_INPUT:-false}" != "true" ] && { [ -f "$expected" ] || [ -f "$legacy" ]; }; then
        echo "=== Skipping ${pkg_name}-${pkg_ver} (already published) ==="
        return 0
    fi

    echo "=== Building ${pkg_name} ==="
    if ! timeout 1200 jpkg build "${pkg_dir}" --build-jpkg --output /var/cache/jpkg; then
        echo "FAILED: ${pkg_name}"
        failures=$((failures + 1))
    fi
}

if [ -n "$PKG_INPUT" ]; then
    recipe_dir=""
    for d in core develop extra; do
      [ -f "/workspace/packages/$d/${PKG_INPUT}/recipe.toml" ] && recipe_dir="/workspace/packages/$d/${PKG_INPUT}" && break
    done
    [ -z "$recipe_dir" ] && { echo "ERROR: no recipe for ${PKG_INPUT} in packages/{core,develop,extra}"; exit 1; }
    pkg_ver=$(grep "^version" "${recipe_dir}/recipe.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
    expected="/var/cache/jpkg-published/${PKG_INPUT}-${pkg_ver}-aarch64.jpkg"
    legacy="/var/cache/jpkg-published/${PKG_INPUT}-${pkg_ver}.jpkg"
    if [ "${REBUILD_INPUT:-false}" != "true" ] && { [ -f "$expected" ] || [ -f "$legacy" ]; }; then
        echo "=== Skipping ${PKG_INPUT}-${pkg_ver} (already published) ==="
    else
        [ "${REBUILD_INPUT:-false}" = "true" ] && echo "=== Rebuilding ${PKG_INPUT} ===" || echo "=== Building ${PKG_INPUT} ==="
        if [ "$PKG_INPUT" = "llvm" ]; then
            echo "=== Repacking llvm from builder image state ==="
            if ! timeout 1200 /workspace/scripts/repack-installed-package.sh llvm /var/cache/jpkg "${recipe_dir}"; then
                echo "llvm repack failed, falling back to source build"
                if ! timeout 3600 jpkg build "${recipe_dir}" --build-jpkg --output /var/cache/jpkg; then
                    echo "FAILED: ${PKG_INPUT}"
                    failures=$((failures + 1))
                fi
            fi
        elif ! timeout 3600 jpkg build "${recipe_dir}" --build-jpkg --output /var/cache/jpkg; then
            echo "FAILED: ${PKG_INPUT}"
            failures=$((failures + 1))
        fi
    fi
else
    for recipe in /workspace/packages/core/*/recipe.toml /workspace/packages/develop/*/recipe.toml /workspace/packages/extra/*/recipe.toml; do
        [ -f "$recipe" ] || continue
        build_one "$recipe"
    done
fi

echo "Built/cached packages (aarch64):"
ls -lh /var/cache/jpkg/*.jpkg 2>/dev/null || echo "(none)"

[ "$failures" -eq 0 ]
