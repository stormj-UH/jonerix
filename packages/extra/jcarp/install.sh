#!/bin/sh
# install.sh — jcarp curl|sh installer (POSIX, dash/mksh/busybox compatible)
#
# Downloads a published jcarp .jpkg from the stormj-UH/jonerix release pool,
# verifies the JPKG header magic, extracts the zstd-compressed tar payload,
# and installs the daemon binary, license, and man pages to a configurable
# prefix. The OpenRC service file, default config, and CAP_NET_*  setcap
# step are opt-in (interactive prompt or CLI flag).
#
# Usage:
#   curl -sSfL https://raw.githubusercontent.com/stormj-UH/jonerix/main/packages/extra/jcarp/install.sh | sh
#   sh install.sh [options]
#
# Options:
#   --version VER         jcarp .jpkg version              (default: 0.1.0-r1)
#   --prefix DIR          install root                     (default: /usr/local)
#   --arch ARCH           aarch64 | x86_64                 (default: uname -m)
#   --with-init-script    install OpenRC service file
#   --no-init-script      do not install service (default)
#   --with-config         install default config
#   --no-config           do not install config (default)
#   --setcap              run setcap on installed binary
#   --no-setcap           skip setcap (default)
#   --no-prompt | --yes   non-interactive; honor flags only
#   --help                show usage and exit
#
# Long-form `--key=value` is also accepted for VER/DIR/ARCH options.
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

# Opt-ins. unset = "ask if interactive, else skip".
WITH_INITD=""
WITH_CONFIG=""
DO_SETCAP=""
NO_PROMPT=0

PROG="install.sh"

# ── helpers ─────────────────────────────────────────────────────────────────
log()  { printf '[jcarp] %s\n' "$*"; }
warn() { printf '[jcarp] warning: %s\n' "$*" >&2; }
die()  { printf '[jcarp] error: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<EOF
${PROG} — install jcarp from a published .jpkg

Usage:
  ${PROG} [options]

Options:
  --version VER         jcarp version            (default: ${DEFAULT_VERSION})
  --prefix DIR          install root             (default: /usr/local)
  --arch ARCH           aarch64 | x86_64         (default: detected)
  --with-init-script    install OpenRC service file
  --no-init-script      skip OpenRC service file (default)
  --with-config         install default config
  --no-config           skip default config (default)
  --setcap              run setcap cap_net_admin,cap_net_raw=ep on the binary
  --no-setcap           skip setcap (default)
  --no-prompt | --yes   non-interactive: honor flags only, no prompts
  --help                show this message

Default install lays down only:
  \${PREFIX}/bin/jcarp
  \${PREFIX}/share/licenses/jcarp/LICENSE
  \${PREFIX}/share/man/...                        (if shipped)

OpenRC service, default config, and setcap are opt-in.

The package is fetched from:
  ${RELEASE_BASE}/jcarp-<VERSION>-<ARCH>.jpkg
EOF
}

# ── argument parsing (long --key value and --key=value) ─────────────────────
while [ $# -gt 0 ]; do
    case "$1" in
        --version)             VERSION="${2:?--version requires a value}"; shift 2 ;;
        --version=*)           VERSION="${1#*=}"; shift ;;
        --prefix)              PREFIX="${2:?--prefix requires a value}"; shift 2 ;;
        --prefix=*)            PREFIX="${1#*=}"; shift ;;
        --arch)                ARCH="${2:?--arch requires a value}"; shift 2 ;;
        --arch=*)              ARCH="${1#*=}"; shift ;;
        --with-init-script)    WITH_INITD=1; shift ;;
        --no-init-script)      WITH_INITD=0; shift ;;
        --with-config)         WITH_CONFIG=1; shift ;;
        --no-config)           WITH_CONFIG=0; shift ;;
        --setcap)              DO_SETCAP=1; shift ;;
        --no-setcap)           DO_SETCAP=0; shift ;;
        --no-prompt|--yes)     NO_PROMPT=1; shift ;;
        -h|--help)             usage; exit 0 ;;
        --)                    shift; break ;;
        *)                     die "unknown option: $1 (try --help)" ;;
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
have zstd    || missing="$missing zstd"
have tar     || missing="$missing tar"
have od      || missing="$missing od"
have dd      || missing="$missing dd"
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

# ── interactive prompt helper ───────────────────────────────────────────────
# ask <question> -> sets ANSWER to 1 (yes) or 0 (no). Default no.
# Only prompts when stdin is a tty and /dev/tty is readable. Otherwise no.
ANSWER=0
ask() {
    ANSWER=0
    if [ "$NO_PROMPT" -eq 1 ]; then
        return 0
    fi
    if [ ! -t 1 ] || [ ! -r /dev/tty ]; then
        return 0
    fi
    printf '[jcarp] %s [y/N] ' "$1" >/dev/tty
    reply=""
    if ! IFS= read -r reply </dev/tty; then
        return 0
    fi
    case "$reply" in
        y|Y|yes|YES|Yes) ANSWER=1 ;;
        *)               ANSWER=0 ;;
    esac
}

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

# ── decide opt-ins (CLI flag wins; unset triggers prompt) ───────────────────
HAS_INITD=0
HAS_CONFIG=0
[ -f "$PAYLOAD_DIR/etc/init.d/jcarp" ]                  && HAS_INITD=1
[ -f "$PAYLOAD_DIR/etc/jcarp/jcarp.conf.default" ]      && HAS_CONFIG=1
[ "$HAS_CONFIG" -eq 0 ] && [ -f "$PAYLOAD_DIR/etc/jcarp/jcarp.conf" ] && HAS_CONFIG=1

if [ -z "$WITH_INITD" ]; then
    if [ "$HAS_INITD" -eq 1 ]; then
        ask "Install OpenRC service file (${PREFIX}/etc/init.d/jcarp)?"
        WITH_INITD="$ANSWER"
    else
        WITH_INITD=0
    fi
fi

if [ -z "$WITH_CONFIG" ]; then
    if [ "$HAS_CONFIG" -eq 1 ]; then
        ask "Install default config (${PREFIX}/etc/jcarp/jcarp.conf.default)?"
        WITH_CONFIG="$ANSWER"
    else
        WITH_CONFIG=0
    fi
fi

if [ -z "$DO_SETCAP" ]; then
    ask "Set CAP_NET_ADMIN+CAP_NET_RAW on ${PREFIX}/bin/jcarp via setcap? Requires root."
    DO_SETCAP="$ANSWER"
fi

# ── install ─────────────────────────────────────────────────────────────────
log "installing into ${PREFIX}"

install -d "$PREFIX/bin" "$PREFIX/share/licenses/jcarp"
install -m 755 "$PAYLOAD_DIR/bin/jcarp" "$PREFIX/bin/jcarp"
INSTALLED_BIN="$PREFIX/bin/jcarp"

INSTALLED_LICENSE=""
if [ -f "$PAYLOAD_DIR/share/licenses/jcarp/LICENSE" ]; then
    install -m 644 "$PAYLOAD_DIR/share/licenses/jcarp/LICENSE" \
        "$PREFIX/share/licenses/jcarp/LICENSE"
    INSTALLED_LICENSE="$PREFIX/share/licenses/jcarp/LICENSE"
fi

# Man pages: always install whatever is shipped under share/man/.
INSTALLED_MAN=""
if [ -d "$PAYLOAD_DIR/share/man" ]; then
    # Preserve manN/ section directories; install files individually so the
    # destination owner/mode is normalized.
    find "$PAYLOAD_DIR/share/man" -type f 2>/dev/null \
    | while IFS= read -r src; do
        rel=${src#"$PAYLOAD_DIR"/}
        dst="$PREFIX/$rel"
        dst_dir=$(dirname "$dst")
        install -d "$dst_dir"
        install -m 644 "$src" "$dst"
    done
    INSTALLED_MAN="$PREFIX/share/man"
fi

INSTALLED_INITD=""
if [ "$WITH_INITD" -eq 1 ]; then
    if [ "$HAS_INITD" -eq 1 ]; then
        install -d "$PREFIX/etc/init.d"
        install -m 755 "$PAYLOAD_DIR/etc/init.d/jcarp" "$PREFIX/etc/init.d/jcarp"
        INSTALLED_INITD="$PREFIX/etc/init.d/jcarp"
    else
        warn "init script requested but not present in payload"
    fi
fi

INSTALLED_CONFIG=""
if [ "$WITH_CONFIG" -eq 1 ]; then
    if [ "$HAS_CONFIG" -eq 1 ]; then
        install -d "$PREFIX/etc/jcarp"
        if [ -f "$PAYLOAD_DIR/etc/jcarp/jcarp.conf.default" ]; then
            install -m 644 "$PAYLOAD_DIR/etc/jcarp/jcarp.conf.default" \
                "$PREFIX/etc/jcarp/jcarp.conf.default"
        else
            install -m 644 "$PAYLOAD_DIR/etc/jcarp/jcarp.conf" \
                "$PREFIX/etc/jcarp/jcarp.conf.default"
        fi
        INSTALLED_CONFIG="$PREFIX/etc/jcarp/jcarp.conf.default"
    else
        warn "config requested but not present in payload"
    fi
fi

SETCAP_DONE=0
SETCAP_FAILED=0
if [ "$DO_SETCAP" -eq 1 ]; then
    if have setcap; then
        if setcap 'cap_net_admin,cap_net_raw=ep' "$INSTALLED_BIN" 2>/dev/null; then
            SETCAP_DONE=1
        else
            SETCAP_FAILED=1
            warn "setcap failed (need root?)"
        fi
    else
        SETCAP_FAILED=1
        warn "setcap not found; install libcap (or equivalent) and re-run with --setcap"
    fi
fi

# ── sanity check ────────────────────────────────────────────────────────────
log "running sanity check: $INSTALLED_BIN --version"
if "$INSTALLED_BIN" --version >/dev/null 2>&1; then
    "$INSTALLED_BIN" --version || true
elif "$INSTALLED_BIN" --help >/dev/null 2>&1; then
    log "--version unsupported, --help OK"
else
    warn "sanity check failed: cannot invoke $INSTALLED_BIN"
    warn "(this may be normal if the host arch differs from the binary)"
fi

# ── post-install trailer ────────────────────────────────────────────────────
printf '\n'
printf 'jcarp %s (%s) installed under %s\n\n' "$VERSION" "$ARCH" "$PREFIX"

printf 'Installed:\n'
printf '  %s\n' "$INSTALLED_BIN"
[ -n "$INSTALLED_LICENSE" ] && printf '  %s\n' "$INSTALLED_LICENSE"
[ -n "$INSTALLED_MAN" ]     && printf '  %s/...\n' "$INSTALLED_MAN"
[ -n "$INSTALLED_INITD" ]   && printf '  %s\n' "$INSTALLED_INITD"
[ -n "$INSTALLED_CONFIG" ]  && printf '  %s\n' "$INSTALLED_CONFIG"
printf '\n'

# PATH check.
# shellcheck disable=SC2016
case ":${PATH-}:" in
    *":$PREFIX/bin:"*) ;;
    *) printf 'NOTE: %s/bin is not on $PATH. Add it, or invoke jcarp by full path.\n\n' "$PREFIX" ;;
esac

# Recommended next steps for declined opt-ins.
need_next=0
if [ "$SETCAP_DONE" -ne 1 ]; then need_next=1; fi
if [ -z "$INSTALLED_INITD" ];   then need_next=1; fi
if [ -z "$INSTALLED_CONFIG" ];  then need_next=1; fi

if [ "$need_next" -eq 1 ]; then
    printf 'Recommended next steps:\n'

    if [ "$SETCAP_DONE" -ne 1 ]; then
        if [ "$SETCAP_FAILED" -eq 1 ]; then
            printf '  - Re-run setcap as root:\n'
        else
            printf '  - Grant raw-socket capabilities (run as root):\n'
        fi
        printf '      sudo setcap cap_net_admin,cap_net_raw=ep %s\n' "$INSTALLED_BIN"
        printf '    Or run jcarp as root via your service manager.\n'
    fi

    if [ -z "$INSTALLED_CONFIG" ]; then
        printf '  - Provide a config (re-run with --with-config to drop the default):\n'
        printf '      sh install.sh --prefix %s --with-config --no-prompt\n' "$PREFIX"
        printf '    Then copy %s/etc/jcarp/jcarp.conf.default to\n' "$PREFIX"
        printf '    %s/etc/jcarp/jcarp.conf and edit it.\n' "$PREFIX"
    else
        printf '  - Copy %s to\n' "$INSTALLED_CONFIG"
        printf '    %s/etc/jcarp/jcarp.conf and edit it.\n' "$PREFIX"
    fi

    if [ -z "$INSTALLED_INITD" ]; then
        printf '  - Install OpenRC service (re-run with --with-init-script):\n'
        printf '      sh install.sh --prefix %s --with-init-script --no-prompt\n' "$PREFIX"
        printf '    Then on OpenRC:\n'
        printf '      rc-update add jcarp default && rc-service jcarp start\n'
    else
        printf '  - On OpenRC, enable and start the service:\n'
        printf '      rc-update add jcarp default && rc-service jcarp start\n'
    fi

    printf '\n'
fi

printf 'jcarp installed. jcarp is a privileged daemon — it will fail without\n'
printf 'CAP_NET_ADMIN+CAP_NET_RAW or running as root.\n'
