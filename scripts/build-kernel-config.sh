#!/bin/sh
# build-kernel-config.sh — merge jonerix kernel fragments into a
# .config inside a Linux source tree.
#
# Usage:
#   cd /usr/src/linux
#   /path/to/jonerix/scripts/build-kernel-config.sh \
#       --arch=aarch64-pi5 --profile=builder
#
# Or specify the jonerix path explicitly:
#   ./build-kernel-config.sh \
#       --jonerix=/path/to/jonerix \
#       --arch=x86_64 --profile=router \
#       --kernel-src=/usr/src/linux
#
# Requirements: Linux source tree with scripts/kconfig/merge_config.sh
# (every kernel since 3.0).  Builds against whatever ARCH= make
# variable matches the chosen jonerix arch fragment.

set -eu

ARCH=
PROFILE=
JONERIX="$(cd "$(dirname "$0")/.." 2>/dev/null && pwd)"
KSRC=

usage() {
    cat <<EOF
build-kernel-config.sh — merge jonerix fragments into .config

  --arch=<NAME>       one of:  x86_64  aarch64-pi5  aarch64-server
  --profile=<NAME>    one of:  minimal  builder  router
  --jonerix=<DIR>     path to jonerix repo (default: \$(dirname \$0)/..)
  --kernel-src=<DIR>  path to Linux source tree (default: pwd)

Examples:
  cd /usr/src/linux
  make defconfig
  $0 --arch=x86_64 --profile=builder
  make olddefconfig
  make -j\$(nproc)

  # Pi 5:
  cd /usr/src/linux-rpi
  make ARCH=arm64 bcm2712_defconfig
  $0 --arch=aarch64-pi5 --profile=minimal
  make ARCH=arm64 olddefconfig
  make ARCH=arm64 Image dtbs modules -j\$(nproc)
EOF
}

while [ $# -gt 0 ]; do
    case "$1" in
        --arch=*)        ARCH=${1#--arch=} ;;
        --profile=*)     PROFILE=${1#--profile=} ;;
        --jonerix=*)     JONERIX=${1#--jonerix=} ;;
        --kernel-src=*)  KSRC=${1#--kernel-src=} ;;
        -h|--help)       usage; exit 0 ;;
        *) printf 'unknown arg: %s\n' "$1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

[ -n "$ARCH" ]    || { printf 'error: --arch required\n' >&2; usage >&2; exit 2; }
[ -n "$PROFILE" ] || { printf 'error: --profile required\n' >&2; usage >&2; exit 2; }

KSRC=${KSRC:-$PWD}

BASE="$JONERIX/config/kernel/base.config"
ARCH_FRAG="$JONERIX/config/kernel/arch/${ARCH}.config"
PROF_FRAG="$JONERIX/config/kernel/profile/${PROFILE}.config"
MERGE="$KSRC/scripts/kconfig/merge_config.sh"
DOTCONFIG="$KSRC/.config"

for f in "$BASE" "$ARCH_FRAG" "$PROF_FRAG" "$MERGE"; do
    [ -r "$f" ] || { printf 'missing: %s\n' "$f" >&2; exit 3; }
done
[ -r "$DOTCONFIG" ] || {
    printf 'no .config in %s — run `make defconfig` (or the arch-\n' "$KSRC" >&2
    printf 'specific defconfig like bcm2712_defconfig) first.\n' >&2
    exit 4
}

printf 'jonerix:     %s\n' "$JONERIX"
printf 'kernel src:  %s\n' "$KSRC"
printf 'merging:     base + arch/%s + profile/%s\n' "$ARCH" "$PROFILE"
printf '\n'

# -m = merge mode (keep newer values), -O = output dir, then .config
# is the starting point and the fragments are layered on top in order.
cd "$KSRC"
"$MERGE" -m -O . "$DOTCONFIG" "$BASE" "$ARCH_FRAG" "$PROF_FRAG"

printf '\n'
printf 'next step: cd %s && make olddefconfig\n' "$KSRC"
