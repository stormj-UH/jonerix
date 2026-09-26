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
#   ./scripts/local-build-aarch64.sh chain22 # libcxx22 -> libllvm22 -> clang22 -> lld22 -> llvm22 -> llvm22-extra
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
SCCACHE_BIN="${BUILD_DIR}/sccache-bin/sccache"

BUILDER_IMAGE="${BUILDER_IMAGE:-ghcr.io/stormj-uh/jonerix:builder}"
GITHUB_REPO="${GITHUB_REPO:-stormj-UH/jonerix}"
RELEASE_TAG="${RELEASE_TAG:-packages}"
JOBS="${JOBS:-2}"
DOCKER="${DOCKER:-docker}"
COLIMA="${COLIMA:-colima}"
LIMACTL="${LIMACTL:-limactl}"
COLIMA_PROFILE="${COLIMA_PROFILE:-default}"
COLIMA_CPUS="${COLIMA_CPUS:-4}"
COLIMA_MEMORY="${COLIMA_MEMORY:-8}"
COLIMA_DISK="${COLIMA_DISK:-100}"
COLIMA_MOUNT_ROOT="${COLIMA_MOUNT_ROOT:-$(dirname "$REPO_ROOT")}"
COLIMA_LIMA_HOME="${COLIMA_LIMA_HOME:-$HOME/.colima/_lima}"

mkdir -p "$JPKG_OUTPUT" "$JPKG_PUBLISHED" "$JPKG_BIN" "$SCCACHE" "$(dirname "$SCCACHE_BIN")"

ensure_sccache() {
    # Auto-fetch the static-musl sccache binary on first build. The CI
    # workflow pulls v0.15.0 from GitHub releases; mirror that exactly so
    # cache keys stay compatible.
    SCCACHE_VERSION="${SCCACHE_VERSION:-v0.15.0}"
    if [ ! -x "$SCCACHE_BIN" ]; then
        printf '==> Fetching sccache %s (aarch64-unknown-linux-musl)\n' "$SCCACHE_VERSION"
        tarball="${BUILD_DIR}/sccache-bin/sccache.tgz"
        curl -fsSL -o "$tarball" \
            "https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/sccache-${SCCACHE_VERSION}-aarch64-unknown-linux-musl.tar.gz"
        tar -xzf "$tarball" -C "$(dirname "$SCCACHE_BIN")" --strip-components=1 \
            "sccache-${SCCACHE_VERSION}-aarch64-unknown-linux-musl/sccache"
        chmod +x "$SCCACHE_BIN"
        rm -f "$tarball"
    fi
}

docker_ready() {
    command -v "$DOCKER" >/dev/null 2>&1 || return 1
    "$DOCKER" version >/dev/null 2>&1
}

docker_context_name() {
    if [ "$COLIMA_PROFILE" = default ]; then
        printf 'colima\n'
    else
        printf 'colima-%s\n' "$COLIMA_PROFILE"
    fi
}

colima_disk_name() {
    if [ "$COLIMA_PROFILE" = default ]; then
        printf 'colima\n'
    else
        printf 'colima-%s\n' "$COLIMA_PROFILE"
    fi
}

colima_profile_exists() {
    "$COLIMA" list 2>/dev/null |
        awk -v profile="$COLIMA_PROFILE" 'NR > 1 && $1 == profile { found = 1 } END { exit found ? 0 : 1 }'
}

ensure_docker_ready() {
    if docker_ready; then
        return 0
    fi

    printf 'ERROR: Docker daemon is not reachable through %s.\n' "$DOCKER" >&2
    printf 'Run: %s up\n' "$0" >&2
    if command -v "$COLIMA" >/dev/null 2>&1; then
        printf '\nColima status:\n' >&2
        "$COLIMA" status "$COLIMA_PROFILE" >&2 || true
    fi
    exit 1
}

usage() {
    cat <<EOF
local-build-aarch64.sh — local hedge builder

  build PKG [PKG...]   Build one or more packages in the colima docker VM.
  chain                Build the LLVM split: libllvm → clang → lld → llvm → llvm-extra.
  chain22              Build the parallel LLVM 22 split under /lib/llvm22.
  up                   Start the Homebrew Colima/Docker builder backend.
  doctor               Show Docker/Colima state for this local builder.
  smoke                Run check-builder-toolchain in the builder image.
  upload               Upload winning .jpkg(s) from $JPKG_OUTPUT to the
                       $RELEASE_TAG release on $GITHUB_REPO, then trigger
                       regen-tag-index to bake them into a signed INDEX.
  status               Show what .jpkgs are sitting in the local cache.
  clean                Wipe $JPKG_OUTPUT (does NOT touch sccache or jpkg-bin).

Env knobs:
  BUILDER_IMAGE   default $BUILDER_IMAGE
  DOCKER          default $DOCKER
  COLIMA          default $COLIMA
  LIMACTL         default $LIMACTL
  COLIMA_PROFILE  default $COLIMA_PROFILE
  COLIMA_MOUNT_ROOT  default $COLIMA_MOUNT_ROOT
  GITHUB_REPO     default $GITHUB_REPO
  RELEASE_TAG     default $RELEASE_TAG
  JOBS            default 2   (LLVM_BUILD_JOBS / BUILD_JOBS passed to recipe)
  REBUILD         set to 1 to rebuild even if $RELEASE_TAG already has the asset
  ALLOW_NON_MAIN_UPLOAD  set to 1 to let `upload` publish to `packages` from a
                  branch other than main (refused by default)
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

cmd_doctor() {
    rc=0

    if command -v "$DOCKER" >/dev/null 2>&1; then
        printf 'Docker client: %s\n' "$(command -v "$DOCKER")"
        "$DOCKER" context ls || rc=1
        if docker_ready; then
            printf 'OK: Docker daemon is reachable.\n'
        else
            printf 'ERROR: Docker daemon is not reachable.\n' >&2
            "$DOCKER" version >&2 || true
            rc=1
        fi
    else
        printf 'ERROR: Docker client not found: %s\n' "$DOCKER" >&2
        rc=1
    fi

    if command -v "$COLIMA" >/dev/null 2>&1; then
        printf '\nColima client: %s\n' "$(command -v "$COLIMA")"
        "$COLIMA" version || rc=1
        "$COLIMA" list || rc=1
        "$COLIMA" status "$COLIMA_PROFILE" || true
    else
        printf 'ERROR: Colima client not found: %s\n' "$COLIMA" >&2
        rc=1
    fi

    return "$rc"
}

cmd_up() {
    command -v "$COLIMA" >/dev/null 2>&1 || {
        printf 'ERROR: Colima client not found: %s\n' "$COLIMA" >&2
        exit 1
    }
    command -v "$DOCKER" >/dev/null 2>&1 || {
        printf 'ERROR: Docker client not found: %s\n' "$DOCKER" >&2
        exit 1
    }

    if docker_ready; then
        printf 'OK: Docker daemon is already reachable.\n'
        return 0
    fi

    if colima_profile_exists; then
        printf '==> Clearing stale Colima state for profile: %s\n' "$COLIMA_PROFILE"
        "$COLIMA" stop "$COLIMA_PROFILE" --force || true
        if command -v "$LIMACTL" >/dev/null 2>&1; then
            LIMA_HOME="$COLIMA_LIMA_HOME" "$LIMACTL" disk unlock "$(colima_disk_name)" || true
        fi
        printf '==> Starting existing Colima profile: %s\n' "$COLIMA_PROFILE"
        "$COLIMA" start "$COLIMA_PROFILE" --runtime docker --ssh-config=false
    else
        printf '==> Creating Colima profile: %s\n' "$COLIMA_PROFILE"
        "$COLIMA" start "$COLIMA_PROFILE" \
            --runtime docker \
            --vm-type vz \
            --cpu "$COLIMA_CPUS" \
            --memory "$COLIMA_MEMORY" \
            --disk "$COLIMA_DISK" \
            --mount "${COLIMA_MOUNT_ROOT}:w" \
            --ssh-config=false
    fi

    "$DOCKER" context use "$(docker_context_name)" >/dev/null 2>&1 || true
    ensure_docker_ready
    printf 'OK: Docker daemon is reachable.\n'
}

cmd_smoke() {
    ensure_docker_ready
    "$DOCKER" run --rm \
        --platform linux/arm64 \
        --entrypoint /bin/sh \
        "$BUILDER_IMAGE" \
        -c 'check-builder-toolchain'
}

cmd_build() {
    [ "$#" -ge 1 ] || { usage; exit 2; }
    ensure_docker_ready
    ensure_sccache

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
        # shellcheck disable=SC2012
        echo "    cached: $(ls "$JPKG_PUBLISHED"/*.jpkg 2>/dev/null | wc -l | tr -d ' ') aarch64 jpkgs"
    fi

    for pkg in "$@"; do
        echo "==> Local hedge build: $pkg (aarch64, JOBS=$JOBS)"
        # The builder image's ENTRYPOINT is /bin/zsh (the runtime login shell
        # for users who docker-exec into it).  --entrypoint /bin/sh swaps it
        # out for the build invocation so the CMD ("sh ci-build-aarch64.sh")
        # actually runs.  Without this, zsh tries to open "sh" as a script
        # and dies with "/bin/zsh: can't open input file: sh".
        # JMAKE_OVERRIDE: optional host path to a jmake binary to mount
        # over /bin/jmake in the container.  Used when the builder image's
        # baked-in jmake is older than a fresh build that fixes a Makefile
        # bug surfaced by a particular recipe (e.g. python3 3.14.5 needs
        # jmake 1.2.2's multi-rule prereq fix).
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
        # shellcheck disable=SC2086
        "$DOCKER" run --rm \
            --platform linux/arm64 \
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
            -e CMAKE_C_COMPILER_LAUNCHER=sccache \
            -e CMAKE_CXX_COMPILER_LAUNCHER=sccache \
            -e RUSTC_WRAPPER=sccache \
            -e CC="sccache clang" \
            -e CXX="sccache clang++" \
            -e LLVM_BUILD_JOBS="$JOBS" \
            -e BUILD_JOBS="$JOBS" \
            $_sign_env_args \
            "$BUILDER_IMAGE" \
            /workspace/scripts/ci-build-aarch64.sh
        cache_local_pkg "$pkg"
    done

    echo "==> Local artifacts:"
    ls -lh "$JPKG_OUTPUT"/*.jpkg 2>/dev/null || echo "    (none)"
}

cache_local_pkg() {
    pkg=$1
    found=0

    for artifact in "$JPKG_OUTPUT"/"$pkg"-*-aarch64.jpkg; do
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

    # The rolling `packages` release feeds every host, so only builds of
    # main may land there (same rule as publish-packages.yml). Branch
    # builds belong in workflow artifacts or a scratch release tag.
    if [ "$RELEASE_TAG" = "packages" ] && [ "${ALLOW_NON_MAIN_UPLOAD:-0}" != "1" ]; then
        _branch=$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD 2>/dev/null || echo unknown)
        if [ "$_branch" != "main" ]; then
            echo "ERROR: refusing to upload to '$RELEASE_TAG' from branch '$_branch'." >&2
            echo "       Build from main, or set ALLOW_NON_MAIN_UPLOAD=1 if you really mean it." >&2
            exit 1
        fi
    fi

    count=0
    for pkg in "$JPKG_OUTPUT"/*-aarch64.jpkg; do
        [ -f "$pkg" ] || continue
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
        # shellcheck disable=SC2012
        size=$(ls -lh "$pkg" | awk '{print $5}')
        printf '  %s  %s\n' "$size" "$(basename "$pkg")"
    done
    echo
    printf 'Published cache (read-only): %s\n' "$JPKG_PUBLISHED"
    # shellcheck disable=SC2012
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
    up)      cmd_up ;;
    doctor)  cmd_doctor ;;
    smoke)   cmd_smoke ;;
    upload)  cmd_upload ;;
    status)  cmd_status ;;
    clean)   cmd_clean ;;
    -h|--help|help|'') usage ;;
    *) echo "Unknown subcommand: $1" >&2; usage; exit 2 ;;
esac
