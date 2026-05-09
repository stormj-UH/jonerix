#!/bin/sh
# install.sh - Install stormwall (drop-in nft/iptables/pfctl front-end) on any Linux.
#
# Default install: stormwall + pfctl binaries, license, man pages. No symlinks,
# no system-level changes. Compatibility symlinks (nft/iptables/...) are opt-in
# via --with-symlinks or the interactive prompt.
#
# Usage:
#   curl -fsSL <url> | sh
#   curl -fsSL <url> | sh -s -- --with-symlinks
#   sh install.sh [options]
#
# Strict POSIX shell -- validated with dash -n, mksh -n, shellcheck -s sh.
# No bashisms (no [[, no <(), no (()), no `local`, no arrays, no `+=`).

set -eu

DEFAULT_VERSION="1.1.0"
RELEASE_URL_BASE="https://github.com/stormj-UH/jonerix/releases/download/packages"

VERSION="$DEFAULT_VERSION"
PREFIX="/usr/local"
ARCH=""
WITH_SYMLINKS=0       # 0=no symlinks, 1=create symlinks
SYMLINKS_EXPLICIT=0   # 1 if user passed --with-symlinks or --no-symlinks
NO_PROMPT=0           # 1 if user passed --no-prompt / --yes

# argv[0] dispatch names that the stormwall binary recognises.
SYMLINK_NAMES="nft iptables iptables-save iptables-restore ip6tables ip6tables-save ip6tables-restore"

usage() {
    cat <<'EOF'
stormwall installer

Usage:
  install.sh [options]

By default, only the stormwall and pfctl binaries (plus shipped license and
man pages) are installed under $PREFIX. No compatibility symlinks are created
and no system paths (/usr/sbin, /sbin, /etc) are touched.

Options:
  --version VER         install stormwall version VER (default: 1.1.0)
  --prefix DIR          install under DIR/bin and DIR/share (default: /usr/local)
  --arch ARCH           override autodetect; one of: aarch64, x86_64
  --with-symlinks       also create nft/iptables/ip6tables/iptables-save/
                        iptables-restore/ip6tables-save/ip6tables-restore
                        symlinks under $PREFIX/bin (all -> stormwall)
  --no-symlinks         do not create the dispatch symlinks (this is the
                        default; flag exists for explicit, scriptable opt-out)
  --no-prompt, --yes    never prompt; use defaults or whatever flags imply
  -h, --help            show this help and exit

Long-form --key=value is accepted for every flag that takes an argument.

Examples:
  sh install.sh
  sh install.sh --prefix "$HOME/.local"
  sh install.sh --version 1.1.0 --arch x86_64
  sh install.sh --with-symlinks --no-prompt
EOF
}

die() {
    printf '%s: %s\n' "install.sh" "$*" >&2
    exit 1
}

# ---------- argument parsing ----------
while [ $# -gt 0 ]; do
    case "$1" in
        --version)        [ $# -ge 2 ] || die "--version needs an argument"; VERSION="$2"; shift 2 ;;
        --version=*)      VERSION="${1#--version=}"; shift ;;
        --prefix)         [ $# -ge 2 ] || die "--prefix needs an argument";  PREFIX="$2";  shift 2 ;;
        --prefix=*)       PREFIX="${1#--prefix=}"; shift ;;
        --arch)           [ $# -ge 2 ] || die "--arch needs an argument";    ARCH="$2";    shift 2 ;;
        --arch=*)         ARCH="${1#--arch=}"; shift ;;
        --with-symlinks)  WITH_SYMLINKS=1; SYMLINKS_EXPLICIT=1; shift ;;
        --no-symlinks)    WITH_SYMLINKS=0; SYMLINKS_EXPLICIT=1; shift ;;
        --no-prompt|--yes) NO_PROMPT=1; shift ;;
        -h|--help)        usage; exit 0 ;;
        --)               shift; break ;;
        -*)               die "unknown option: $1 (try --help)" ;;
        *)                die "unexpected argument: $1 (try --help)" ;;
    esac
done

# ---------- arch autodetect ----------
if [ -z "$ARCH" ]; then
    UM=$(uname -m 2>/dev/null || echo unknown)
    case "$UM" in
        x86_64|amd64)   ARCH="x86_64" ;;
        aarch64|arm64)  ARCH="aarch64" ;;
        *) die "unsupported architecture: $UM (use --arch x86_64 or --arch aarch64)" ;;
    esac
else
    case "$ARCH" in
        x86_64|aarch64) : ;;
        *) die "invalid --arch: $ARCH (must be x86_64 or aarch64)" ;;
    esac
fi

# ---------- downloader + tool check ----------
DL=""
if command -v curl >/dev/null 2>&1; then
    DL="curl"
elif command -v wget >/dev/null 2>&1; then
    DL="wget"
fi

need_tool() {
    _tool="$1"
    _hint="$2"
    if ! command -v "$_tool" >/dev/null 2>&1; then
        printf 'install.sh: missing required tool: %s\n' "$_tool" >&2
        printf '  install via one of:\n' >&2
        printf '    apt install %s    # Debian/Ubuntu\n' "$_hint" >&2
        printf '    dnf install %s    # Fedora/RHEL\n' "$_hint" >&2
        printf '    apk add %s        # Alpine\n' "$_hint" >&2
        printf '    pkg install %s    # FreeBSD\n' "$_hint" >&2
        exit 1
    fi
}

if [ -z "$DL" ]; then
    printf 'install.sh: need either curl or wget to download packages\n' >&2
    printf '  install via:  apt install curl    (or:  apk add curl)\n' >&2
    exit 1
fi

need_tool zstd zstd
need_tool tar tar
need_tool od coreutils
need_tool dd coreutils
need_tool install coreutils

# ---------- interactive prompt for symlinks ----------
# Only prompt when:
#   - the user did not pass --with-symlinks or --no-symlinks
#   - the user did not pass --no-prompt / --yes
#   - stdout is a terminal AND /dev/tty is readable (so we can prompt even
#     when stdin is the curl pipe in `curl ... | sh`)
if [ "$SYMLINKS_EXPLICIT" -eq 0 ] && [ "$NO_PROMPT" -eq 0 ] \
   && [ -t 1 ] && [ -r /dev/tty ]; then
    printf 'Create iptables/nft/ip6tables compatibility symlinks under %s/bin? [y/N] ' "$PREFIX"
    REPLY=""
    # shellcheck disable=SC2162
    read REPLY < /dev/tty || REPLY=""
    case "$REPLY" in
        y|Y|yes|YES|Yes) WITH_SYMLINKS=1 ;;
        *)               WITH_SYMLINKS=0 ;;
    esac
fi

# ---------- workdir + traps ----------
TMP=$(mktemp -d 2>/dev/null || mktemp -d -t stormwall-install)
[ -n "$TMP" ] && [ -d "$TMP" ] || die "could not create temp directory"
trap 'rm -rf "$TMP"' EXIT
trap 'rm -rf "$TMP"; exit 130' INT
trap 'rm -rf "$TMP"; exit 143' TERM
trap 'rm -rf "$TMP"; exit 129' HUP

# ---------- download ----------
JPKG_NAME="stormwall-${VERSION}-${ARCH}.jpkg"
JPKG_URL="${RELEASE_URL_BASE}/${JPKG_NAME}"
JPKG="$TMP/$JPKG_NAME"

# A local file with the same name (handy for offline/dev installs).
if [ -f "./$JPKG_NAME" ]; then
    printf 'stormwall: using local %s\n' "./$JPKG_NAME"
    cp "./$JPKG_NAME" "$JPKG" || die "could not copy ./$JPKG_NAME"
else
    printf 'stormwall: downloading %s\n' "$JPKG_URL"
    if [ "$DL" = "curl" ]; then
        curl -fsSL -o "$JPKG" "$JPKG_URL" || die "download failed: $JPKG_URL"
    else
        wget -q -O "$JPKG" "$JPKG_URL" || die "download failed: $JPKG_URL"
    fi
fi

# ---------- JPKG magic + header ----------
MAGIC=$(dd if="$JPKG" bs=1 count=4 status=none 2>/dev/null || true)
if [ "$MAGIC" != "JPKG" ]; then
    die "downloaded file is not a JPKG package (bad magic): $JPKG_URL"
fi

# Layout: bytes 0-3 magic, 4-7 format version (LE u32), 8-11 metadata length
# (LE u32), then metadata, then zstd-compressed tar.
MD_LEN=$(od -An -tu4 -N4 -j8 "$JPKG" | tr -d ' \t\n')
case "$MD_LEN" in
    ''|*[!0-9]*) die "could not read metadata length from $JPKG_NAME" ;;
esac
PAYLOAD_OFFSET=$((12 + MD_LEN))

EXTRACT_DIR="$TMP/extract"
mkdir -p "$EXTRACT_DIR"

printf 'stormwall: decompressing payload\n'
dd if="$JPKG" bs=1 skip="$PAYLOAD_OFFSET" status=none 2>/dev/null \
    | zstd -d -q \
    | tar -x -C "$EXTRACT_DIR" \
    || die "failed to decompress or untar payload"

# ---------- locate shipped files ----------
SRC_STORMWALL="$EXTRACT_DIR/bin/stormwall"
SRC_PFCTL="$EXTRACT_DIR/bin/pfctl"
SRC_LICENSE="$EXTRACT_DIR/share/licenses/stormwall/LICENSE"
SRC_MAN_DIR="$EXTRACT_DIR/share/man"

[ -f "$SRC_STORMWALL" ] || die "package missing bin/stormwall"
# pfctl is a real binary in the package, not a symlink.
[ -f "$SRC_PFCTL" ] || die "package missing bin/pfctl"
[ -L "$SRC_PFCTL" ] && die "package bin/pfctl is a symlink (must be a real binary)"

# ---------- install ----------
DEST_BIN="$PREFIX/bin"
DEST_LICENSE_DIR="$PREFIX/share/licenses/stormwall"
DEST_MAN_DIR="$PREFIX/share/man"

printf 'stormwall: installing to %s\n' "$PREFIX"
install -d "$DEST_BIN" || die "could not create $DEST_BIN (try sudo or --prefix=\$HOME/.local)"

install -m 0755 "$SRC_STORMWALL" "$DEST_BIN/stormwall" || die "install of stormwall failed"
install -m 0755 "$SRC_PFCTL" "$DEST_BIN/pfctl" || die "install of pfctl failed"

INSTALLED_LICENSE=""
if [ -f "$SRC_LICENSE" ]; then
    install -d "$DEST_LICENSE_DIR" || die "could not create $DEST_LICENSE_DIR"
    install -m 0644 "$SRC_LICENSE" "$DEST_LICENSE_DIR/LICENSE" || die "install of LICENSE failed"
    INSTALLED_LICENSE="$DEST_LICENSE_DIR/LICENSE"
fi

INSTALLED_MAN_COUNT=0
if [ -d "$SRC_MAN_DIR" ]; then
    # Walk every regular file under share/man/ and install it with the same
    # subdirectory layout under $PREFIX/share/man/.
    install -d "$DEST_MAN_DIR" || die "could not create $DEST_MAN_DIR"
    # Use find -print0-free traversal: POSIX find with -type f, ordinary read.
    # File names with newlines are extremely unusual in man pages.
    find "$SRC_MAN_DIR" -type f -print | while IFS= read -r SRC_F; do
        REL=${SRC_F#"$SRC_MAN_DIR/"}
        DEST_F="$DEST_MAN_DIR/$REL"
        DEST_F_DIR=${DEST_F%/*}
        install -d "$DEST_F_DIR" || die "could not create $DEST_F_DIR"
        install -m 0644 "$SRC_F" "$DEST_F" || die "install of man page failed: $REL"
    done
    # Re-count outside the subshell.
    INSTALLED_MAN_COUNT=$(find "$DEST_MAN_DIR" -type f 2>/dev/null | wc -l | tr -d ' ')
fi

# Optional dispatch symlinks (relative target so the link works under any PREFIX).
INSTALLED_SYMLINKS=""
if [ "$WITH_SYMLINKS" -eq 1 ]; then
    for name in $SYMLINK_NAMES; do
        ln -sf stormwall "$DEST_BIN/$name" || die "could not symlink $DEST_BIN/$name"
        INSTALLED_SYMLINKS="$INSTALLED_SYMLINKS $DEST_BIN/$name"
    done
fi

# ---------- verify ----------
printf 'stormwall: verifying %s/bin/stormwall --version\n' "$PREFIX"
VERSION_OUT=""
if VERSION_OUT=$("$DEST_BIN/stormwall" --version 2>&1); then
    case "$VERSION_OUT" in
        *stormwall*) printf '  %s\n' "$VERSION_OUT" ;;
        *) die "version check failed: got '$VERSION_OUT'" ;;
    esac
else
    die "could not run $DEST_BIN/stormwall --version"
fi

# ---------- post-install trailer ----------
printf '\nSummary\n'
printf '  %s\n' "$DEST_BIN/stormwall"
printf '  %s\n' "$DEST_BIN/pfctl"
if [ -n "$INSTALLED_LICENSE" ]; then
    printf '  %s\n' "$INSTALLED_LICENSE"
fi
if [ "$INSTALLED_MAN_COUNT" -gt 0 ]; then
    printf '  %s   (%s man page' "$DEST_MAN_DIR" "$INSTALLED_MAN_COUNT"
    [ "$INSTALLED_MAN_COUNT" -ne 1 ] && printf 's'
    printf ')\n'
fi
if [ -n "$INSTALLED_SYMLINKS" ]; then
    for s in $INSTALLED_SYMLINKS; do
        printf '  %s -> stormwall\n' "$s"
    done
fi

printf '\nRecommended next steps\n'

# 1. PATH advice if PREFIX/bin isn't on PATH already.
PATH_HAS_BIN=0
case ":${PATH:-}:" in
    *":$DEST_BIN:"*) PATH_HAS_BIN=1 ;;
esac
if [ "$PATH_HAS_BIN" -eq 0 ]; then
    # shellcheck disable=SC2016  # the literal '$PATH' is what we want printed.
    printf '  - %s is not on your $PATH. Add it with:\n' "$DEST_BIN"
    # shellcheck disable=SC2016
    printf '        export PATH="%s:$PATH"\n' "$DEST_BIN"
fi

# 2. Symlink advice depending on whether the user opted in.
if [ "$WITH_SYMLINKS" -eq 1 ]; then
    printf '  - Compatibility symlinks installed under %s.\n' "$DEST_BIN"
    printf '    For dockerd, fail2ban, ufw, or other tools that shell out to\n'
    printf '    /usr/sbin/iptables or /usr/sbin/nft, put %s BEFORE\n' "$DEST_BIN"
    # shellcheck disable=SC2016
    printf '    /usr/sbin in $PATH so the stormwall versions take precedence.\n'
else
    printf '  - To enable iptables/nft compatibility (so dockerd, scripts, and\n'
    printf '    other tools that shell out to /usr/sbin/iptables find stormwall),\n'
    printf '    re-run with --with-symlinks. This creates 7 symlinks under\n'
    printf '    %s pointing to stormwall:\n' "$DEST_BIN"
    for name in $SYMLINK_NAMES; do
        printf '        %s/%s\n' "$DEST_BIN" "$name"
    done
fi

printf '\nstormwall installed.\n'
