#!/bin/sh
# install.sh -- POSIX-shell installer for exproxide on non-jonerix systems.
#
# Fetches the published .jpkg from the jonerix GitHub release, verifies the
# JPKG magic, decodes the framed metadata + zstd-compressed tar payload, and
# stages files under $PREFIX with conservative defaults:
#
#   - bin/exproxide                 -> $PREFIX/bin/exproxide                (always)
#   - LICENSE                       -> $PREFIX/share/licenses/exproxide/LICENSE
#   - share/man/...                 -> $PREFIX/share/man/...                (if shipped)
#   - bin/expr -> exproxide symlink -> opt-in via --make-default-expr
#   - etc/                          -> staged to etc-default; opt-in --install-etc
#
# The bin/expr symlink is INTENTIONALLY off by default: $PREFIX/bin/expr would
# shadow /usr/bin/expr if $PREFIX/bin precedes /usr/bin on $PATH, and that's a
# foot-gun on a host that already has GNU or BSD expr.
#
#   curl -fsSL https://castle.great-morpho.ts.net:3000/jonerik/exproxide/raw/branch/main/install.sh | sh
#
# Or with options:
#
#   sh install.sh --prefix /opt/exproxide --version 0.1.1-r0 --arch x86_64 --no-prompt
#
# Strictly POSIX. Validated with: dash -n, mksh -n, shellcheck -s sh.

set -eu

DEFAULT_VERSION='0.1.1-r0'
DEFAULT_PREFIX='/usr/local'
RELEASE_BASE='https://github.com/stormj-UH/jonerix/releases/download/packages'

VERSION="$DEFAULT_VERSION"
PREFIX="$DEFAULT_PREFIX"
ARCH=''

# Tri-state opt-ins: '' = ask interactively, 1 = yes, 0 = no.
MAKE_DEFAULT_EXPR=''
INSTALL_ETC=''
NO_PROMPT=0

PRIMARY_BIN='exproxide'

usage() {
    cat <<EOF
Usage: install.sh [options]

  --version <VER>          exproxide version to fetch        (default: $DEFAULT_VERSION)
  --prefix  <DIR>          install root                      (default: $DEFAULT_PREFIX)
  --arch    <ARCH>         override uname -m detection (e.g. x86_64, aarch64)

  --make-default-expr      also install \$PREFIX/bin/expr -> exproxide
  --no-make-default-expr   do not install the bin/expr symlink (default)

  --install-etc            copy any etc/ payload onto \$PREFIX/etc
  --no-install-etc         stage etc/ under \$PREFIX/etc-default (default)

  --no-prompt | --yes      never prompt; use defaults / explicit flags only
  --help                   show this message

Long options also accept --key=value.

Notes:
  * The bin/expr symlink is OFF by default. Turning it on with
    --make-default-expr will shadow /usr/bin/expr if \$PREFIX/bin precedes
    /usr/bin on \$PATH.
  * This installer never modifies /usr/bin/expr or any other system binary.
EOF
}

# err -- print to stderr and abort.
err() {
    printf '%s: %s\n' "$0" "$*" >&2
    exit 1
}

info() {
    printf '[install.sh] %s\n' "$*"
}

warn() {
    printf '[install.sh] WARN: %s\n' "$*" >&2
}

# need_arg -- option NAME requires a value; complain if it is missing.
need_arg() {
    [ $# -ge 2 ] || err "option $1 requires an argument"
}

# parse_bool -- normalise --foo=value into 0/1 or abort.
parse_bool() {
    case "$2" in
        1|true|TRUE|yes|YES|on|ON)   eval "$1=1" ;;
        0|false|FALSE|no|NO|off|OFF) eval "$1=0" ;;
        *) err "$3=$2: expected 1/0/true/false/yes/no/on/off" ;;
    esac
}

# Argument parser. Accepts both `--key value` and `--key=value`.
while [ $# -gt 0 ]; do
    case "$1" in
        --version)        need_arg "$@"; VERSION="$2"; shift 2 ;;
        --version=*)      VERSION="${1#--version=}"; shift ;;
        --prefix)         need_arg "$@"; PREFIX="$2"; shift 2 ;;
        --prefix=*)       PREFIX="${1#--prefix=}"; shift ;;
        --arch)           need_arg "$@"; ARCH="$2"; shift 2 ;;
        --arch=*)         ARCH="${1#--arch=}"; shift ;;

        --make-default-expr)        MAKE_DEFAULT_EXPR=1; shift ;;
        --no-make-default-expr)     MAKE_DEFAULT_EXPR=0; shift ;;
        --make-default-expr=*)
            parse_bool MAKE_DEFAULT_EXPR "${1#--make-default-expr=}" --make-default-expr
            shift
            ;;

        --install-etc)              INSTALL_ETC=1; shift ;;
        --no-install-etc)           INSTALL_ETC=0; shift ;;
        --install-etc=*)
            parse_bool INSTALL_ETC "${1#--install-etc=}" --install-etc
            shift
            ;;

        --no-prompt|--yes)          NO_PROMPT=1; shift ;;

        -h|--help)        usage; exit 0 ;;
        --)               shift; break ;;
        -*)               err "unknown option: $1 (try --help)" ;;
        *)                err "unexpected argument: $1 (try --help)" ;;
    esac
done

# Architecture detection. Map the common Linux uname -m strings onto the
# names the jonerix release pipeline publishes; pass everything else through
# verbatim so an unrecognised host can still target a published asset by
# spelling out --arch.
if [ -z "$ARCH" ]; then
    raw_arch="$(uname -m)"
    case "$raw_arch" in
        x86_64|amd64)            ARCH='x86_64' ;;
        aarch64|arm64)           ARCH='aarch64' ;;
        armv7l|armv7|armhf)      ARCH='armv7' ;;
        *)                       ARCH="$raw_arch" ;;
    esac
fi

# Tooling check. We need a downloader (curl or wget), zstd to decompress
# the payload, tar to extract it, and od + dd to slice the framed header.
have() { command -v "$1" >/dev/null 2>&1; }

if have curl; then
    DOWNLOADER='curl'
elif have wget; then
    DOWNLOADER='wget'
else
    err 'need curl or wget on PATH'
fi

missing=''
for t in zstd tar od dd; do
    have "$t" || missing="$missing $t"
done
[ -z "$missing" ] || err "missing required tools:$missing"

# Resolve the target asset URL.
JPKG_NAME="exproxide-${VERSION}-${ARCH}.jpkg"
JPKG_URL="${RELEASE_BASE}/${JPKG_NAME}"

# Working directory + cleanup trap. POSIX trap for HUP/INT/TERM/EXIT.
TMPDIR_INSTALL="$(mktemp -d 2>/dev/null || mktemp -d -t exproxide)"
[ -n "$TMPDIR_INSTALL" ] && [ -d "$TMPDIR_INSTALL" ] || err "mktemp -d failed"

cleanup() {
    rm -rf "$TMPDIR_INSTALL" 2>/dev/null || true
}
trap cleanup EXIT
trap 'cleanup; exit 130' HUP INT TERM

JPKG_FILE="$TMPDIR_INSTALL/$JPKG_NAME"
PAYLOAD_DIR="$TMPDIR_INSTALL/payload"
mkdir -p "$PAYLOAD_DIR"

info "exproxide version : $VERSION"
info "host arch detected: $ARCH"
info "install prefix    : $PREFIX"
info "downloading       : $JPKG_URL"

# Download with whichever fetcher we found. Both follow redirects (the
# GitHub release URL 302s into a Blob storage URL).
case "$DOWNLOADER" in
    curl) curl -fSL --proto '=https' --tlsv1.2 -o "$JPKG_FILE" "$JPKG_URL" ;;
    wget) wget --https-only -O "$JPKG_FILE" "$JPKG_URL" ;;
esac

[ -s "$JPKG_FILE" ] || err "downloaded file is empty: $JPKG_FILE"

# Verify "JPKG" magic at offset 0. Reading 4 bytes from a binary file in a
# POSIX-portable way means going through od; `head -c` is not in POSIX, and
# command substitution would strip trailing whitespace anyway.
magic="$(dd if="$JPKG_FILE" bs=1 count=4 status=none | od -An -c | tr -d ' \t\n')"
[ "$magic" = 'JPKG' ] || err "bad magic: expected JPKG, got '$magic' (truncated download?)"

# Read the metadata length (le u32 at offset 8). od -tu4 reads a 4-byte
# unsigned integer in host byte order; jonerix runs on little-endian arches
# only, so this matches the on-disk encoding without byte-swapping.
md_len="$(od -An -tu4 -N4 -j8 "$JPKG_FILE" | tr -d ' \t\n')"
case "$md_len" in
    ''|*[!0-9]*) err "metadata length parse failed (got '$md_len')" ;;
esac
[ "$md_len" -gt 0 ] || err "metadata length is zero"
[ "$md_len" -lt 1048576 ] || err "metadata length absurd: $md_len bytes"

payload_offset=$((12 + md_len))
info "JPKG magic ok; metadata length = $md_len; payload offset = $payload_offset"

# Decode + extract the payload. dd skips the 12-byte header and md_len-byte
# metadata; zstd decompresses; tar unpacks into PAYLOAD_DIR with -p so the
# 0755 mode on the exproxide binary survives.
dd if="$JPKG_FILE" bs=1 skip="$payload_offset" status=none \
    | zstd -d -q \
    | tar -xpf - -C "$PAYLOAD_DIR" \
    || err "failed to decode/extract jpkg payload"

# Validate the payload shape we expect. We require bin/exproxide; everything
# else (LICENSE, share/man, etc/, bin/expr) is optional.
[ -f "$PAYLOAD_DIR/bin/exproxide" ] || \
    err "payload missing bin/exproxide -- not an exproxide jpkg?"

# ---- prompts ----------------------------------------------------------------
#
# ask YES_NO_VAR PROMPT DEFAULT
#   Reads a y/N answer from /dev/tty (so the prompt still works when the
#   script is piped from curl). Sets the named variable to 0 or 1. If
#   --no-prompt was passed, or there is no tty, falls back to DEFAULT.
ask() {
    var="$1"
    prompt_text="$2"
    default_val="$3"

    if [ "$NO_PROMPT" -eq 1 ]; then
        eval "$var=$default_val"
        return 0
    fi
    if [ ! -r /dev/tty ] || [ ! -w /dev/tty ]; then
        eval "$var=$default_val"
        return 0
    fi

    suffix='[y/N]'
    [ "$default_val" -eq 1 ] && suffix='[Y/n]'
    printf '%s %s ' "$prompt_text" "$suffix" > /dev/tty
    if ! IFS= read -r reply < /dev/tty; then
        # EOF on the tty -- behave as --no-prompt.
        eval "$var=$default_val"
        return 0
    fi
    case "$reply" in
        y|Y|yes|YES|Yes) eval "$var=1" ;;
        n|N|no|NO|No)    eval "$var=0" ;;
        '')              eval "$var=$default_val" ;;
        *)               eval "$var=$default_val" ;;
    esac
}

if [ -z "$MAKE_DEFAULT_EXPR" ]; then
    if [ -L "$PAYLOAD_DIR/bin/expr" ] || [ -e "$PAYLOAD_DIR/bin/expr" ]; then
        ask MAKE_DEFAULT_EXPR \
            "Install $PREFIX/bin/expr -> exproxide symlink? Will shadow /usr/bin/expr if $PREFIX/bin is ahead on PATH." \
            0
    else
        MAKE_DEFAULT_EXPR=0
    fi
fi

if [ -z "$INSTALL_ETC" ]; then
    if [ -d "$PAYLOAD_DIR/etc" ]; then
        ask INSTALL_ETC \
            "Install /etc-default templates from the package?" \
            0
    else
        INSTALL_ETC=0
    fi
fi

# ---- staging ----------------------------------------------------------------
mkdir -p "$PREFIX" || err "cannot create $PREFIX (try sudo, or pick a writable --prefix)"

# install_file SRC DST -- atomic-ish copy preserving mode.
install_file() {
    src="$1"
    dst="$2"
    dst_dir="$(dirname "$dst")"
    mkdir -p "$dst_dir"
    cp -f "$src" "$dst"
    # Preserve executable bit; cp -f does not always do so on every platform.
    if [ -x "$src" ]; then
        chmod 0755 "$dst"
    else
        chmod 0644 "$dst"
    fi
}

# copy_subtree SRC DST -- mirror SRC's contents into DST, preserving symlinks.
copy_subtree() {
    src="$1"
    dst="$2"
    [ -d "$src" ] || return 0
    mkdir -p "$dst"
    # cp -a preserves symlinks, mode, and timestamps; the trailing /. copies
    # contents (not the directory itself) into the destination.
    cp -a "$src/." "$dst/"
}

INSTALLED_LICENSE=0
INSTALLED_MAN=0
INSTALLED_EXPR_SYMLINK=0
INSTALLED_ETC=0
ETC_STAGED_DEFAULT=0

# 1. Primary binary -- always.
info "installing bin/exproxide -> $PREFIX/bin/exproxide"
install_file "$PAYLOAD_DIR/bin/exproxide" "$PREFIX/bin/exproxide"

# 2. LICENSE -- always, if present.
if [ -f "$PAYLOAD_DIR/LICENSE" ]; then
    info "installing LICENSE -> $PREFIX/share/licenses/exproxide/LICENSE"
    install_file "$PAYLOAD_DIR/LICENSE" "$PREFIX/share/licenses/exproxide/LICENSE"
    INSTALLED_LICENSE=1
fi

# 3. Man pages -- always, if present.
if [ -d "$PAYLOAD_DIR/share/man" ]; then
    info "installing share/man -> $PREFIX/share/man"
    copy_subtree "$PAYLOAD_DIR/share/man" "$PREFIX/share/man"
    INSTALLED_MAN=1
fi

# 4. bin/expr symlink -- opt-in.
if [ "$MAKE_DEFAULT_EXPR" -eq 1 ]; then
    info "installing bin/expr -> exproxide symlink (--make-default-expr)"
    mkdir -p "$PREFIX/bin"
    # Use a relative symlink target so the link survives a $PREFIX move.
    ln -sf exproxide "$PREFIX/bin/expr"
    INSTALLED_EXPR_SYMLINK=1
fi

# 5. etc/ -- opt-in. Otherwise stage under $PREFIX/etc-default if shipped.
if [ -d "$PAYLOAD_DIR/etc" ]; then
    if [ "$INSTALL_ETC" -eq 1 ]; then
        info "installing etc/ -> $PREFIX/etc (--install-etc)"
        copy_subtree "$PAYLOAD_DIR/etc" "$PREFIX/etc"
        INSTALLED_ETC=1
    else
        info "staging etc/ -> $PREFIX/etc-default (use --install-etc to opt in)"
        copy_subtree "$PAYLOAD_DIR/etc" "$PREFIX/etc-default"
        ETC_STAGED_DEFAULT=1
    fi
fi

# ---- sanity check -----------------------------------------------------------
# Probe the installed exproxide binary so we catch a wrong-arch download or a
# corrupt binary now rather than the next time the user runs it.
if [ -x "$PREFIX/bin/exproxide" ]; then
    if "$PREFIX/bin/exproxide" --version >/dev/null 2>&1; then
        info "ok: exproxide --version"
    elif "$PREFIX/bin/exproxide" --help >/dev/null 2>&1; then
        info "ok: exproxide --help"
    else
        warn "exproxide did not respond to --version or --help"
    fi
fi

# ---- post-install trailer ---------------------------------------------------
printf '\n'
printf '[install.sh] Summary\n'
printf '  exproxide version : %s\n' "$VERSION"
printf '  arch              : %s\n' "$ARCH"
printf '  prefix            : %s\n' "$PREFIX"
printf '  installed         : %s/bin/exproxide\n' "$PREFIX"
if [ "$INSTALLED_LICENSE" -eq 1 ]; then
    printf '                      %s/share/licenses/exproxide/LICENSE\n' "$PREFIX"
fi
if [ "$INSTALLED_MAN" -eq 1 ]; then
    printf '                      %s/share/man/...\n' "$PREFIX"
fi
if [ "$INSTALLED_EXPR_SYMLINK" -eq 1 ]; then
    printf '                      %s/bin/expr -> exproxide\n' "$PREFIX"
fi
if [ "$INSTALLED_ETC" -eq 1 ]; then
    printf '                      %s/etc/...\n' "$PREFIX"
fi
if [ "$ETC_STAGED_DEFAULT" -eq 1 ]; then
    printf '                      %s/etc-default/...  (staged, not active)\n' "$PREFIX"
fi
printf '\n'

# PATH advisory.
case ":${PATH:-}:" in
    *":$PREFIX/bin:"*) : ;;
    *)
        # shellcheck disable=SC2016  # literal $PATH in user-facing message
        printf '[install.sh] NOTE: %s/bin is not on $PATH.\n' "$PREFIX"
        printf '            Add it to your shell rc, e.g.:\n'
        # shellcheck disable=SC2016  # literal $PATH in suggested rc line
        printf '              export PATH="%s/bin:$PATH"\n\n' "$PREFIX"
        ;;
esac

# Recommended next steps for declined opt-ins.
if [ "$INSTALLED_EXPR_SYMLINK" -eq 0 ]; then
    printf '[install.sh] NOTE: bin/expr symlink not installed.\n'
    # shellcheck disable=SC2016  # literal expr in user-facing message
    printf '            To use exproxide whenever a script runs `expr`, either:\n'
    printf '              ln -sf exproxide %s/bin/expr\n' "$PREFIX"
    printf '            or rerun this installer with --make-default-expr.\n'
    printf '            This will shadow /usr/bin/expr only if %s/bin is\n' "$PREFIX"
    # shellcheck disable=SC2016  # literal $PATH in user-facing message
    printf '            ahead of /usr/bin on $PATH; system binaries are untouched.\n\n'
fi

if [ "$ETC_STAGED_DEFAULT" -eq 1 ] && [ "$INSTALLED_ETC" -eq 0 ]; then
    printf '[install.sh] NOTE: etc/ payload staged at %s/etc-default.\n' "$PREFIX"
    printf '            Review the templates and either copy what you want into\n'
    printf '              %s/etc/  (or /etc/),\n' "$PREFIX"
    printf '            or rerun the installer with --install-etc.\n\n'
fi

# Never-touch reassurance: this installer does not modify /usr/bin/expr or
# any other system binary. A pre-existing /usr/bin/expr is left alone.

info "$PRIMARY_BIN installed."
