#!/bin/sh
# install.sh — jcarp curl|sh installer (POSIX, dash/mksh/busybox compatible)
#
# Downloads a published jcarp .jpkg from the stormj-UH/jonerix release pool,
# verifies the JPKG header magic, extracts the zstd-compressed tar payload,
# and installs the daemon plus its OpenRC service file, default config, and
# license to a configurable prefix.
#
# Usage:
#   curl -sSfL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/extra/jcarp/install.sh | sh
#   sh install.sh [--version VER] [--prefix DIR] [--arch ARCH] [--help]
#
# Options:
#   --version VER   jcarp .jpkg version to fetch       (default: 0.1.0-r1)
#   --prefix DIR    install root                       (default: /usr/local)
#   --arch ARCH     aarch64 | x86_64                   (default: uname -m)
#   --help          show usage and exit
#
# JPKG layout (see scripts/gen-index.sh in the jonerix repo):
#   bytes  0..3   "JPKG" ASCII magic
#   bytes  4..7   format version le u32 (currently 1)
#   bytes  8..11  metadata length le u32
#   bytes 12..    TOML metadata, then zstd-compressed tar.

set -eu

# ── defaults ────────────────────────────────────────────────────────────────
DEFAULT_VERSION="0.1.0-r1"
RELEASE_TAG="packages"
RELEASE_BASE="https://github.com/stormj-UH/jonerix/releases/download/${RELEASE_TAG}"

VERSION="$DEFAULT_VERSION"
PREFIX="/usr/local"
ARCH=""

PROG="install.sh"

# ── helpers ─────────────────────────────────────────────────────────────────
log()  { printf '[jcarp] %s\n' "$*"; }
warn() { printf '[jcarp] warning: %s\n' "$*" >&2; }
die()  { printf '[jcarp] error: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<EOF
${PROG} — install jcarp from a published .jpkg

Usage:
  ${PROG} [--version VER] [--prefix DIR] [--arch ARCH] [--help]

Options:
  --version VER   jcarp version to install   (default: ${DEFAULT_VERSION})
  --prefix DIR    install root               (default: /usr/local)
  --arch ARCH     aarch64 | x86_64           (default: detected)
  --help          show this message

The package is fetched from:
  ${RELEASE_BASE}/jcarp-<VERSION>-<ARCH>.jpkg

After install, copy \${PREFIX}/etc/jcarp/jcarp.conf.default to
\${PREFIX}/etc/jcarp/jcarp.conf, edit it, then start the service.
EOF
}

# ── argument parsing (long --key value and --key=value) ─────────────────────
while [ $# -gt 0 ]; do
    case "$1" in
        --version)  VERSION="${2:?--version requires a value}"; shift 2 ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --prefix)   PREFIX="${2:?--prefix requires a value}"; shift 2 ;;
        --prefix=*) PREFIX="${1#*=}"; shift ;;
        --arch)     ARCH="${2:?--arch requires a value}"; shift 2 ;;
        --arch=*)   ARCH="${1#*=}"; shift ;;
        -h|--help)  usage; exit 0 ;;
        --)         shift; break ;;
        *)          die "unknown option: $1 (try --help)" ;;
    esac
done

# ── arch detect ─────────────────────────────────────────────────────────────
if [ -z "$ARCH" ]; then
    uname_m=$(uname -m 2>/dev/null || echo unknown)
    case "$uname_m" in
        aarch64|arm64)        ARCH=aarch64 ;;
        x86_64|amd64)         ARCH=x86_64 ;;
        *) die "unsupported architecture '$uname_m' — pass --arch aarch64|x86_64" ;;
    esac
fi
case "$ARCH" in
    aarch64|x86_64) ;;
    *) die "unsupported --arch '$ARCH' (must be aarch64 or x86_64)" ;;
esac

# ── tool check ──────────────────────────────────────────────────────────────
have() { command -v "$1" >/dev/null 2>&1; }

DOWNLOADER=""
if   have curl; then DOWNLOADER=curl
elif have wget; then DOWNLOADER=wget
fi

missing=""
[ -n "$DOWNLOADER" ] || missing="$missing curl-or-wget"
have zstd  || missing="$missing zstd"
have tar   || missing="$missing tar"
have od    || missing="$missing od"
have dd    || missing="$missing dd"
have install || missing="$missing install"

if [ -n "$missing" ]; then
    warn "missing required tools:$missing"
    cat >&2 <<'HINT'

Install hints:
  Alpine / jonerix : apk add curl zstd tar coreutils
  Debian / Ubuntu  : apt-get install -y curl zstd tar coreutils
  Fedora / RHEL    : dnf install -y curl zstd tar coreutils
  Arch             : pacman -S --noconfirm curl zstd tar coreutils
  macOS            : brew install curl zstd gnu-tar coreutils
HINT
    exit 1
fi

# ── workspace ───────────────────────────────────────────────────────────────
WORKDIR=$(mktemp -d 2>/dev/null) || die "mktemp -d failed"
cleanup() { rm -rf "$WORKDIR"; }
trap cleanup EXIT INT TERM HUP

JPKG="$WORKDIR/jcarp.jpkg"
PAYLOAD_DIR="$WORKDIR/payload"
mkdir -p "$PAYLOAD_DIR"

URL="${RELEASE_BASE}/jcarp-${VERSION}-${ARCH}.jpkg"
log "fetching ${URL}"

if [ "$DOWNLOADER" = curl ]; then
    curl -fSL --retry 3 --retry-delay 2 -o "$JPKG" "$URL" \
        || die "download failed: $URL"
else
    wget -q -O "$JPKG" "$URL" \
        || die "download failed: $URL"
fi

[ -s "$JPKG" ] || die "downloaded file is empty: $JPKG"

# ── verify magic ────────────────────────────────────────────────────────────
magic=$(dd if="$JPKG" bs=1 count=4 2>/dev/null || true)
if [ "$magic" != "JPKG" ]; then
    die "bad magic: not a JPKG archive (got: $(printf '%s' "$magic" | od -An -c | tr -s ' '))"
fi

# ── extract ─────────────────────────────────────────────────────────────────
md_len=$(dd if="$JPKG" bs=1 skip=8 count=4 2>/dev/null | od -An -tu4 | tr -d ' \n')
case "$md_len" in
    ''|*[!0-9]*) die "invalid metadata length in JPKG header" ;;
esac
[ "$md_len" -gt 0 ] || die "metadata length is zero"

log "extracting payload (metadata $md_len bytes)"
dd if="$JPKG" bs=1 skip=$((12 + md_len)) status=none 2>/dev/null \
    | zstd -d -q \
    | tar -x -C "$PAYLOAD_DIR" \
    || die "extract failed (zstd|tar)"

[ -f "$PAYLOAD_DIR/bin/jcarp" ] || die "payload missing bin/jcarp"

# ── install ─────────────────────────────────────────────────────────────────
log "installing into ${PREFIX}"

install -d "$PREFIX/bin" "$PREFIX/etc/jcarp" "$PREFIX/share/licenses/jcarp"

install -m 755 "$PAYLOAD_DIR/bin/jcarp" "$PREFIX/bin/jcarp"

if [ -f "$PAYLOAD_DIR/etc/init.d/jcarp" ]; then
    install -d "$PREFIX/etc/init.d"
    install -m 755 "$PAYLOAD_DIR/etc/init.d/jcarp" "$PREFIX/etc/init.d/jcarp"
fi

# Config goes to .default — never clobber an existing operator config.
if [ -f "$PAYLOAD_DIR/etc/jcarp/jcarp.conf.default" ]; then
    install -m 644 "$PAYLOAD_DIR/etc/jcarp/jcarp.conf.default" \
        "$PREFIX/etc/jcarp/jcarp.conf.default"
elif [ -f "$PAYLOAD_DIR/etc/jcarp/jcarp.conf" ]; then
    install -m 644 "$PAYLOAD_DIR/etc/jcarp/jcarp.conf" \
        "$PREFIX/etc/jcarp/jcarp.conf.default"
fi

if [ -f "$PAYLOAD_DIR/share/licenses/jcarp/LICENSE" ]; then
    install -m 644 "$PAYLOAD_DIR/share/licenses/jcarp/LICENSE" \
        "$PREFIX/share/licenses/jcarp/LICENSE"
fi

# ── sanity check ────────────────────────────────────────────────────────────
log "running sanity check: $PREFIX/bin/jcarp --version"
if "$PREFIX/bin/jcarp" --version >/dev/null 2>&1; then
    "$PREFIX/bin/jcarp" --version || true
elif "$PREFIX/bin/jcarp" --help >/dev/null 2>&1; then
    log "--version unsupported, --help OK"
else
    warn "sanity check failed: cannot invoke $PREFIX/bin/jcarp"
    warn "(this may be normal if the host arch differs from the binary)"
fi

# ── post-install message ────────────────────────────────────────────────────
cat <<EOF

jcarp ${VERSION} (${ARCH}) installed under ${PREFIX}

Files:
  ${PREFIX}/bin/jcarp
  ${PREFIX}/etc/jcarp/jcarp.conf.default
  ${PREFIX}/etc/init.d/jcarp                  (OpenRC service, if shipped)
  ${PREFIX}/share/licenses/jcarp/LICENSE

jcarp is a daemon — it will not run until configured. To finish setup:

  1. Copy and edit the config:
        cp ${PREFIX}/etc/jcarp/jcarp.conf.default ${PREFIX}/etc/jcarp/jcarp.conf
        \$EDITOR ${PREFIX}/etc/jcarp/jcarp.conf

  2. Privileges. CARP/VRRP needs raw sockets and link-layer access:
        setcap 'cap_net_admin,cap_net_raw=ep' ${PREFIX}/bin/jcarp
     ...or run jcarp as root via the supplied service.

  3. Service management:
        OpenRC (Alpine / jonerix):
            rc-update add jcarp default
            rc-service jcarp start
        systemd: see the jonerix package README for a one-shot unit pattern.

Done.
EOF
