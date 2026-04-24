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
CLANG_TRIPLE=$(clang -dumpmachine 2>/dev/null || echo "aarch64-jonerix-linux-musl")
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
    if [ -x /workspace/tools/bsdtar-static-aarch64 ]; then
        install -m 755 /workspace/tools/bsdtar-static-aarch64 /bin/bsdtar
        echo "bsdtar: restored static fallback (dynamic libarchive artifact expects OpenSSL 3)"
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

install_cached_pkg_if_available() {
    pkg="$1"
    cached_pkg=$(ls /var/cache/jpkg/${pkg}-*-*.jpkg 2>/dev/null | sort -V | tail -1)
    if [ -z "$cached_pkg" ] || [ ! -f "$cached_pkg" ]; then
        return 1
    fi
    echo "=== Installing cached ${pkg}: $(basename "$cached_pkg") ==="
    hdr_len=$(od -An -v -tu4 -N4 -j8 "$cached_pkg" | tr -d ' ')
    skip=$((12 + hdr_len))
    tail -c +$((skip + 1)) "$cached_pkg" | zstd -dc | tar xf - -C /
    return 0
}

have_working_expr() {
    /bin/expr 1 + 1 >/dev/null 2>&1 &&
    /bin/expr length expr >/dev/null 2>&1 &&
    /bin/expr xexpr : 'x\(.*\)' >/dev/null 2>&1
}

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

# Ensure jmake is the latest version. `jpkg install --force` only
# reinstalls the currently-installed version — it does NOT upgrade.
# Use `jpkg upgrade jmake` to pull the newest from INDEX (needed for
# every jmake fix since 1.0.4: VPATH expansion (1.0.5), GNUmakefile
# preference (1.0.6), ifeq-colon dispatch (1.0.7)). Critical for
# Python 3.14 build perf AND Ruby 3.4 configure's GNU-make detection.
if install_cached_pkg_if_available jmake; then
    ln -sf jmake /bin/make 2>/dev/null || true
else
    jpkg upgrade jmake 2>&1 | tail -5
fi

# exproxide — clean-room Rust `expr(1)`. Autoconf-generated
# configure scripts call `expr` at probe time (unbound, libevent,
# and anything else with `expr "x$FOO"` checks); toybox's expr
# applet is not compatible enough for those builders, so flip
# /bin/expr to exproxide up front and fail loudly if it regresses.
echo "=== Installing exproxide for /bin/expr ==="
if install_cached_pkg_if_available exproxide; then
    ln -sf exproxide /bin/expr 2>/dev/null || true
else
    jpkg install --force exproxide 2>&1 | tail -5 || echo "exproxide install failed"
fi
if ! have_working_expr; then
    echo "FATAL: exproxide failed the minimal autoconf expr probe set"
    exit 1
fi

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
        llvm) echo 14400 ;;
        *) echo 3600 ;;
    esac
}

install_local_jpkg() {
    # Extract a jpkg file directly into / — needed when a build dep
    # was rebuilt earlier in this same CI run and therefore isn't
    # yet in the release INDEX. jpkg install only accepts package
    # names via INDEX, so we replicate its tail half here.
    # Format (packages/jpkg/src/pkg.h): 8B magic + 4B LE header_len
    # + TOML header + zstd-compressed tar payload.
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
        # Library packages — always install via jpkg, even if a
        # namesake binary is on PATH (Alpine's xz/bzip2/zstd/curl
        # satisfy `command -v` but don't put headers at /include).
        # python3 build failed 2026-04-20 run 24687524373 on
        # `lzma.h not found` because Alpine's xz short-circuited
        # the install of the jonerix xz jpkg (which ships
        # /include/lzma.h). Recognise these by name so they
        # always get jpkg-installed.
        case "$dep" in
            xz|bzip2|zstd|zlib|lz4|ncurses|pcre2|libffi|sqlite|\
            libressl|libarchive|libevent|libcxx|nloxide|curl|expat|\
            jonerix-headers)
                is_library_pkg=1
                ;;
            *)
                is_library_pkg=0
                ;;
        esac

        if [ "$is_library_pkg" = 0 ] && command -v "$dep" >/dev/null 2>&1; then
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

        # Prefer a just-built jpkg in /var/cache/jpkg over whatever is
        # in INDEX. Uploads to the release happen at end-of-job, so a
        # dep rebuilt earlier in the same run isn't yet indexed; a
        # plain `jpkg install --force <name>` would pull the stale
        # released version (run 24682739978: nloxide-r3 built, but
        # wpa_supplicant got r2 and failed). `jpkg install` only
        # takes package names, so for local-cache hits we extract
        # the jpkg directly via install_local_jpkg.
        # Pick the highest-sorting version; sort -V so r10 > r2.
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

    # Arch gate: a recipe can pin itself to one arch via
    #   [package]
    #   arch = "aarch64"
    # which jpkg already honors at packaging time. Skip the build
    # entirely when the pinned arch doesn't match this runner —
    # otherwise hardware-specific recipes (jonerix-raspi5-fixups'
    # aarch64 inline asm, etc.) fail at compile on the wrong arch.
    pkg_arch=$(grep "^arch" "${pkg_dir}/recipe.toml" | head -1 \
        | sed 's/.*= *"\(.*\)"/\1/')
    if [ -n "$pkg_arch" ] && [ "$pkg_arch" != "aarch64" ]; then
        echo "=== Skipping ${pkg_name}-${pkg_ver} (arch=${pkg_arch}, runner=aarch64) ==="
        return 0
    fi

    # License gate: GPL recipes are kept in-tree purely for header
    # reference (Linux kernel UAPI) and build-system documentation.
    # jpkg's own license gate blocks them; running `jpkg build` just
    # to hit that block pollutes CI logs with a spurious `FAILED:`
    # line. Skip here with a clear message instead.
    pkg_license=$(grep "^license" "${pkg_dir}/recipe.toml" | head -1 \
        | sed 's/.*= *"\(.*\)"/\1/')
    case "$pkg_license" in
        GPL-*|LGPL-*|AGPL-*)
            echo "=== Skipping ${pkg_name}-${pkg_ver} (license=${pkg_license}, blocked by permissive-only policy; in-tree for headers/reference only) ==="
            return 0
            ;;
    esac

    # Operational skip list — packages whose off-CI-built jpkg works
    # fine and whose CI rebuild is expensive (nodejs' v8 takes 20-30
    # min) or flaky. Accepts any published version of the package,
    # not just the recipe's current version. Bypass via
    # `REBUILD_INPUT=true` or a single-package `PKG_INPUT` dispatch.
    if [ "${REBUILD_INPUT:-false}" != "true" ] && [ -z "$PKG_INPUT" ]; then
        case "$pkg_name" in
            nodejs|rust|llvm)
                if ls /var/cache/jpkg-published/${pkg_name}-*-aarch64.jpkg >/dev/null 2>&1; then
                    echo "=== Skipping ${pkg_name} (NO_CI_REBUILD — existing jpkg works fine) ==="
                    return 0
                fi
                ;;
        esac
    fi

    expected="/var/cache/jpkg-published/${pkg_name}-${pkg_ver}-aarch64.jpkg"
    legacy="/var/cache/jpkg-published/${pkg_name}-${pkg_ver}.jpkg"

    if [ "${REBUILD_INPUT:-false}" != "true" ] && { [ -f "$expected" ] || [ -f "$legacy" ]; }; then
        echo "=== Skipping ${pkg_name}-${pkg_ver} (already published) ==="
        return 0
    fi

    # Install the recipe's declared build deps BEFORE invoking jpkg
    # build. Without this, packages that need header-only deps like
    # jonerix-headers (Linux UAPI), nloxide (netlink C headers), or
    # libressl (TLS headers) fail mid-compile with missing-header
    # errors. The single-package `PKG_INPUT` branch below already
    # calls install_target_build_deps; the full-run loop did not,
    # which is why every full publish-packages run since at least
    # 2026-04-20 lost nodejs, wpa_supplicant, hostapd, dhcpcd, and
    # friends to this pattern.
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
    expected="/var/cache/jpkg-published/${PKG_INPUT}-${pkg_ver}-aarch64.jpkg"
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

echo "Built/cached packages (aarch64):"
ls -lh /var/cache/jpkg/*.jpkg 2>/dev/null || echo "(none)"

[ "$failures" -eq 0 ]
