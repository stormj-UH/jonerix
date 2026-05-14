#!/bin/sh
# local-build-x86_64.sh — Local hedge builder for x86_64.
#
# Mirrors local-build-aarch64.sh. Intended to run on castle (Ryzen 5 1600,
# 6c/12t, 15 GB RAM, gentoo, docker). The host has plenty of CPU but tight
# RAM, so we cap LLVM_BUILD_JOBS / BUILD_JOBS at 3 (overridable via the
# JOBS env var) to keep the libllvm/clang compile from OOMing.
#
# Usage:
#   ./scripts/local-build-x86_64.sh build PKG [PKG...]
#   ./scripts/local-build-x86_64.sh chain   # libllvm → clang → lld → llvm → llvm-extra
#   ./scripts/local-build-x86_64.sh chain22 # libcxx22 -> libllvm22 -> clang22 -> lld22 -> llvm22 -> llvm22-extra
#   ./scripts/local-build-x86_64.sh upload  # push winning .jpkgs to GitHub release
#   ./scripts/local-build-x86_64.sh status  # what's in the local hedge cache
#
# Once a build finishes we upload the .jpkg(s) to the `packages` release
# on GitHub, then trigger the regen-tag-index workflow so the freshly-
# uploaded asset gets pulled into a signed INDEX.zst.
#
# SPDX-License-Identifier: MIT

set -eu

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
BUILD_DIR="${REPO_ROOT}/.local-build"
JPKG_OUTPUT="${BUILD_DIR}/jpkg-output"
JPKG_PUBLISHED="${BUILD_DIR}/jpkg-published"
JPKG_BIN="${BUILD_DIR}/jpkg-bin-x86_64"
SCCACHE="${BUILD_DIR}/sccache-cache"
SCCACHE_BIN="${BUILD_DIR}/sccache-bin/sccache-x86_64"

BUILDER_IMAGE="${BUILDER_IMAGE:-ghcr.io/stormj-uh/jonerix:builder}"
GITHUB_REPO="${GITHUB_REPO:-stormj-UH/jonerix}"
RELEASE_TAG="${RELEASE_TAG:-packages}"

# Default to -j3: AMD Ryzen 5 1600 has 12 threads but 15 GB RAM caps the
# LLVM compile at ~4 GB peak per CC1 worker. -j3 fits comfortably; -j4
# risks OOM during clang/Sema and libllvm/Support.
JOBS="${JOBS:-3}"

mkdir -p "$JPKG_OUTPUT" "$JPKG_PUBLISHED" "$JPKG_BIN" "$SCCACHE" "$(dirname "$SCCACHE_BIN")"

# Auto-fetch the static-musl sccache binary on first run. The CI workflow
# pulls v0.15.0 from GitHub releases; we mirror that exactly so cache keys
# stay compatible.
SCCACHE_VERSION="${SCCACHE_VERSION:-v0.15.0}"
if [ ! -x "$SCCACHE_BIN" ]; then
    echo "==> Fetching sccache $SCCACHE_VERSION (x86_64-unknown-linux-musl)"
    tarball="${BUILD_DIR}/sccache-bin/sccache-x86_64.tgz"
    curl -fsSL -o "$tarball" \
        "https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz"
    tar -xzf "$tarball" -C "$(dirname "$SCCACHE_BIN")" --strip-components=1 \
        "sccache-${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache"
    mv "$(dirname "$SCCACHE_BIN")/sccache" "$SCCACHE_BIN"
    chmod +x "$SCCACHE_BIN"
    rm -f "$tarball"
fi

usage() {
    cat <<EOF
local-build-x86_64.sh — local hedge builder

  build PKG [PKG...]   Build one or more packages in a docker container.
  chain                Build the LLVM split: libllvm → clang → lld → llvm → llvm-extra.
  chain22              Build the parallel LLVM 22 split under /lib/llvm22.
  upload               Upload winning .jpkg(s) from $JPKG_OUTPUT to the
                       $RELEASE_TAG release on $GITHUB_REPO, then trigger
                       regen-tag-index to bake them into a signed INDEX.
  status               Show what .jpkgs are sitting in the local cache.
  clean                Wipe $JPKG_OUTPUT (does NOT touch sccache or jpkg-bin).

Env knobs:
  BUILDER_IMAGE   default $BUILDER_IMAGE
  GITHUB_REPO     default $GITHUB_REPO
  RELEASE_TAG     default $RELEASE_TAG
  JOBS            default 3   (LLVM_BUILD_JOBS / BUILD_JOBS passed to recipe)
  REBUILD         set to 1 to rebuild even if $RELEASE_TAG already has the asset
  JPKG_SIGN_KEY   optional path to a jpkg .sec key mounted read-only into the
                  builder so local artifacts are signed at build time

Volumes mounted into the container:
  /workspace             $REPO_ROOT
  /var/cache/jpkg        $JPKG_OUTPUT
  /var/cache/jpkg-published  $JPKG_PUBLISHED
  /jpkg-bin              $JPKG_BIN
  /var/cache/sccache     $SCCACHE
EOF
}

cmd_build() {
    [ "$#" -ge 1 ] || { usage; exit 2; }

    # Refresh the local jpkg-published cache so the in-container build
    # script (ci-build-x86_64.sh, reused unchanged) can detect already-
    # published packages and skip them.
    if [ -z "${SKIP_PUBLISHED_REFRESH:-}" ]; then
        echo "==> Refreshing $JPKG_PUBLISHED (gh release download $RELEASE_TAG)"
        gh release download "$RELEASE_TAG" \
            --repo "$GITHUB_REPO" \
            --pattern "*-x86_64.jpkg" \
            --dir "$JPKG_PUBLISHED" \
            --skip-existing 2>/dev/null || true
        echo "    cached: $(ls "$JPKG_PUBLISHED"/*.jpkg 2>/dev/null | wc -l | tr -d ' ') x86_64 jpkgs"
    fi

    for pkg in "$@"; do
        echo "==> Local hedge build: $pkg (x86_64, JOBS=$JOBS)"
        # JMAKE_OVERRIDE: optional host path to a jmake binary to mount
        # over /bin/jmake in the container. Same pattern as aarch64.
        _jmake_override_args=""
        if [ -n "${JMAKE_OVERRIDE:-}" ] && [ -f "${JMAKE_OVERRIDE}" ]; then
            _jmake_override_args="-v ${JMAKE_OVERRIDE}:/bin/jmake:ro -v ${JMAKE_OVERRIDE}:/bin/make:ro"
            echo "    using jmake override: $JMAKE_OVERRIDE"
        fi
        _sign_mount_args=""
        _sign_env_args=""
        if [ -n "${JPKG_SIGN_KEY:-}" ]; then
            if [ ! -r "$JPKG_SIGN_KEY" ]; then
                echo "ERROR: JPKG_SIGN_KEY is set but not readable: $JPKG_SIGN_KEY" >&2
                exit 1
            fi
            _sign_mount_args="-v ${JPKG_SIGN_KEY}:${JPKG_SIGN_KEY}:ro"
            _sign_env_args="-e JPKG_SIGN_KEY=${JPKG_SIGN_KEY}"
            echo "    signing enabled with JPKG_SIGN_KEY"
        fi
        docker run --rm \
            --platform linux/amd64 \
            --entrypoint /bin/sh \
            -v "$REPO_ROOT:/workspace" \
            -v "$JPKG_OUTPUT:/var/cache/jpkg" \
            -v "$JPKG_PUBLISHED:/var/cache/jpkg-published" \
            -v "$JPKG_BIN:/jpkg-bin" \
            -v "$SCCACHE:/var/cache/sccache" \
            -v "$SCCACHE_BIN:/bin/sccache:ro" \
            $_jmake_override_args \
            $_sign_mount_args \
            -w /workspace \
            -e PKG_INPUT="$pkg" \
            -e REBUILD_INPUT="${REBUILD:-false}" \
            -e SCCACHE_DIR=/var/cache/sccache \
            -e RUSTC_WRAPPER=sccache \
            -e CC="sccache clang" \
            -e CXX="sccache clang++" \
            -e LLVM_BUILD_JOBS="$JOBS" \
            -e BUILD_JOBS="$JOBS" \
            $_sign_env_args \
            "$BUILDER_IMAGE" \
            /workspace/scripts/ci-build-x86_64.sh
        cache_local_pkg "$pkg"
    done

    echo "==> Local artifacts:"
    ls -lh "$JPKG_OUTPUT"/*.jpkg 2>/dev/null || echo "    (none)"
}

cache_local_pkg() {
    pkg=$1
    found=0

    for artifact in "$JPKG_OUTPUT"/"$pkg"-*-x86_64.jpkg; do
        [ -f "$artifact" ] || continue
        cp -f "$artifact" "$JPKG_PUBLISHED/"
        found=1
    done

    if [ "$found" -eq 1 ]; then
        echo "==> Mirrored local $pkg package(s) into $JPKG_PUBLISHED"
    fi
}

cmd_chain() {
    cmd_build libllvm clang lld llvm llvm-extra
}

cmd_chain22() {
    cmd_build libcxx22 libllvm22 clang22 lld22 llvm22 llvm22-extra
}

cmd_upload() {
    if ! command -v gh >/dev/null 2>&1; then
        echo "ERROR: gh CLI not found" >&2
        exit 1
    fi

    count=0
    for pkg in "$JPKG_OUTPUT"/*-x86_64.jpkg; do
        [ -f "$pkg" ] || continue
        name=$(basename "$pkg")
        for asset in "$pkg" "$pkg.sig"; do
            [ -f "$asset" ] || continue
            echo "==> Uploading $(basename "$asset") to $GITHUB_REPO ($RELEASE_TAG)"
            gh release upload "$RELEASE_TAG" "$asset" \
                --repo "$GITHUB_REPO" \
                --clobber
        done
        count=$((count + 1))
    done

    [ "$count" -eq 0 ] && { echo "Nothing to upload."; return 0; }

    echo "==> Triggering INDEX regen for $RELEASE_TAG"
    gh workflow run regen-tag-index.yml \
        --repo "$GITHUB_REPO" \
        -f tag="$RELEASE_TAG"
    echo "    INDEX regen dispatched.  Watch with: gh run list --repo $GITHUB_REPO --workflow=regen-tag-index.yml --limit 1"
}

cmd_status() {
    printf 'Local hedge cache: %s\n' "$JPKG_OUTPUT"
    if [ -z "$(ls "$JPKG_OUTPUT"/*.jpkg 2>/dev/null)" ]; then
        echo "  (empty)"
        return 0
    fi
    for pkg in "$JPKG_OUTPUT"/*.jpkg; do
        size=$(ls -lh "$pkg" | awk '{print $5}')
        printf '  %s  %s\n' "$size" "$(basename "$pkg")"
    done
    echo
    printf 'Published cache (read-only): %s\n' "$JPKG_PUBLISHED"
    n=$(ls "$JPKG_PUBLISHED"/*.jpkg 2>/dev/null | wc -l | tr -d ' ')
    printf '  %s pre-fetched .jpkg(s)\n' "$n"
}

cmd_clean() {
    rm -f "$JPKG_OUTPUT"/*.jpkg "$JPKG_OUTPUT"/*.sig
    echo "Cleaned $JPKG_OUTPUT"
}

case "${1:-}" in
    build)   shift; cmd_build "$@" ;;
    chain)   cmd_chain ;;
    chain22) cmd_chain22 ;;
    upload)  cmd_upload ;;
    status)  cmd_status ;;
    clean)   cmd_clean ;;
    -h|--help|help|'') usage ;;
    *) echo "Unknown subcommand: $1" >&2; usage; exit 2 ;;
esac
