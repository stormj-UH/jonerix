#!/bin/sh
# local-build-aarch64.sh — Local hedge builder on Apple Silicon (colima/docker).
#
# Mirrors the publish-packages.yml CI flow, but everything stays on this Mac:
# the host runs colima with virtiofs, so we mount paths under
# /Users/jonerik/Desktop/jonerix/.local-build/ which colima passes through to
# the container.  /tmp doesn't work — that path lives inside the colima VM
# and never propagates back to the host.
#
# Usage:
#   ./scripts/local-build-aarch64.sh build PKG [PKG...]
#   ./scripts/local-build-aarch64.sh chain  # libllvm → clang → lld → llvm → llvm-extra
#   ./scripts/local-build-aarch64.sh upload  # push winning .jpkgs to GitHub release
#   ./scripts/local-build-aarch64.sh status  # what's in the local hedge cache
#
# Once one of these wins a race against CI we upload the .jpkg(s) to the
# `packages` release on GitHub, then trigger the regen-tag-index workflow
# so the freshly-uploaded asset gets pulled into a signed INDEX.zst.
#
# SPDX-License-Identifier: MIT

set -eu

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
BUILD_DIR="${REPO_ROOT}/.local-build"
JPKG_OUTPUT="${BUILD_DIR}/jpkg-output"
JPKG_PUBLISHED="${BUILD_DIR}/jpkg-published"
JPKG_BIN="${BUILD_DIR}/jpkg-bin-aarch64"
SCCACHE="${BUILD_DIR}/sccache-cache"

BUILDER_IMAGE="${BUILDER_IMAGE:-ghcr.io/stormj-uh/jonerix:builder-arm64}"
GITHUB_REPO="${GITHUB_REPO:-stormj-UH/jonerix}"
RELEASE_TAG="${RELEASE_TAG:-packages}"

mkdir -p "$JPKG_OUTPUT" "$JPKG_PUBLISHED" "$JPKG_BIN" "$SCCACHE"

usage() {
    cat <<EOF
local-build-aarch64.sh — local hedge builder

  build PKG [PKG...]   Build one or more packages in the colima docker VM.
  chain                Build the LLVM split: libllvm → clang → lld → llvm → llvm-extra.
  upload               Upload winning .jpkg(s) from $JPKG_OUTPUT to the
                       $RELEASE_TAG release on $GITHUB_REPO, then trigger
                       regen-tag-index to bake them into a signed INDEX.
  status               Show what .jpkgs are sitting in the local cache.
  clean                Wipe $JPKG_OUTPUT (does NOT touch sccache or jpkg-bin).

Env knobs:
  BUILDER_IMAGE   default $BUILDER_IMAGE
  GITHUB_REPO     default $GITHUB_REPO
  RELEASE_TAG     default $RELEASE_TAG
  REBUILD         set to 1 to rebuild even if $RELEASE_TAG already has the asset

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

    # Refresh the local jpkg-published cache so the in-container build script
    # (ci-build-aarch64.sh, reused unchanged) can detect already-published
    # packages and skip them.  Cheap: gh release download is incremental.
    if [ -z "${SKIP_PUBLISHED_REFRESH:-}" ]; then
        echo "==> Refreshing $JPKG_PUBLISHED (gh release download $RELEASE_TAG)"
        gh release download "$RELEASE_TAG" \
            --repo "$GITHUB_REPO" \
            --pattern "*-aarch64.jpkg" \
            --dir "$JPKG_PUBLISHED" \
            --skip-existing 2>/dev/null || true
        echo "    cached: $(ls "$JPKG_PUBLISHED"/*.jpkg 2>/dev/null | wc -l | tr -d ' ') aarch64 jpkgs"
    fi

    for pkg in "$@"; do
        echo "==> Local hedge build: $pkg (aarch64)"
        docker run --rm \
            --platform linux/arm64 \
            -v "$REPO_ROOT:/workspace" \
            -v "$JPKG_OUTPUT:/var/cache/jpkg" \
            -v "$JPKG_PUBLISHED:/var/cache/jpkg-published" \
            -v "$JPKG_BIN:/jpkg-bin" \
            -v "$SCCACHE:/var/cache/sccache" \
            -w /workspace \
            -e PKG_INPUT="$pkg" \
            -e REBUILD_INPUT="${REBUILD:-false}" \
            -e SCCACHE_DIR=/var/cache/sccache \
            -e RUSTC_WRAPPER=sccache \
            -e CC="sccache clang" \
            -e CXX="sccache clang++" \
            "$BUILDER_IMAGE" \
            sh /workspace/scripts/ci-build-aarch64.sh
    done

    echo "==> Local artifacts:"
    ls -lh "$JPKG_OUTPUT"/*.jpkg 2>/dev/null || echo "    (none)"
}

cmd_chain() {
    cmd_build libllvm clang lld llvm llvm-extra
}

cmd_upload() {
    if ! command -v gh >/dev/null 2>&1; then
        echo "ERROR: gh CLI not found" >&2
        exit 1
    fi

    count=0
    for pkg in "$JPKG_OUTPUT"/*-aarch64.jpkg; do
        [ -f "$pkg" ] || continue
        name=$(basename "$pkg")
        echo "==> Uploading $name to $GITHUB_REPO ($RELEASE_TAG)"
        gh release upload "$RELEASE_TAG" "$pkg" \
            --repo "$GITHUB_REPO" \
            --clobber
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
    upload)  cmd_upload ;;
    status)  cmd_status ;;
    clean)   cmd_clean ;;
    -h|--help|help|'') usage ;;
    *) echo "Unknown subcommand: $1" >&2; usage; exit 2 ;;
esac
