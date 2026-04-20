#!/bin/sh
# ci-build-x86_64.sh — Run inside ghcr.io/stormj-uh/jonerix:builder-amd64 container
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
      -D_POSIX_C_SOURCE=200809L -D_DEFAULT_SOURCE -DJPKG_USE_LIBRESSL \
      -o jpkg src/*.c \
      -ltls -lssl -lcrypto -lzstd
    install -m 755 jpkg /bin/jpkg
    cp jpkg /jpkg-bin/jpkg
    cd /workspace
fi

# Ensure the clang cfg exists (written by LLVM recipe; recreate if absent).
# NOTE: Alpine's clang 21 does NOT auto-load /etc/clang/<triple>.cfg â€”
# CLANG_CONFIG_FILE_SYSTEM_DIR is unset. cmake-based recipes must pass
# -DCMAKE_TRY_COMPILE_TARGET_TYPE=STATIC_LIBRARY and explicit linker flags
# to avoid the missing crtbeginS.o / libgcc failure.
CLANG_TRIPLE=$(clang -dumpmachine 2>/dev/null || echo "x86_64-jonerix-linux-musl")
CLANG_CFG="/etc/clang/${CLANG_TRIPLE}.cfg"
if [ ! -f "$CLANG_CFG" ]; then
    mkdir -p /etc/clang
    printf -- '--rtlib=compiler-rt\n--unwindlib=libunwind\n-fuse-ld=lld\n' > "$CLANG_CFG"
    echo "clang: created $CLANG_CFG"
fi

[ -f /lib/libssp_nonshared.a ] || printf '!<arch>\n' > /lib/libssp_nonshared.a

# Ensure GCC compat symlinks exist (cargo/rustc need libgcc_s.so.1 for unwinding)
[ -f /lib/libgcc_s.so.1 ] || ln -sf libunwind.so.1 /lib/libgcc_s.so.1 2>/dev/null || true
[ -f /lib/libstdc++.so.6 ] || ln -sf libc++.so.1 /lib/libstdc++.so.6 2>/dev/null || true

# Ensure bsdtar/tar is functional. Some published libarchive artifacts were built
# against OpenSSL 3 or older lz4 sonames, while jonerix ships LibreSSL and the
# current lz4 package set. Repair the stale tool before jpkg or package recipes try
# to extract sources.
if ! bsdtar --version >/dev/null 2>&1; then
    if [ -x /workspace/tools/bsdtar-static-x86_64 ]; then
        install -m 755 /workspace/tools/bsdtar-static-x86_64 /bin/bsdtar
        echo "bsdtar: restored static fallback (dynamic libarchive artifact expects OpenSSL 3)"
    elif command -v apk >/dev/null 2>&1; then
        apk add --no-cache libarchive-tools 2>/dev/null || true
        echo "bsdtar: installed from Alpine packages"
    fi
fi

if ! tar --version >/dev/null 2>&1; then
    if bsdtar --version >/dev/null 2>&1; then
        ln -sf /bin/bsdtar /bin/tar
        echo "tar: linked to bsdtar fallback"
    elif command -v toybox >/dev/null 2>&1; then
        ln -sf /bin/toybox /bin/tar
        echo "tar: linked to toybox fallback"
    fi
fi

if ! bsdtar --version >/dev/null 2>&1 && \
   ! /bin/toybox tar --help >/dev/null 2>&1 && \
   ! tar --version >/dev/null 2>&1; then
    echo "FATAL: no usable tar implementation found"
    exit 1
fi

if [ -z "${JPKG_SOURCE_CACHE:-}" ] && [ -d /workspace/sources ]; then
    export JPKG_SOURCE_CACHE=/workspace/sources
fi

# /bin/expr shim: jonerix is permissive-only and toybox lacks expr;
# many autoconf-vintage configures (libffi, python3, openntpd, sudo,
# jq, pico, tzdata, libevent, byacc) loop forever without it. Install
# once here so every recipe can rely on it. Source lives in scripts/
# to avoid per-recipe duplication / heredoc hell.
if [ ! -x /bin/expr ]; then
    echo "== installing /bin/expr shim =="
    clang -Os --rtlib=compiler-rt --unwindlib=libunwind -fuse-ld=lld \
        -o /bin/expr /workspace/scripts/expr-shim.c
fi

# /bin/install GNU-compat shim: toybox install lacks -c (GNU's "copy"
# flag, which is the default on toybox anyway — just needs to be
# stripped before toybox parses argv). autoconf-generated Makefiles
# emit `install -c -m 644 src dst`, and toybox parses `-c` as the
# destination. Reproduced on Python 3.14.3 build 2026-04-17.
echo "== installing /bin/install GNU-compat shim =="
clang -Os --rtlib=compiler-rt --unwindlib=libunwind -fuse-ld=lld \
    -o /bin/install /workspace/scripts/install-shim.c

# Update package index
jpkg update

# Ensure jmake is at least 1.0.4 (adds find_pattern_rule memoization;
# without it Python 3.14's make install hangs indefinitely on
# ./Include/**/*.h lookups). The builder image may ship an older jmake;
# force-install from the INDEX so we always have the perf fix.
jpkg upgrade jmake 2>&1 | tail -5

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

package_timeout() {
    case "$1" in
        llvm) echo 20000 ;;
        *) echo 3600 ;;
    esac
}

install_local_jpkg() {
    # Extract a jpkg file directly into / — see aarch64 sibling for
    # rationale. jpkg format: 8B magic + 4B LE header_len + TOML +
    # zstd tar payload.
    local f="$1"
    local hdr_len skip
    hdr_len=$(od -An -v -tu4 -N4 -j8 "$f" | tr -d ' ')
    skip=$((12 + hdr_len))
    tail -c +$((skip + 1)) "$f" | zstd -dc | tar xf - -C /
}

install_target_build_deps() {
    recipe_dir="$1"
    deps_line=$(awk '
        $0 == "[depends]" { in_dep = 1; next }
        /^\[/ { if (in_dep) exit }
        in_dep && $1 == "build" { print; exit }
    ' "$recipe_dir/recipe.toml")

    [ -z "$deps_line" ] && return 0

    deps=$(printf '%s\n' "$deps_line" |
        sed -E 's/.*\[(.*)\].*/\1/' |
        sed 's/"//g' |
        sed 's/,/ /g')

    for dep in $deps; do
        [ -n "$dep" ] || continue
        # Clear the known-broken byacc symlink loop baked into the builder
        # image (byacc -> yacc -> byacc, neither resolves to a real file)
        # before dep-check so we actually reinstall byacc from the package.
        if [ "$dep" = "byacc" ] && [ -L /bin/yacc ] && [ ! -e /bin/yacc ]; then
            echo "byacc: clearing stale symlink loop from builder image"
            rm -f /bin/yacc /bin/byacc
        fi
        if command -v "$dep" >/dev/null 2>&1; then
            continue
        fi

        dep_pkg="$dep"
        case "$dep" in
            clang|clang++|ld.lld|llvm-ar|llvm-ranlib|llvm-nm|llvm-strip)
                dep_pkg=llvm
                ;;
            make)
                dep_pkg=jmake
                ;;
            python)
                dep_pkg=python3
                ;;
            rust)
                # rust package provides cargo/rustc, not a 'rust' binary
                command -v cargo >/dev/null 2>&1 && continue
                ;;
            jonerix-headers)
                # header-only package, no binary to check
                dep_pkg=jonerix-headers
                ;;
        esac

        # Prefer a just-built jpkg in /var/cache/jpkg over INDEX. See
        # aarch64 sibling for rationale. jpkg install only takes
        # names, so local hits go through install_local_jpkg.
        local_pkg=$(ls /var/cache/jpkg/${dep_pkg}-*-*.jpkg 2>/dev/null \
            | sort -V | tail -1)
        if [ -n "$local_pkg" ] && [ -f "$local_pkg" ]; then
            echo "=== Ensuring build dependency: ${dep_pkg} (for ${dep}) — extracting local $(basename "$local_pkg") ==="
            install_local_jpkg "$local_pkg"
        else
            echo "=== Ensuring build dependency: ${dep_pkg} (for ${dep}) ==="
            jpkg install --force "$dep_pkg"
        fi
    done
}

build_one() {
    recipe="$1"
    pkg_dir="$(dirname "$recipe")"
    pkg_name="$(basename "$pkg_dir")"
    pkg_ver=$(grep "^version" "${pkg_dir}/recipe.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')

    # Arch gate — see aarch64 sibling for the full rationale.
    pkg_arch=$(grep "^arch" "${pkg_dir}/recipe.toml" | head -1 \
        | sed 's/.*= *"\(.*\)"/\1/')
    if [ -n "$pkg_arch" ] && [ "$pkg_arch" != "x86_64" ]; then
        echo "=== Skipping ${pkg_name}-${pkg_ver} (arch=${pkg_arch}, runner=x86_64) ==="
        return 0
    fi

    expected="/var/cache/jpkg-published/${pkg_name}-${pkg_ver}-x86_64.jpkg"
    legacy="/var/cache/jpkg-published/${pkg_name}-${pkg_ver}.jpkg"

    if [ "${REBUILD_INPUT:-false}" != "true" ] && { [ -f "$expected" ] || [ -f "$legacy" ]; }; then
        echo "=== Skipping ${pkg_name}-${pkg_ver} (already published) ==="
        return 0
    fi

    # Install the recipe's declared build deps before invoking jpkg.
    # Same reasoning as the aarch64 sibling — without this, header-
    # only build deps (jonerix-headers, nloxide, libressl) never get
    # installed and dependent packages blow up with missing-header
    # errors mid-compile.
    install_target_build_deps "$pkg_dir"

    echo "=== Building ${pkg_name} ==="
    if ! timeout "$(package_timeout "$pkg_name")" jpkg build "${pkg_dir}" --build-jpkg --output /var/cache/jpkg; then
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
    install_target_build_deps "$recipe_dir"
    pkg_ver=$(grep "^version" "${recipe_dir}/recipe.toml" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
    expected="/var/cache/jpkg-published/${PKG_INPUT}-${pkg_ver}-x86_64.jpkg"
    legacy="/var/cache/jpkg-published/${PKG_INPUT}-${pkg_ver}.jpkg"
    if [ "${REBUILD_INPUT:-false}" != "true" ] && { [ -f "$expected" ] || [ -f "$legacy" ]; }; then
        echo "=== Skipping ${PKG_INPUT}-${pkg_ver} (already published) ==="
    else
        [ "${REBUILD_INPUT:-false}" = "true" ] && echo "=== Rebuilding ${PKG_INPUT} ===" || echo "=== Building ${PKG_INPUT} ==="
        if ! timeout "$(package_timeout "$PKG_INPUT")" jpkg build "${recipe_dir}" --build-jpkg --output /var/cache/jpkg; then
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

echo "Built/cached packages (x86_64):"
ls -lh /var/cache/jpkg/*.jpkg 2>/dev/null || echo "(none)"

[ "$failures" -eq 0 ]
