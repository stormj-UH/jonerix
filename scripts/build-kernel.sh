#!/bin/sh
# build-kernel.sh — build the Linux kernel in an Alpine container
#
# Usage:
#   ./scripts/build-kernel.sh [--output DIR]
#
# This script builds packages/extra/linux/ inside a fresh Alpine container.
# It is the supported way to build the kernel recipe while jpkg's permissive-only
# license gate intentionally blocks GPL packages.
#
# Prerequisites:
#   - Docker (or a compatible runtime) available on PATH
#   - The jonerix repo checked out (this script uses $PWD as the repo root)
#   - At least ~20 GB of free disk space for the kernel build tree
#
# Output:
#   The built .jpkg file is written to /tmp/jpkg-output/ by default,
#   or to the directory passed with --output.
#
# Notes:
#   - LLVM=1 is used so clang/lld replace GCC/binutils entirely.
#   - GNU make, perl, bash, bc, flex, and bison are installed from Alpine for
#     the kernel build system; they are build-time only and do NOT enter the
#     final .jpkg archive.
#   - jpkg is compiled from source inside the container (requires cargo+rust).

set -e

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUTPUT_DIR="/tmp/jpkg-output"

# Parse arguments
while [ $# -gt 0 ]; do
    case "$1" in
        --output)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --output=*)
            OUTPUT_DIR="${1#--output=}"
            shift
            ;;
        -h|--help)
            sed -n '2,/^$/p' "$0"
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

mkdir -p "$OUTPUT_DIR"

echo "==> Building Linux kernel recipe"
echo "    Repo:   $REPO_ROOT"
echo "    Output: $OUTPUT_DIR"
echo ""

docker run --rm \
    -v "$REPO_ROOT:/workspace" \
    -v "$OUTPUT_DIR:/output" \
    alpine:latest sh -c '
        set -e

        echo "==> Installing Alpine build dependencies"
        # llvm provides llvm-ar, llvm-nm, llvm-objcopy, llvm-strip — all
        # required by the kernel build when LLVM=1 is set; clang+lld alone
        # are not enough.
        apk add --no-cache \
            clang lld llvm compiler-rt musl-dev \
            make perl bash bc flex bison \
            elfutils-dev openssl-dev linux-headers \
            diffutils findutils coreutils \
            zstd-dev zlib-dev \
            git rsync \
            cargo rust python3

        echo "==> Building jpkg 2.0 (Rust) from source"
        cd /workspace/packages/core/jpkg
        TRIPLE=$(rustc -vV | sed -n "s/^host: //p")
        RUSTFLAGS="-C strip=symbols -C target-feature=+crt-static" \
            cargo build --release --frozen --target "$TRIPLE" --bin jpkg --bin jpkg-local
        install -m 755 "target/$TRIPLE/release/jpkg" /usr/local/bin/jpkg
        echo "jpkg version: $(jpkg --version 2>/dev/null || echo unknown)"

        echo "==> Starting Linux kernel build"
        # jpkg build is blocked by the GPL license gate, so the kernel build
        # stays manual here.
        RECIPE_DIR=/workspace/packages/extra/linux
        # PKG_VERSION may be e.g. "6.14.2-r1" (recipe-level revision).
        # KERNEL_VERSION is the upstream Linux version used to fetch the tarball
        # (e.g. "6.14.2"). Strip any -rN suffix to recover it.
        PKG_VERSION=$(awk -F\" "/^version/ {print \$2; exit}" "$RECIPE_DIR/recipe.toml")
        KERNEL_VERSION=${PKG_VERSION%-r*}
        TARBALL="linux-${KERNEL_VERSION}.tar.xz"
        KERNEL_URL="https://cdn.kernel.org/pub/linux/kernel/v6.x/${TARBALL}"

        mkdir -p /build
        cd /build

        echo "==> Downloading Linux ${KERNEL_VERSION}"
        if [ ! -f "${TARBALL}" ]; then
            # BusyBox wget (Alpine default) does not support --show-progress.
            # Use -q and emit our own progress dot per chunk via -S, or just be silent.
            wget -q "${KERNEL_URL}" -O "${TARBALL}"
        fi

        echo "==> Verifying sha256 (update recipe.toml with the real hash)"
        # Fetch the official checksum file and verify
        wget -q "https://cdn.kernel.org/pub/linux/kernel/v6.x/sha256sums.asc" -O sha256sums.asc 2>/dev/null || true
        if [ -f sha256sums.asc ]; then
            EXPECTED=$(awk "/  ${TARBALL}$/ {print \$1}" sha256sums.asc)
            if [ -n "$EXPECTED" ]; then
                echo "${EXPECTED}  ${TARBALL}" | sha256sum -c -
                echo "sha256 OK: ${EXPECTED}"
                echo ""
                echo ">>> Update recipe.toml source.sha256 to: ${EXPECTED}"
                echo ""
            fi
        fi

        echo "==> Extracting Linux ${KERNEL_VERSION}"
        tar -xf "${TARBALL}"
        cd "linux-${KERNEL_VERSION}"

        ARCH=$(uname -m)
        case "$ARCH" in
            x86_64)  KARCH=x86_64 ; CONFIG_FRAG=jonerix-x86_64.config ; VMLINUZ=arch/x86/boot/bzImage ;;
            aarch64) KARCH=arm64  ; CONFIG_FRAG=jonerix-aarch64.config ; VMLINUZ=arch/arm64/boot/Image.gz ;;
            *) echo "Unsupported arch: $ARCH" ; exit 1 ;;
        esac

        export LLVM=1
        export LD=ld.lld

        # Clang 21 (Alpine latest) enables several warnings by default that
        # Linux 6.14.2 has not adopted fixes for. The kernel build promotes
        # warnings to errors, so suppress these via KCFLAGS:
        #   * -Wdefault-const-init-var-unsafe / -field-unsafe: triggers in
        #     include/net/ip.h and arch/x86/kernel/alternative.c
        #   * -Wunterminated-string-initialization: triggers throughout
        #     drivers/acpi/tables.c (ACPI signatures are exactly 4 chars,
        #     stored in char[4] without the trailing NUL)
        # These are build-environment workarounds; no runtime change.
        export KCFLAGS="-Wno-default-const-init-unsafe -Wno-default-const-init-var-unsafe -Wno-default-const-init-field-unsafe -Wno-unterminated-string-initialization"

        # Parallelism: cap at RAM/2 GB to avoid OOM during linking
        NCPUS=$(nproc)
        RAM_GB=$(awk "/MemTotal/ { printf \"%d\", \$2/1024/1024 }" /proc/meminfo 2>/dev/null || echo 4)
        MAX_JOBS=$(( RAM_GB / 2 ))
        [ "$MAX_JOBS" -lt 1 ] && MAX_JOBS=1
        [ "$MAX_JOBS" -gt "$NCPUS" ] && MAX_JOBS="$NCPUS"
        echo "==> Building ARCH=$KARCH LLVM=1 JOBS=$MAX_JOBS (${RAM_GB}GB RAM)"

        make ARCH="$KARCH" defconfig

        if [ -f "$RECIPE_DIR/$CONFIG_FRAG" ]; then
            echo "==> Merging jonerix config fragment: $CONFIG_FRAG"
            scripts/kconfig/merge_config.sh -m .config "$RECIPE_DIR/$CONFIG_FRAG"
            make ARCH="$KARCH" olddefconfig
        fi

        make ARCH="$KARCH" -j"$MAX_JOBS" all

        echo "==> Installing to DESTDIR"
        KVER=$(make ARCH="$KARCH" -s kernelversion)
        DESTDIR="/build/destdir"
        mkdir -p "$DESTDIR/boot"

        install -m644 "$VMLINUZ" "$DESTDIR/boot/vmlinuz-${KVER}"
        ln -sf "vmlinuz-${KVER}" "$DESTDIR/boot/vmlinuz"
        install -m644 System.map "$DESTDIR/boot/System.map-${KVER}"
        install -m644 .config    "$DESTDIR/boot/config-${KVER}"

        make ARCH="$KARCH" INSTALL_MOD_PATH="$DESTDIR" INSTALL_MOD_STRIP=1 modules_install
        rm -f "$DESTDIR/lib/modules/${KVER}/build" "$DESTDIR/lib/modules/${KVER}/source"

        make ARCH="$KARCH" INSTALL_HDR_PATH="$DESTDIR" headers_install
        if [ -d "$DESTDIR/usr" ]; then
            cp -a "$DESTDIR/usr/." "$DESTDIR/"
            rm -rf "$DESTDIR/usr"
        fi

        echo "==> Packaging into .jpkg archive"
        # Build a minimal .jpkg using jpkg internals or a direct tar+zstd pack.
        # Until the license gate is fixed, create the archive manually.
        cd "$DESTDIR"
        # Use the upstream kernel version in the raw archive name; the wrapper
        # step that produces the real .jpkg will use the full PKG_VERSION
        # (e.g. 6.14.2-r1) in the final file name.
        PKGNAME="linux-${KERNEL_VERSION}-${ARCH}.jpkg"
        tar -cf - . | zstd -19 -o "/output/${PKGNAME}"
        echo "==> Built: /output/${PKGNAME}"
        ls -lh "/output/${PKGNAME}"
    '

echo ""
echo "==> Kernel build complete."
echo "    Output: $OUTPUT_DIR"
ls -lh "$OUTPUT_DIR"
