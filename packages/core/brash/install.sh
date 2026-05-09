#!/bin/sh
# brash installer (POSIX sh)
#
# Default behaviour: install ONLY the binary, the license, and any man pages
# under $PREFIX (default /usr/local).  No system-global state is changed.
#
# Optional, opt-in side effects (off by default):
#   * append $PREFIX/bin/brash to /etc/shells   (--register-shell)
#   * symlink $PREFIX/bin/bash -> brash         (--make-default-bash)
#
# When stdin is interactive AND --no-prompt/--yes is not given, the installer
# will offer those opt-ins as [y/N] prompts after the binary is in place.
#
# Usage:
#     sh install.sh [--version VER] [--prefix DIR] [--arch ARCH]
#                   [--register-shell | --no-register-shell]
#                   [--make-default-bash | --no-make-default-bash]
#                   [--no-prompt | --yes] [--help]
#
# Defaults: --version 1.0.16  --prefix /usr/local  --arch <autodetected>

set -eu

DEFAULT_VERSION=1.0.16
DEFAULT_PREFIX=/usr/local
RELEASE_BASE=https://github.com/stormj-UH/jonerix/releases/download/packages

VERSION=$DEFAULT_VERSION
PREFIX=$DEFAULT_PREFIX
ARCH=

# Tri-state: 0 = unset (ask if interactive), 1 = yes, 2 = no.
REGISTER_SHELL=0
MAKE_DEFAULT_BASH=0
NO_PROMPT=0

usage() {
    cat <<'EOF'
brash installer

Usage:
  sh install.sh [options]

Default behaviour installs ONLY the brash binary, license, and man pages
under --prefix.  No system-global files are touched unless you opt in.

Options:
  --version VER             brash version to install (default 1.0.16)
  --prefix  DIR             install prefix (default /usr/local)
  --arch    ARCH            override autodetect (x86_64 | aarch64)

  --register-shell          yes: append $PREFIX/bin/brash to /etc/shells
  --no-register-shell       no:  do not touch /etc/shells (default)
  --make-default-bash       yes: symlink $PREFIX/bin/bash -> brash
  --no-make-default-bash    no:  do not create the bash symlink (default)

  --no-prompt, --yes        non-interactive mode: never prompt; use the
                            explicit flags above as the answers, with "no"
                            as the default for unspecified opt-ins
  -h, --help                show this message and exit

Each long option also accepts the --opt=value form.

The installer downloads:
  https://github.com/stormj-UH/jonerix/releases/download/packages/brash-<VER>-<ARCH>.jpkg
EOF
}

die() {
    printf 'brash-install: %s\n' "$*" >&2
    exit 1
}

warn() {
    printf 'brash-install: %s\n' "$*" >&2
}

note() {
    printf 'brash-install: %s\n' "$*"
}

# ----- argument parsing ------------------------------------------------------
while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || die "--version requires an argument"
            VERSION=$2
            shift 2
            ;;
        --version=*)
            VERSION=${1#--version=}
            shift
            ;;
        --prefix)
            [ $# -ge 2 ] || die "--prefix requires an argument"
            PREFIX=$2
            shift 2
            ;;
        --prefix=*)
            PREFIX=${1#--prefix=}
            shift
            ;;
        --arch)
            [ $# -ge 2 ] || die "--arch requires an argument"
            ARCH=$2
            shift 2
            ;;
        --arch=*)
            ARCH=${1#--arch=}
            shift
            ;;
        --register-shell)
            REGISTER_SHELL=1
            shift
            ;;
        --no-register-shell)
            REGISTER_SHELL=2
            shift
            ;;
        --make-default-bash)
            MAKE_DEFAULT_BASH=1
            shift
            ;;
        --no-make-default-bash)
            MAKE_DEFAULT_BASH=2
            shift
            ;;
        --no-prompt|--yes)
            NO_PROMPT=1
            shift
            ;;
        --no-prompt=*|--yes=*)
            # Tolerate `--no-prompt=` / `--yes=` for the long-form rule, but
            # they take no value.
            NO_PROMPT=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        -*)
            die "unknown option: $1 (try --help)"
            ;;
        *)
            die "unexpected argument: $1 (try --help)"
            ;;
    esac
done

[ -n "$VERSION" ] || die "--version cannot be empty"
[ -n "$PREFIX" ]  || die "--prefix cannot be empty"

# ----- arch autodetect -------------------------------------------------------
if [ -z "$ARCH" ]; then
    machine=$(uname -m 2>/dev/null || echo unknown)
    case "$machine" in
        x86_64|amd64)        ARCH=x86_64 ;;
        aarch64|arm64)       ARCH=aarch64 ;;
        *)                   die "unsupported architecture: $machine (override with --arch)" ;;
    esac
else
    case "$ARCH" in
        x86_64|aarch64) : ;;
        amd64)          ARCH=x86_64 ;;
        arm64)          ARCH=aarch64 ;;
        *)              die "unsupported --arch value: $ARCH (use x86_64 or aarch64)" ;;
    esac
fi

# ----- tool checks -----------------------------------------------------------
have() { command -v "$1" >/dev/null 2>&1; }

DOWNLOAD=
if have curl; then
    DOWNLOAD=curl
elif have wget; then
    DOWNLOAD=wget
fi

missing=
[ -n "$DOWNLOAD" ] || missing="${missing} curl-or-wget"
have zstd || missing="${missing} zstd"
have tar  || missing="${missing} tar"
have od   || missing="${missing} od"
have dd   || missing="${missing} dd"

if [ -n "$missing" ]; then
    warn "missing required tools:${missing}"
    warn ""
    warn "Suggested install commands:"
    warn "  Debian/Ubuntu : sudo apt-get install -y curl zstd tar coreutils"
    warn "  Fedora/RHEL   : sudo dnf install -y curl zstd tar coreutils"
    warn "  Arch          : sudo pacman -S --needed curl zstd tar coreutils"
    warn "  Alpine        : sudo apk add curl zstd tar coreutils"
    warn "  openSUSE      : sudo zypper install curl zstd tar coreutils"
    exit 1
fi

# ----- workspace -------------------------------------------------------------
TMP=$(mktemp -d 2>/dev/null || mktemp -d -t brash-install)
[ -n "$TMP" ] && [ -d "$TMP" ] || die "could not create temp directory"

# shellcheck disable=SC2329  # invoked indirectly via trap
cleanup() {
    rm -rf "$TMP"
}
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM

JPKG_NAME="brash-${VERSION}-${ARCH}.jpkg"
JPKG_URL="${RELEASE_BASE}/${JPKG_NAME}"
JPKG_FILE="${TMP}/${JPKG_NAME}"

note "version : ${VERSION}"
note "arch    : ${ARCH}"
note "prefix  : ${PREFIX}"
note "url     : ${JPKG_URL}"

# ----- download --------------------------------------------------------------
note "downloading ${JPKG_NAME}"
if [ "$DOWNLOAD" = curl ]; then
    if ! curl -fsSL --retry 3 --retry-delay 2 "$JPKG_URL" -o "$JPKG_FILE"; then
        die "download failed: $JPKG_URL"
    fi
else
    if ! wget -q -O "$JPKG_FILE" "$JPKG_URL"; then
        die "download failed: $JPKG_URL"
    fi
fi

[ -s "$JPKG_FILE" ] || die "downloaded file is empty: $JPKG_FILE"

# ----- magic check -----------------------------------------------------------
# The JPKG container starts with the ASCII bytes "JPKG".
magic=$(dd if="$JPKG_FILE" bs=1 count=4 status=none 2>/dev/null | tr -d '\000')
if [ "$magic" != "JPKG" ]; then
    die "bad magic in $JPKG_NAME (got '$magic', expected 'JPKG'); aborted"
fi

# ----- metadata length (little-endian u32 at offset 8) -----------------------
md_len=$(od -An -tu4 -N4 -j8 "$JPKG_FILE" | tr -d ' \t\n')
case "$md_len" in
    ''|*[!0-9]*) die "could not read metadata length from $JPKG_NAME" ;;
esac

payload_offset=$(( 12 + md_len ))
note "metadata length: ${md_len} bytes; payload starts at offset ${payload_offset}"

# ----- extract payload -------------------------------------------------------
EXTRACT_DIR="${TMP}/extract"
mkdir -p "$EXTRACT_DIR"

note "decompressing payload"
if ! dd if="$JPKG_FILE" bs=1 skip="$payload_offset" status=none 2>/dev/null \
        | zstd -d -q \
        | tar -x -C "$EXTRACT_DIR"; then
    die "failed to extract payload from $JPKG_NAME"
fi

[ -f "$EXTRACT_DIR/bin/brash" ] || die "payload is missing bin/brash"

# ----- install ---------------------------------------------------------------
BIN_DIR="${PREFIX}/bin"
LICENSE_DIR="${PREFIX}/share/licenses/brash"
MAN_DIR="${PREFIX}/share/man"

note "installing into ${PREFIX}"

mkdir -p "$BIN_DIR" || die "cannot create $BIN_DIR (need sudo?)"
cp "$EXTRACT_DIR/bin/brash" "${BIN_DIR}/brash.new"
chmod 0755 "${BIN_DIR}/brash.new"
mv "${BIN_DIR}/brash.new" "${BIN_DIR}/brash"

INSTALLED_LICENSE=
if [ -f "$EXTRACT_DIR/share/licenses/brash/LICENSE" ]; then
    mkdir -p "$LICENSE_DIR"
    cp "$EXTRACT_DIR/share/licenses/brash/LICENSE" "${LICENSE_DIR}/LICENSE"
    chmod 0644 "${LICENSE_DIR}/LICENSE"
    INSTALLED_LICENSE="${LICENSE_DIR}/LICENSE"
fi

# ----- install man pages (if present in payload) -----------------------------
INSTALLED_MAN_COUNT=0
if [ -d "$EXTRACT_DIR/share/man" ]; then
    # Walk each manN/ directory we find and copy *.N pages individually.
    # Done with `find` + a while-read loop to stay POSIX (no arrays, no globs
    # that might be empty).
    find "$EXTRACT_DIR/share/man" -type f \
        \( -name '*.1' -o -name '*.2' -o -name '*.3' -o -name '*.4' \
           -o -name '*.5' -o -name '*.6' -o -name '*.7' -o -name '*.8' \
           -o -name '*.1.gz' -o -name '*.5.gz' -o -name '*.8.gz' \) \
        > "${TMP}/manpages.list" 2>/dev/null || :
    if [ -s "${TMP}/manpages.list" ]; then
        while IFS= read -r src; do
            [ -n "$src" ] || continue
            base=${src##*/}
            # last component before the basename gives "man1", "man5", etc.
            section_dir=${src%/*}
            section=${section_dir##*/}
            case "$section" in
                man[0-9])
                    mkdir -p "${MAN_DIR}/${section}"
                    cp "$src" "${MAN_DIR}/${section}/${base}"
                    chmod 0644 "${MAN_DIR}/${section}/${base}"
                    INSTALLED_MAN_COUNT=$(( INSTALLED_MAN_COUNT + 1 ))
                    ;;
                *) : ;;
            esac
        done < "${TMP}/manpages.list"
    fi
fi

# ----- verify (do this BEFORE prompting so we know whether to surface it) ---
# Best-effort: a non-zero exit (e.g. cross-OS install on a host that can't
# execute the ELF) is informational, not fatal -- the install on disk is
# already correct.
reported=
if "${BIN_DIR}/brash" --version >/dev/null 2>&1; then
    reported=$("${BIN_DIR}/brash" --version 2>/dev/null | head -n1 || true)
else
    warn "post-install check: ${BIN_DIR}/brash --version did not exit cleanly"
    warn "  (this is expected when installing for a different OS/arch)"
fi

# ----- decide whether to prompt ---------------------------------------------
INTERACTIVE=0
if [ "$NO_PROMPT" -eq 0 ] && [ -t 1 ] && [ -r /dev/tty ]; then
    INTERACTIVE=1
fi

# Helper: ask a yes/no question with default = no.  Reads from /dev/tty so
# `curl ... | sh` still works (stdin is the script).  Returns 0 for yes, 1
# for no.
ask_yn() {
    # $1 = prompt text
    printf '%s' "$1" >&2
    ans=
    if ! IFS= read -r ans < /dev/tty; then
        printf '\n' >&2
        return 1
    fi
    case "$ans" in
        y|Y|yes|YES|Yes) return 0 ;;
        *)               return 1 ;;
    esac
}

# Resolve the two opt-ins to a final 1=do / 2=skip decision.
if [ "$REGISTER_SHELL" -eq 0 ]; then
    if [ "$INTERACTIVE" -eq 1 ]; then
        if ask_yn "brash-install: Add ${BIN_DIR}/brash to /etc/shells (so chsh -s works)? [y/N] "; then
            REGISTER_SHELL=1
        else
            REGISTER_SHELL=2
        fi
    else
        REGISTER_SHELL=2
    fi
fi

if [ "$MAKE_DEFAULT_BASH" -eq 0 ]; then
    if [ "$INTERACTIVE" -eq 1 ]; then
        if ask_yn "brash-install: Symlink ${BIN_DIR}/bash -> brash? Note: this is dangerous if /bin/bash is ahead on \$PATH. [y/N] "; then
            MAKE_DEFAULT_BASH=1
        else
            MAKE_DEFAULT_BASH=2
        fi
    else
        MAKE_DEFAULT_BASH=2
    fi
fi

# ----- shell registration (optional) -----------------------------------------
DID_REGISTER_SHELL=0
SKIPPED_REGISTER_REASON=
if [ "$REGISTER_SHELL" -eq 1 ]; then
    SHELLS_FILE=/etc/shells
    uid_now=$(id -u 2>/dev/null || echo 1)
    if [ -w "$SHELLS_FILE" ] || [ "$uid_now" = "0" ]; then
        if [ -f "$SHELLS_FILE" ] && grep -Fxq "${BIN_DIR}/brash" "$SHELLS_FILE"; then
            note "${BIN_DIR}/brash already in $SHELLS_FILE"
            DID_REGISTER_SHELL=1
        else
            if printf '%s\n' "${BIN_DIR}/brash" >> "$SHELLS_FILE" 2>/dev/null; then
                note "appended ${BIN_DIR}/brash to $SHELLS_FILE"
                DID_REGISTER_SHELL=1
            else
                warn "--register-shell: failed to write $SHELLS_FILE; re-run as root"
                SKIPPED_REGISTER_REASON="permission denied on $SHELLS_FILE"
            fi
        fi
    else
        warn "--register-shell: cannot write to $SHELLS_FILE (re-run as root, e.g. sudo)"
        SKIPPED_REGISTER_REASON="not root and $SHELLS_FILE not writable"
    fi
fi

# ----- bash symlink (optional) -----------------------------------------------
DID_BASH_SYMLINK=0
SKIPPED_BASH_REASON=
if [ "$MAKE_DEFAULT_BASH" -eq 1 ]; then
    found_bash=
    OLD_IFS=$IFS
    IFS=:
    # Walk PATH; if any /bin/bash is found ahead of $BIN_DIR, decline.
    saw_prefix=0
    for d in $PATH; do
        [ -n "$d" ] || continue
        if [ "$d" = "$BIN_DIR" ]; then
            saw_prefix=1
            continue
        fi
        if [ -x "$d/bash" ]; then
            if [ "$saw_prefix" -eq 0 ]; then
                # Existing bash precedes BIN_DIR (or BIN_DIR isn't on PATH).
                found_bash=$d
                break
            fi
        fi
    done
    IFS=$OLD_IFS

    if [ -n "$found_bash" ]; then
        warn "--make-default-bash: $found_bash/bash precedes $BIN_DIR in PATH; skipping symlink"
        SKIPPED_BASH_REASON="$found_bash/bash is earlier in PATH"
    else
        if ln -sf brash "${BIN_DIR}/bash" 2>/dev/null; then
            note "symlinked ${BIN_DIR}/bash -> brash"
            DID_BASH_SYMLINK=1
        else
            warn "--make-default-bash: failed to create ${BIN_DIR}/bash (need sudo?)"
            SKIPPED_BASH_REASON="could not write ${BIN_DIR}/bash"
        fi
    fi
fi

# ----- post-install trailer --------------------------------------------------
printf '\n' >&2
note "Installed paths:"
note "  binary  : ${BIN_DIR}/brash"
[ -n "$INSTALLED_LICENSE" ] && note "  license : ${INSTALLED_LICENSE}"
if [ "$INSTALLED_MAN_COUNT" -gt 0 ]; then
    note "  man     : ${MAN_DIR}/man*/  (${INSTALLED_MAN_COUNT} page(s))"
fi
[ -n "$reported" ] && note "  reports : ${reported}"

if [ "$DID_REGISTER_SHELL" -eq 1 ]; then
    note "  shells  : ${BIN_DIR}/brash registered in /etc/shells"
fi
if [ "$DID_BASH_SYMLINK" -eq 1 ]; then
    note "  symlink : ${BIN_DIR}/bash -> brash"
fi

# PATH warning ---------------------------------------------------------------
case ":$PATH:" in
    *":${BIN_DIR}:"*) : ;;
    *)
        printf '\n' >&2
        warn "${BIN_DIR} is not in your \$PATH."
        warn "Add this to your shell rc (e.g. ~/.profile):"
        warn "    export PATH=\"${BIN_DIR}:\$PATH\""
        ;;
esac

# Recommendations for opt-ins the user did NOT take. -------------------------
need_blank=1
print_blank_once() {
    if [ "$need_blank" -eq 1 ]; then
        printf '\n' >&2
        need_blank=0
    fi
}

if [ "$DID_REGISTER_SHELL" -eq 0 ]; then
    print_blank_once
    note "To register brash as a system shell (so 'chsh -s ${BIN_DIR}/brash' works):"
    if [ -n "$SKIPPED_REGISTER_REASON" ]; then
        note "  (skipped earlier: ${SKIPPED_REGISTER_REASON})"
    fi
    note "  re-run with --register-shell, or as root:"
    note "    echo ${BIN_DIR}/brash | sudo tee -a /etc/shells"
fi

if [ "$DID_BASH_SYMLINK" -eq 0 ]; then
    print_blank_once
    note "To use brash as 'bash' (CAUTION: dangerous if /bin/bash is earlier on \$PATH):"
    if [ -n "$SKIPPED_BASH_REASON" ]; then
        note "  (skipped earlier: ${SKIPPED_BASH_REASON})"
    fi
    note "  re-run with --make-default-bash, or manually:"
    note "    ln -sf brash ${BIN_DIR}/bash"
fi

printf '\n' >&2
note "brash installed."

exit 0
