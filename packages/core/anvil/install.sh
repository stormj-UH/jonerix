#!/bin/sh
# anvil installer — fetches an anvil .jpkg from the jonerix release feed and
# unpacks it under $PREFIX on any Linux. POSIX shell; dash/mksh/busybox safe.
#
# One-liner:
#   curl -fsSL https://castle.great-morpho.ts.net:3000/jonerik/anvil/raw/branch/main/install.sh | sh
#
# Default install set: all binaries (with their internal package symlinks
# mkfs.ext{2,3} -> mkfs.ext4, fsck.ext{2,3,4} -> e2fsck, mkfs.{fat,msdos} +
# mkdosfs -> mkfs.vfat, mklost+found -> mklostfound), MIT LICENSE, and man
# pages — all under $PREFIX. The installer never writes outside $PREFIX,
# never touches /sbin or /etc, and never creates system-level symlinks that
# shadow GNU e2fsprogs (see the post-install trailer for guidance).
#
# Flags:
#   --version <V>           anvil version (default 0.2.1-r1)
#   --prefix  <DIR>         install root  (default /usr/local)
#   --arch    <A>           override arch (default: uname -m)
#   --install-etc           also stage etc/ from the payload to $PREFIX/etc-default/
#   --no-install-etc        skip the etc-default stage even if etc/ is in the payload
#   --no-prompt, --yes      never prompt; accept defaults (no etc-default unless --install-etc)
#   -h, --help              print this help
#
# Long-form --key=value is also accepted (e.g. --prefix=$HOME/.local).

set -eu

VERSION="0.2.1-r1"
PREFIX="/usr/local"
ARCH=""
INSTALL_ETC=""        # "" = unset (ask), 1 = yes, 0 = no
NO_PROMPT=0
URL_BASE="${ANVIL_INSTALL_URL_BASE:-https://github.com/stormj-UH/jonerix/releases/download/packages}"

usage() {
    cat <<'EOF'
anvil installer

Usage: install.sh [--version V] [--prefix DIR] [--arch A]
                  [--install-etc | --no-install-etc]
                  [--no-prompt | --yes] [-h]

Default install (no flags) puts the full set under $PREFIX:
  - all binaries (mkfs.ext4, e2fsck, tune2fs, debugfs, dumpe2fs, resize2fs,
    e4defrag, e2image, e2label, e2freefrag, logsave, findfs, filefrag,
    blkid, chattr, lsattr, mklostfound, mkfs.vfat, ...)
  - internal package symlinks: mkfs.ext{2,3} -> mkfs.ext4,
    fsck.ext{2,3,4} -> e2fsck, mkfs.{fat,msdos} + mkdosfs -> mkfs.vfat,
    mklost+found -> mklostfound (all live in $PREFIX/bin alongside their
    targets — these are NOT system-level symlinks)
  - LICENSE under $PREFIX/share/licenses/anvil/
  - man pages under $PREFIX/share/man/

The installer NEVER writes outside $PREFIX. It does NOT install etc/
defaults by default — pass --install-etc to stage them to
$PREFIX/etc-default/ for manual review.

Flags:
  --version <V>       .jpkg version to fetch (default 0.2.1-r1)
  --prefix  <DIR>     install root (default /usr/local)
  --arch    <A>       arch override (default: detected via uname -m)
  --install-etc       stage etc/ from the payload to $PREFIX/etc-default/
                      (does NOT copy into $PREFIX/etc; you copy from
                      etc-default/ yourself if you want them live)
  --no-install-etc    skip the etc-default stage even if etc/ is in the payload
  --no-prompt, --yes  non-interactive; accept defaults (no etc-default unless
                      --install-etc was also passed)
  -h, --help          this message

Examples:
  sh install.sh
  sh install.sh --prefix=$HOME/.local
  sh install.sh --version 0.2.1-r1 --arch aarch64
  sh install.sh --install-etc --no-prompt
EOF
}

# --- argument parse (POSIX, supports --key value and --key=value) -----------
while [ $# -gt 0 ]; do
    case "$1" in
        --version)         VERSION="$2"; shift 2 ;;
        --version=*)       VERSION="${1#--version=}"; shift ;;
        --prefix)          PREFIX="$2"; shift 2 ;;
        --prefix=*)        PREFIX="${1#--prefix=}"; shift ;;
        --arch)            ARCH="$2"; shift 2 ;;
        --arch=*)          ARCH="${1#--arch=}"; shift ;;
        --install-etc)     INSTALL_ETC=1; shift ;;
        --no-install-etc)  INSTALL_ETC=0; shift ;;
        --no-prompt|--yes) NO_PROMPT=1; shift ;;
        -h|--help)         usage; exit 0 ;;
        --)                shift; break ;;
        *)                 printf 'install.sh: unknown argument: %s\n' "$1" >&2
                           usage >&2; exit 2 ;;
    esac
done

# --- arch detection ---------------------------------------------------------
if [ -z "$ARCH" ]; then
    UM=$(uname -m)
    case "$UM" in
        x86_64|amd64)   ARCH="x86_64" ;;
        aarch64|arm64)  ARCH="aarch64" ;;
        *)
            printf 'install.sh: unsupported arch %s\n' "$UM" >&2
            printf '  supported: x86_64, aarch64 (override with --arch)\n' >&2
            exit 1
            ;;
    esac
fi

# --- tool checks ------------------------------------------------------------
need_one() {
    # need_one tool1 tool2 ...  — succeed if any is on PATH
    for t in "$@"; do
        if command -v "$t" >/dev/null 2>&1; then return 0; fi
    done
    printf 'install.sh: need one of:' >&2
    for t in "$@"; do printf ' %s' "$t" >&2; done
    printf '\n' >&2
    return 1
}
need() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'install.sh: missing required tool: %s\n' "$1" >&2
        return 1
    fi
}
MISSING=0
need_one curl wget || MISSING=1
need zstd || MISSING=1
need tar  || MISSING=1
need od   || MISSING=1
need dd   || MISSING=1
[ "$MISSING" -eq 0 ] || exit 1

if command -v curl >/dev/null 2>&1; then
    DL="curl -fsSL -o"
else
    DL="wget -qO"
fi

# --- workspace --------------------------------------------------------------
TMP=$(mktemp -d 2>/dev/null || mktemp -d -t anvil-install)
# shellcheck disable=SC2329  # invoked indirectly via trap
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT INT HUP TERM

JPKG="$TMP/anvil.jpkg"
EXTRACT="$TMP/extract"
mkdir -p "$EXTRACT"

URL="$URL_BASE/anvil-$VERSION-$ARCH.jpkg"
printf 'install.sh: fetching %s\n' "$URL"
# shellcheck disable=SC2086
$DL "$JPKG" "$URL" || {
    printf 'install.sh: download failed: %s\n' "$URL" >&2
    exit 1
}

# --- magic + metadata length ------------------------------------------------
MAGIC=$(dd if="$JPKG" bs=1 count=4 status=none | od -An -c | tr -d ' \n')
case "$MAGIC" in
    JPKG) ;;
    *)
        printf 'install.sh: %s is not a JPKG file (magic: %s)\n' "$JPKG" "$MAGIC" >&2
        exit 1
        ;;
esac

# Read a little-endian u32 from $JPKG at byte offset $1.
# Portable across BSD od (default big-endian) and GNU od by reading bytes
# one at a time and combining.
read_le_u32() {
    _off="$1"
    # shellcheck disable=SC2046
    set -- $(od -An -tu1 -N4 -j"$_off" "$JPKG")
    [ $# -eq 4 ] || { printf 'install.sh: short read at offset %s\n' "$_off" >&2; exit 1; }
    echo $(( $1 + $2 * 256 + $3 * 65536 + $4 * 16777216 ))
}

# Format-version field at bytes 4..7. The published 0.2.1-r1 packages encode
# this as a 16-bit value with two trailing zero bytes (so a naive LE u32 read
# yields 256), but the meaningful guard for a valid JPKG is the magic above
# and a sane metadata length below. We surface unfamiliar values as a notice
# rather than aborting, so future format bumps don't brick this installer.
VER=$(read_le_u32 4)
case "$VER" in
    1|256) ;;
    *) printf 'install.sh: notice: unfamiliar JPKG format version %s; continuing\n' "$VER" >&2 ;;
esac

MD_LEN=$(read_le_u32 8)
if [ -z "$MD_LEN" ] || [ "$MD_LEN" -le 0 ]; then
    printf 'install.sh: bad metadata length: %s\n' "$MD_LEN" >&2
    exit 1
fi

PAYLOAD_OFFSET=$((12 + MD_LEN))

printf 'install.sh: extracting payload (offset %s)\n' "$PAYLOAD_OFFSET"
dd if="$JPKG" bs=1 skip="$PAYLOAD_OFFSET" status=none \
    | zstd -d -q \
    | tar -x -C "$EXTRACT"

# --- install ----------------------------------------------------------------
# Per the recipe, the merged-usr flatten leaves bin/, lib/, share/, etc/ at
# the payload root. Default: install all binary and share dirs under $PREFIX.
# etc/ is NOT installed by default — pass --install-etc to stage to
# $PREFIX/etc-default/, or answer the prompt yes (when running interactively).

if [ ! -d "$EXTRACT" ] || [ -z "$(ls -A "$EXTRACT" 2>/dev/null)" ]; then
    printf 'install.sh: extracted payload is empty\n' >&2
    exit 1
fi

mkdir -p "$PREFIX"

# Detect whether the payload actually carries an etc/ tree. If not, the
# whole etc-default question is moot and we don't prompt.
PAYLOAD_HAS_ETC=0
if [ -d "$EXTRACT/etc" ] && [ -n "$(ls -A "$EXTRACT/etc" 2>/dev/null)" ]; then
    PAYLOAD_HAS_ETC=1
fi

# Resolve INSTALL_ETC: if unset and payload has etc/ and stdin is a tty and
# we weren't told --no-prompt, ask the user. Otherwise default to 0 (skip).
if [ -z "$INSTALL_ETC" ]; then
    if [ "$PAYLOAD_HAS_ETC" -eq 1 ] && [ "$NO_PROMPT" -eq 0 ] && [ -t 0 ]; then
        printf 'Install /etc-default templates from the package payload to %s/etc-default/?\n' "$PREFIX"
        printf '  (You can review and copy to %s/etc/ yourself.) [y/N] ' "$PREFIX"
        # POSIX read; quote -r-equivalent is implicit here since we're not
        # processing escapes. mksh/dash both accept this form.
        REPLY=""
        read -r REPLY || REPLY=""
        case "$REPLY" in
            y|Y|yes|YES|Yes) INSTALL_ETC=1 ;;
            *)               INSTALL_ETC=0 ;;
        esac
    else
        INSTALL_ETC=0
    fi
fi

INSTALLED_DIRS=""
DECLINED_ETC=0
for src in "$EXTRACT"/*; do
    [ -d "$src" ] || continue
    name=$(basename "$src")
    case "$name" in
        bin|sbin|lib|lib32|lib64|libexec|share|include|var)
            dst="$PREFIX/$name"
            mkdir -p "$dst"
            # cp -a preserves symlinks (the internal package symlinks like
            # mkfs.ext2 -> mkfs.ext4 ride along here), modes, and timestamps.
            cp -a "$src/." "$dst/"
            INSTALLED_DIRS="$INSTALLED_DIRS $name"
            ;;
        etc)
            if [ "$INSTALL_ETC" -eq 1 ]; then
                # Stage to etc-default/, NOT etc/. The operator merges into
                # $PREFIX/etc/ (or system /etc) by hand if they want them live.
                dst="$PREFIX/etc-default"
                mkdir -p "$dst"
                cp -a "$src/." "$dst/"
                INSTALLED_DIRS="$INSTALLED_DIRS etc-default"
            else
                DECLINED_ETC=1
            fi
            ;;
        *)
            # Unknown top-level dir — copy under the same name to be safe.
            # Stays inside $PREFIX, so this is still a no-system-touch install.
            dst="$PREFIX/$name"
            mkdir -p "$dst"
            cp -a "$src/." "$dst/"
            INSTALLED_DIRS="$INSTALLED_DIRS $name"
            ;;
    esac
done

# --- per-binary sanity check ------------------------------------------------
BIN="$PREFIX/bin"
SANITY_OK=1
if [ -d "$BIN" ]; then
    for b in "$BIN"/*; do
        [ -f "$b" ] || continue
        [ -x "$b" ] || continue
        # mklost+found and the chattr/lsattr tools have no --version flag and
        # accept --help. Try --version, fall back to --help; suppress output.
        if "$b" --version >/dev/null 2>&1; then
            :
        elif "$b" --help >/dev/null 2>&1; then
            :
        else
            printf 'install.sh: warning: %s did not respond to --version/--help\n' "$b" >&2
            SANITY_OK=0
        fi
    done
fi

# --- post-install trailer ---------------------------------------------------
printf '\n'
printf '==========================================================================\n'
printf 'install.sh: anvil %s installed under %s\n' "$VERSION" "$PREFIX"
printf '==========================================================================\n'

# Summary of installed paths.
printf '\nInstalled paths:\n'
for d in $INSTALLED_DIRS; do
    printf '  %s/%s/\n' "$PREFIX" "$d"
done
if [ -f "$PREFIX/share/licenses/anvil/LICENSE" ]; then
    printf '  %s/share/licenses/anvil/LICENSE\n' "$PREFIX"
fi

# PATH warning.
case ":${PATH:-}:" in
    *":$BIN:"*)
        :
        ;;
    *)
        # shellcheck disable=SC2016  # literal $PATH for user-facing instruction
        printf '\nNOTE: %s is not on $PATH.\n' "$BIN"
        printf '      Add it with:\n'
        # shellcheck disable=SC2016  # literal $PATH for user-facing instruction
        printf '        export PATH="%s:$PATH"\n' "$BIN"
        ;;
esac

# e2fsprogs independence note.
printf '\nIMPORTANT: anvil is INDEPENDENT of GNU e2fsprogs.\n'
printf '  anvil ships its own mkfs.ext4, e2fsck, tune2fs, debugfs, etc.\n'
printf '  in %s/bin/. They do NOT shadow /sbin/mkfs.ext4 from a system\n' "$PREFIX"
printf '  e2fsprogs install — system tools (mount, mkfs, fsck wrapper) look\n'
printf '  in /sbin first.\n'
printf '\n'
printf '  To use anvil in place of e2fsprogs system-wide, you would have to\n'
printf '  manually symlink the binaries into /sbin yourself, e.g.:\n'
printf '\n'
printf '      sudo ln -sf %s/bin/mkfs.ext4 /sbin/mkfs.ext4\n' "$PREFIX"
printf '      sudo ln -sf %s/bin/e2fsck    /sbin/e2fsck\n' "$PREFIX"
printf '      # ...and so on for the others.\n'
printf '\n'
printf '  That is OUTSIDE the scope of this installer. This script never\n'
printf '  writes outside %s.\n' "$PREFIX"

# etc-default guidance.
if [ "$DECLINED_ETC" -eq 1 ]; then
    printf '\nNOTE: etc/ defaults from the payload were NOT staged.\n'
    printf '      Re-run with --install-etc to stage them to %s/etc-default/\n' "$PREFIX"
    printf '      for manual review, then copy into %s/etc/ (or your system\n' "$PREFIX"
    printf '      /etc/) yourself if you want them live.\n'
elif [ "$INSTALL_ETC" -eq 1 ] && [ "$PAYLOAD_HAS_ETC" -eq 1 ]; then
    printf '\nNOTE: etc/ defaults staged to %s/etc-default/.\n' "$PREFIX"
    printf '      They are NOT live. Review them and copy into %s/etc/ (or\n' "$PREFIX"
    printf '      your system /etc/) yourself if you want them live.\n'
fi

[ "$SANITY_OK" -eq 1 ] || \
    printf '\nWARNING: some binaries failed sanity check (see warnings above)\n' >&2

printf '\n'
exit 0
