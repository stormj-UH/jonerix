#!/bin/sh
# m4oxide installer (strict POSIX sh; dash/mksh/busybox compatible).
#
# Default install lays down only the primary binary plus its license and any
# bundled man pages. The convenience symlink $PREFIX/bin/m4 -> m4oxide is
# strictly opt-in: either pass --make-default, or accept the interactive
# prompt. The installer never touches /usr/bin/m4 or any other system binary.
#
# Usage:
#   sh install.sh [--version VER] [--prefix DIR] [--arch ARCH]
#                 [--make-default | --no-make-default]
#                 [--no-prompt | --yes]
#                 [--help]
#   curl -fsSL https://example/install.sh | sh
#   curl -fsSL https://example/install.sh | sh -s -- --no-prompt --make-default

set -eu

VERSION="0.1.2-r0"
PREFIX="/usr/local"
ARCH=""
MAKE_DEFAULT=0           # 0=unset, 1=yes, -1=explicit no
NO_PROMPT=0
: "${URL_TEMPLATE:=https://github.com/stormj-UH/jonerix/releases/download/packages/m4oxide-%s-%s.jpkg}"

usage() {
    cat <<EOF
Usage: $0 [options]

Options:
  --version VER       package version to install (default: $VERSION)
  --prefix  DIR       install prefix (default: $PREFIX)
  --arch    ARCH      target arch (default: detected via uname -m)
  --make-default      also install \$PREFIX/bin/m4 as a symlink to m4oxide
  --no-make-default   skip the m4 symlink (default; suppresses prompt)
  --no-prompt         non-interactive; accept defaults for all opt-ins
  --yes               alias for --no-prompt
  --help              show this help and exit

Long options also accept --key=value form.

Default install lays down:
  \$PREFIX/bin/m4oxide
  \$PREFIX/share/licenses/m4oxide/LICENSE   (if shipped in payload)
  \$PREFIX/share/man/manN/m4oxide.N         (if shipped in payload)

The \$PREFIX/bin/m4 symlink is opt-in via --make-default or the interactive
prompt. /usr/bin/m4 (or any other system binary) is never touched.
EOF
}

# ---- argument parsing ----------------------------------------------------
# POSIX getopts can't do long options; do it by hand.
while [ $# -gt 0 ]; do
    arg=$1
    val=""
    case $arg in
        --version=*) val=${arg#*=}; arg=--version ;;
        --prefix=*)  val=${arg#*=}; arg=--prefix ;;
        --arch=*)    val=${arg#*=}; arg=--arch ;;
    esac
    case $arg in
        --version)
            if [ -z "$val" ]; then shift; [ $# -gt 0 ] || { echo "$0: --version needs a value" >&2; exit 2; }; val=$1; fi
            VERSION=$val
            ;;
        --prefix)
            if [ -z "$val" ]; then shift; [ $# -gt 0 ] || { echo "$0: --prefix needs a value" >&2; exit 2; }; val=$1; fi
            PREFIX=$val
            ;;
        --arch)
            if [ -z "$val" ]; then shift; [ $# -gt 0 ] || { echo "$0: --arch needs a value" >&2; exit 2; }; val=$1; fi
            ARCH=$val
            ;;
        --make-default)
            MAKE_DEFAULT=1
            ;;
        --no-make-default)
            MAKE_DEFAULT=-1
            ;;
        --no-prompt|--yes)
            NO_PROMPT=1
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        -*)
            echo "$0: unknown option: $arg" >&2
            usage >&2
            exit 2
            ;;
        *)
            echo "$0: unexpected argument: $arg" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

# ---- arch detection ------------------------------------------------------
if [ -z "$ARCH" ]; then
    raw=$(uname -m 2>/dev/null || echo unknown)
    case $raw in
        x86_64|amd64)        ARCH=x86_64 ;;
        aarch64|arm64)       ARCH=aarch64 ;;
        armv7l|armv7|armhf)  ARCH=armv7 ;;
        riscv64)             ARCH=riscv64 ;;
        i686|i386)           ARCH=i686 ;;
        *)                   ARCH=$raw ;;
    esac
fi

echo "m4oxide installer"
echo "  version: $VERSION"
echo "  arch:    $ARCH"
echo "  prefix:  $PREFIX"

# ---- tool checks ---------------------------------------------------------
need_one_of() {
    # echo the first command found in $PATH from the args, or empty
    for c in "$@"; do
        if command -v "$c" >/dev/null 2>&1; then
            printf '%s\n' "$c"
            return 0
        fi
    done
    return 1
}

DL=$(need_one_of curl wget) || {
    echo "error: need curl or wget on PATH" >&2
    exit 1
}

for t in zstd tar od dd install mktemp uname; do
    command -v "$t" >/dev/null 2>&1 || {
        echo "error: required tool not found: $t" >&2
        exit 1
    }
done

# ---- temp workspace ------------------------------------------------------
TMP=$(mktemp -d 2>/dev/null || mktemp -d -t m4oxide)
[ -n "$TMP" ] && [ -d "$TMP" ] || {
    echo "error: failed to create temp dir" >&2
    exit 1
}
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT INT HUP TERM

JPKG=$TMP/pkg.jpkg
EXTRACT=$TMP/extract
mkdir -p "$EXTRACT"

# ---- download ------------------------------------------------------------
URL=$(printf '%s' "$URL_TEMPLATE" | awk -v v="$VERSION" -v a="$ARCH" '{
    sub(/%s/, v); sub(/%s/, a); print
}')

echo "downloading: $URL"
case $DL in
    curl) curl -fL --retry 3 --retry-delay 1 -o "$JPKG" "$URL" ;;
    wget) wget -q --tries=3 -O "$JPKG" "$URL" ;;
esac

[ -s "$JPKG" ] || {
    echo "error: download empty or missing: $JPKG" >&2
    exit 1
}

# ---- magic check ---------------------------------------------------------
# First 4 bytes must be 'JPKG' = 0x4a 0x50 0x4b 0x47.
magic=$(dd if="$JPKG" bs=1 count=4 status=none | od -An -c | tr -d ' \n')
if [ "$magic" != "JPKG" ]; then
    echo "error: not a JPKG file (magic='$magic')" >&2
    exit 1
fi

# Format version: bytes 4..7 (little-endian). Don't gate on a specific value.
fmt=$(od -An -tu4 -N4 -j4 "$JPKG" | tr -d ' ')
echo "format version: $fmt"

# ---- extract -------------------------------------------------------------
md_len=$(od -An -tu4 -N4 -j8 "$JPKG" | tr -d ' ')
[ -n "$md_len" ] && [ "$md_len" -gt 0 ] || {
    echo "error: invalid metadata length: $md_len" >&2
    exit 1
}

payload_offset=$((12 + md_len))
echo "extracting payload (offset=$payload_offset)"
dd if="$JPKG" bs=1 skip="$payload_offset" status=none | zstd -d -q | tar -x -C "$EXTRACT"

# Locate the binary. The recipe installs to bin/m4oxide, but try a few sane
# fallbacks in case a future package layout changes.
BIN_SRC=""
for cand in "$EXTRACT/bin/m4oxide" "$EXTRACT/usr/bin/m4oxide" "$EXTRACT/usr/local/bin/m4oxide"; do
    if [ -f "$cand" ]; then
        BIN_SRC=$cand
        break
    fi
done
[ -n "$BIN_SRC" ] || {
    echo "error: m4oxide binary not found in package payload" >&2
    echo "payload contents:" >&2
    (cd "$EXTRACT" && find . -type f -o -type l) >&2 || true
    exit 1
}

# License (best effort).
LIC_SRC=""
for cand in \
    "$EXTRACT/share/licenses/m4oxide/LICENSE" \
    "$EXTRACT/usr/share/licenses/m4oxide/LICENSE" \
    "$EXTRACT/LICENSE" \
    "$EXTRACT/LICENSE.txt" \
    "$EXTRACT/license" \
    "$EXTRACT/COPYING"; do
    if [ -f "$cand" ]; then
        LIC_SRC=$cand
        break
    fi
done

# Man pages (best effort). Find any m4oxide.N or m4oxide.N.gz under common
# man roots in the payload and remember their relative paths.
MAN_LIST=$TMP/manlist
: >"$MAN_LIST"
for root in "$EXTRACT/share/man" "$EXTRACT/usr/share/man" "$EXTRACT/usr/local/share/man"; do
    [ -d "$root" ] || continue
    # Use -path so we capture the manN/ directory in the relative path.
    find "$root" -type f \( -name 'm4oxide.[0-9]' -o -name 'm4oxide.[0-9].gz' \) \
        -print 2>/dev/null \
        | while IFS= read -r man_path; do
            # Strip the root prefix; what remains starts with /manN/file.
            rel=${man_path#"$root"/}
            printf '%s\t%s\n' "$man_path" "$rel" >>"$MAN_LIST"
        done
done

# ---- interactive opt-ins -------------------------------------------------
# If the user didn't specify --make-default or --no-make-default, prompt
# (unless --no-prompt). A non-interactive stdin (e.g. piped curl|sh) also
# implies "no prompt" so we don't hang.
if [ "$MAKE_DEFAULT" -eq 0 ]; then
    if [ "$NO_PROMPT" -eq 1 ] || [ ! -t 0 ]; then
        MAKE_DEFAULT=-1
    else
        # shellcheck disable=SC2016
        printf 'Symlink %s/bin/m4 -> m4oxide? Will be shadowed by /usr/bin/m4 if %s/bin is later on $PATH. [y/N] ' "$PREFIX" "$PREFIX"
        # POSIX read; trailing newline already consumed.
        if IFS= read -r reply; then
            case $reply in
                [yY]|[yY][eE][sS]) MAKE_DEFAULT=1 ;;
                *)                 MAKE_DEFAULT=-1 ;;
            esac
        else
            MAKE_DEFAULT=-1
        fi
    fi
fi

# ---- install -------------------------------------------------------------
BIN_DIR=$PREFIX/bin
LIC_DIR=$PREFIX/share/licenses/m4oxide
MAN_BASE=$PREFIX/share/man

echo "installing to $PREFIX"

# Need write access; if not, re-exec under sudo when available, else error.
if ! mkdir -p "$BIN_DIR" "$LIC_DIR" 2>/dev/null; then
    if command -v sudo >/dev/null 2>&1; then
        SUDO=sudo
    else
        echo "error: cannot write under $PREFIX and sudo is unavailable" >&2
        exit 1
    fi
    $SUDO mkdir -p "$BIN_DIR" "$LIC_DIR"
else
    SUDO=""
fi

$SUDO install -m 0755 "$BIN_SRC" "$BIN_DIR/m4oxide"

INSTALLED_LICENSE=0
if [ -n "$LIC_SRC" ]; then
    $SUDO install -m 0644 "$LIC_SRC" "$LIC_DIR/LICENSE"
    INSTALLED_LICENSE=1
else
    echo "warning: no LICENSE file in payload; skipping license install"
fi

INSTALLED_MAN=0
if [ -s "$MAN_LIST" ]; then
    while IFS='	' read -r src rel; do
        [ -n "$src" ] || continue
        # rel looks like "man1/m4oxide.1" or "man1/m4oxide.1.gz".
        dest_dir=$MAN_BASE/${rel%/*}
        dest=$MAN_BASE/$rel
        $SUDO mkdir -p "$dest_dir"
        $SUDO install -m 0644 "$src" "$dest"
        INSTALLED_MAN=$((INSTALLED_MAN + 1))
    done <"$MAN_LIST"
fi

SYMLINK_CREATED=0
if [ "$MAKE_DEFAULT" -eq 1 ]; then
    # Best effort: if a real `m4` already exists at the same path, replace it
    # with our symlink (preserving the prior file as .pre-m4oxide). We never
    # touch any path outside $BIN_DIR.
    target=$BIN_DIR/m4
    if [ -e "$target" ] && [ ! -L "$target" ]; then
        echo "preserving existing $target as ${target}.pre-m4oxide"
        $SUDO mv -f "$target" "${target}.pre-m4oxide"
    fi
    $SUDO ln -sfn m4oxide "$target"
    SYMLINK_CREATED=1
fi

# ---- verify --------------------------------------------------------------
if ! "$BIN_DIR/m4oxide" --version >/dev/null 2>&1; then
    echo "error: $BIN_DIR/m4oxide --version failed" >&2
    exit 1
fi
INSTALLED_VERSION=$("$BIN_DIR/m4oxide" --version 2>&1 | head -n 1)

# ---- post-install trailer -----------------------------------------------
# Detect whether $BIN_DIR is on PATH and whether some other m4 wins.
on_path=0
shadow_m4=""
IFS_save=$IFS
IFS=:
for d in $PATH; do
    [ -z "$d" ] && continue
    if [ "$d" = "$BIN_DIR" ]; then
        on_path=1
        break
    fi
    if [ -x "$d/m4" ]; then
        # First m4 on $PATH ahead of $BIN_DIR.
        shadow_m4=$d/m4
    fi
done
IFS=$IFS_save

echo
echo "=========================================================="
echo " m4oxide install complete"
echo "=========================================================="
echo "  version installed: $INSTALLED_VERSION"
echo "  binary:            $BIN_DIR/m4oxide"
if [ "$INSTALLED_LICENSE" -eq 1 ]; then
    echo "  license:           $LIC_DIR/LICENSE"
else
    echo "  license:           (not in payload, skipped)"
fi
if [ "$INSTALLED_MAN" -gt 0 ]; then
    echo "  man pages:         $INSTALLED_MAN file(s) under $MAN_BASE"
else
    echo "  man pages:         (none in payload)"
fi
if [ "$SYMLINK_CREATED" -eq 1 ]; then
    echo "  m4 symlink:        $BIN_DIR/m4 -> m4oxide"
else
    echo "  m4 symlink:        not installed"
fi
echo

if [ "$on_path" -eq 0 ]; then
    echo "PATH: $BIN_DIR is NOT on your PATH."
    echo "      Add it via: export PATH=\"$BIN_DIR:\$PATH\""
    echo
fi

if [ "$SYMLINK_CREATED" -eq 1 ] && [ -n "$shadow_m4" ]; then
    echo "*****************************************************"
    echo "** WARNING: $shadow_m4"
    echo "** appears earlier on \$PATH than $BIN_DIR/m4."
    echo "** Typing 'm4' will continue to invoke that binary,"
    echo "** NOT m4oxide. To make m4oxide the default 'm4':"
    echo "**   - reorder \$PATH so $BIN_DIR comes first, OR"
    echo "**   - remove the older m4 from $shadow_m4 manually."
    echo "** This installer never touches /usr/bin/m4 or any"
    echo "** other system binary."
    echo "*****************************************************"
    echo
fi

if [ "$SYMLINK_CREATED" -eq 0 ]; then
    echo "Next steps (declined opt-ins):"
    echo "  - to use m4oxide as plain 'm4', re-run with --make-default,"
    echo "    or symlink it yourself: ln -sfn m4oxide $BIN_DIR/m4"
    echo "  - to invoke directly, run: $BIN_DIR/m4oxide"
    echo
fi

echo "done."
