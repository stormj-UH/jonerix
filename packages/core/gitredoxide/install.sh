#!/bin/sh
# gitredoxide installer
#
# Downloads a published gitredoxide .jpkg release and installs it
# under a prefix.  By default ONLY the primary binary, license, and
# bundled man pages are installed; helper binaries (git-upload-pack,
# git-receive-pack) are opt-in.  Default rename is `git-redoxide`
# so the script does not clobber an existing /usr/bin/git.  Pass
# `--rename-bin git` to install as plain `git` for full takeover.
#
# POSIX shell only: must run under dash, mksh, busybox ash.

set -eu

PROG=$(basename "$0")
DEFAULT_VERSION="1.0.22"
DEFAULT_PREFIX="/usr/local"
DEFAULT_RENAME="git-redoxide"
RELEASE_BASE="https://github.com/stormj-UH/jonerix/releases/download/packages"

VERSION="$DEFAULT_VERSION"
PREFIX="$DEFAULT_PREFIX"
RENAME_BIN="$DEFAULT_RENAME"
ARCH=""

# Tri-state: empty = ask (or default to no), 1 = yes, 0 = no.
WITH_HELPERS=""
NO_PROMPT=0
RENAME_EXPLICIT=0

usage() {
    cat <<EOF
Usage: $PROG [options]

Install gitredoxide, a permissively licensed drop-in for /bin/git
built on the gitoxide gix-* crate ecosystem.

Options:
  --version <VER>      Package version to install (default: $DEFAULT_VERSION)
  --prefix <DIR>       Install prefix (default: $DEFAULT_PREFIX)
  --arch <ARCH>        Target architecture; auto-detected from \`uname -m\`
                       if omitted.  Supported: x86_64, aarch64.
  --rename-bin <NAME>  Install the primary binary as <NAME> rather than
                       the default '$DEFAULT_RENAME'.  Use
                       '--rename-bin git' to take over plain 'git'.
  --with-helpers       Also install server helper binaries
                       (git-upload-pack, git-receive-pack).  Required
                       only if you want to host repositories with
                       gitredoxide acting as the git server.
  --no-helpers         Force-skip helper binaries (default; useful to
                       suppress the interactive prompt explicitly).
  --no-prompt, --yes   Never prompt; use defaults / flags only.
  --help, -h           Show this message.

Long-form --key=value is accepted for every flag that takes an
argument.

The default rename ('$DEFAULT_RENAME') avoids shadowing an existing
/bin/git or /usr/bin/git.  By default ONLY the primary binary, the
license, and (if the payload bundles them) man pages are installed.
Helper binaries used for serving repositories are opt-in.

The installer fetches:
  $RELEASE_BASE/gitredoxide-<VERSION>-<ARCH>.jpkg

Required tools: curl or wget; zstd; tar; od; dd.
EOF
}

err() {
    printf '%s: error: %s\n' "$PROG" "$*" >&2
}

die() {
    err "$@"
    exit 1
}

note() {
    printf '%s: %s\n' "$PROG" "$*"
}

# Argument parsing.  Supports --flag value and --flag=value for
# every flag that takes an argument.
while [ $# -gt 0 ]; do
    case $1 in
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
        --rename-bin)
            [ $# -ge 2 ] || die "--rename-bin requires an argument"
            RENAME_BIN=$2
            RENAME_EXPLICIT=1
            shift 2
            ;;
        --rename-bin=*)
            RENAME_BIN=${1#--rename-bin=}
            RENAME_EXPLICIT=1
            shift
            ;;
        --with-helpers)
            WITH_HELPERS=1
            shift
            ;;
        --no-helpers)
            WITH_HELPERS=0
            shift
            ;;
        --no-prompt|--yes)
            NO_PROMPT=1
            shift
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
            err "unknown option: $1"
            usage >&2
            exit 2
            ;;
        *)
            err "unexpected argument: $1"
            usage >&2
            exit 2
            ;;
    esac
done

[ -n "$VERSION" ] || die "--version cannot be empty"
[ -n "$PREFIX" ]  || die "--prefix cannot be empty"
[ -n "$RENAME_BIN" ] || die "--rename-bin cannot be empty"

# Architecture detection.  Map common uname -m strings to the names
# used in published .jpkg asset filenames.
if [ -z "$ARCH" ]; then
    UNAME_M=$(uname -m 2>/dev/null || echo unknown)
    case $UNAME_M in
        x86_64|amd64)            ARCH=x86_64 ;;
        aarch64|arm64)           ARCH=aarch64 ;;
        *)
            die "could not auto-detect architecture (uname -m: $UNAME_M); pass --arch x86_64 or --arch aarch64"
            ;;
    esac
fi

# Tool checks.  We need exactly one of curl / wget for download,
# plus zstd, tar, od, dd for extraction.
have() {
    command -v "$1" >/dev/null 2>&1
}

if have curl; then
    DOWNLOADER=curl
elif have wget; then
    DOWNLOADER=wget
else
    die "need curl or wget to download the package"
fi

for tool in zstd tar od dd; do
    have "$tool" || die "missing required tool: $tool"
done

# Determine whether we may prompt the user interactively.  Both stdin
# and stdout must be a real terminal AND /dev/tty must be readable;
# piped `curl ... | sh` will not satisfy these checks and we fall
# through to defaults.
INTERACTIVE=0
if [ "$NO_PROMPT" -eq 0 ] && [ -t 1 ] && [ -r /dev/tty ]; then
    INTERACTIVE=1
fi

# Read a yes/no answer.  $1 is the prompt text, $2 is the default
# ("y" or "n").  Echoes "y" or "n" on stdout.  POSIX read with -r so
# backslashes in user input are not mangled.
prompt_yn() {
    _q=$1
    _default=$2
    _suffix='[y/N]'
    if [ "$_default" = "y" ]; then
        _suffix='[Y/n]'
    fi
    while :; do
        printf '%s %s ' "$_q" "$_suffix" > /dev/tty
        if ! read -r _ans < /dev/tty; then
            _ans=""
        fi
        case $_ans in
            '')
                if [ "$_default" = "y" ]; then
                    echo y
                else
                    echo n
                fi
                return 0
                ;;
            y|Y|yes|YES|Yes)
                echo y
                return 0
                ;;
            n|N|no|NO|No)
                echo n
                return 0
                ;;
            *)
                printf '  please answer y or n\n' > /dev/tty
                ;;
        esac
    done
}

# Resolve helper-install opt-in.
if [ -z "$WITH_HELPERS" ]; then
    if [ "$INTERACTIVE" -eq 1 ]; then
        _ans=$(prompt_yn "Install git-upload-pack and git-receive-pack helpers? They're needed if you want to use this as a git server." n)
        if [ "$_ans" = "y" ]; then
            WITH_HELPERS=1
        else
            WITH_HELPERS=0
        fi
    else
        WITH_HELPERS=0
    fi
fi

# Resolve git-takeover opt-in.  Only fires interactively when the
# user did NOT pass --rename-bin on the command line; if they did,
# we honor it without re-asking.
if [ "$RENAME_EXPLICIT" -eq 0 ] && [ "$INTERACTIVE" -eq 1 ]; then
    _ans=$(prompt_yn "Replace 'git-redoxide' with 'git' (will shadow /usr/bin/git if \$PREFIX/bin is ahead on PATH)?" n)
    if [ "$_ans" = "y" ]; then
        RENAME_BIN=git
    fi
fi

# Workspace + cleanup trap.
WORKDIR=$(mktemp -d 2>/dev/null || mktemp -d -t gitredoxide-install)
[ -n "$WORKDIR" ] && [ -d "$WORKDIR" ] || die "could not create temp dir"

# shellcheck disable=SC2329  # invoked indirectly by trap
cleanup() {
    rm -rf "$WORKDIR"
}
trap cleanup EXIT INT HUP TERM

ASSET="gitredoxide-${VERSION}-${ARCH}.jpkg"
URL="${RELEASE_BASE}/${ASSET}"
JPKG="${WORKDIR}/${ASSET}"

note "downloading $URL"
case $DOWNLOADER in
    curl)
        curl -fsSL --retry 3 --retry-delay 2 -o "$JPKG" "$URL" \
            || die "download failed: $URL"
        ;;
    wget)
        wget -q --tries=3 -O "$JPKG" "$URL" \
            || die "download failed: $URL"
        ;;
esac

[ -s "$JPKG" ] || die "downloaded file is empty: $JPKG"

# Verify JPKG magic.  The header starts with the four ASCII bytes
# 'JPKG'.  Any other prefix means the asset is corrupt or the URL
# served HTML (e.g. a 404 page).
MAGIC=$(dd if="$JPKG" bs=1 count=4 status=none 2>/dev/null || true)
if [ "$MAGIC" != "JPKG" ]; then
    die "not a JPKG archive (bad magic): $JPKG"
fi

# Read metadata length (little-endian u32 at offset 8) so we can
# skip past the header + metadata to the zstd-compressed tar payload.
MD_LEN=$(od -An -tu4 -N4 -j8 "$JPKG" 2>/dev/null | tr -d ' ' || true)
case $MD_LEN in
    ''|*[!0-9]*) die "could not read metadata length from $JPKG" ;;
esac

PAYLOAD_OFFSET=$((12 + MD_LEN))
EXTRACT="${WORKDIR}/extract"
mkdir -p "$EXTRACT"

note "extracting payload (offset $PAYLOAD_OFFSET)"
dd if="$JPKG" bs=1 skip="$PAYLOAD_OFFSET" status=none \
    | zstd -d -q \
    | tar -x -C "$EXTRACT" \
    || die "failed to extract payload"

[ -d "$EXTRACT/bin" ] || die "payload missing bin/ directory"

# Verify the primary binary is present so we can rename it
# correctly during install.
PRIMARY_REAL=""
if [ -f "$EXTRACT/bin/gitredoxide" ]; then
    PRIMARY_REAL=gitredoxide
elif [ -f "$EXTRACT/bin/git" ] && [ ! -L "$EXTRACT/bin/git" ]; then
    PRIMARY_REAL=git
else
    die "payload bin/ does not contain the gitredoxide binary"
fi

# Pre-create install directories.
DEST_BIN="${PREFIX}/bin"
DEST_LIC="${PREFIX}/share/licenses/gitredoxide"
DEST_MAN="${PREFIX}/share/man"
mkdir -p "$DEST_BIN" "$DEST_LIC" \
    || die "could not create install directories under $PREFIX"

# Helper names that we install only when --with-helpers is set.
# Anything matching one of these on the bin/ side is skipped by
# default.  Server helpers are listed here.
is_helper_name() {
    case $1 in
        git-upload-pack|git-receive-pack|git-upload-archive)
            return 0
            ;;
    esac
    return 1
}

# Install from bin/.
#
# Default behaviour: install ONLY the primary binary (renamed to
# $RENAME_BIN), and skip everything else.  When --with-helpers is
# set, additionally install the server helper binaries listed by
# is_helper_name().  Any published 'git' alias-symlink is dropped:
# if the user wants the binary to BE 'git', we install the primary
# under that name directly via $RENAME_BIN.
note "installing to $DEST_BIN"
INSTALLED_NAMES=""
SKIPPED_HELPERS=""
for src in "$EXTRACT/bin/"*; do
    [ -e "$src" ] || continue
    name=$(basename "$src")

    case $name in
        "$PRIMARY_REAL")
            target_name=$RENAME_BIN
            ;;
        git)
            # Published alias to the primary binary.  Always skip;
            # the primary is already installed under $RENAME_BIN.
            continue
            ;;
        *)
            if is_helper_name "$name"; then
                if [ "$WITH_HELPERS" -ne 1 ]; then
                    SKIPPED_HELPERS="${SKIPPED_HELPERS}${SKIPPED_HELPERS:+ }${name}"
                    continue
                fi
                target_name=$name
            else
                # Anything else in bin/ is treated as a non-default
                # extra and skipped unless --with-helpers requested
                # the full set.
                if [ "$WITH_HELPERS" -ne 1 ]; then
                    continue
                fi
                target_name=$name
            fi
            ;;
    esac

    dest="${DEST_BIN}/${target_name}"

    if [ -L "$src" ]; then
        link_target=$(readlink "$src")
        case $link_target in
            "$PRIMARY_REAL")
                link_target=$RENAME_BIN
                ;;
        esac
        rm -f "$dest"
        ln -s "$link_target" "$dest" \
            || die "failed to create symlink $dest -> $link_target"
    else
        rm -f "$dest"
        # Use cat + chmod rather than `install` (not POSIX) or
        # `cp -p` (preserves modes we may not want).
        cat "$src" > "$dest" \
            || die "failed to copy $src to $dest"
        chmod 0755 "$dest" \
            || die "failed to set permissions on $dest"
    fi

    INSTALLED_NAMES="${INSTALLED_NAMES}${INSTALLED_NAMES:+ }${target_name}"
done

# License: install if the payload bundles one, otherwise note that
# the published terms (MIT OR Apache-2.0) live in the source repo.
LICENSE_INSTALLED=
for cand in LICENSE LICENSE-MIT LICENSE.txt licenses/LICENSE licenses/LICENSE-MIT; do
    if [ -f "$EXTRACT/$cand" ]; then
        cat "$EXTRACT/$cand" > "${DEST_LIC}/LICENSE" \
            || die "failed to install license"
        chmod 0644 "${DEST_LIC}/LICENSE" || true
        LICENSE_INSTALLED=1
        break
    fi
done

if [ -z "$LICENSE_INSTALLED" ]; then
    cat > "${DEST_LIC}/LICENSE" <<'EOF'
gitredoxide is licensed MIT OR Apache-2.0.

Full license text:
  https://github.com/stormj-UH/gitredoxide/blob/main/LICENSE-MIT

Upstream gitoxide code is also MIT OR Apache-2.0; this project
accepts the MIT option.  No GPL Git source code is included.
EOF
    chmod 0644 "${DEST_LIC}/LICENSE" || true
fi

# Man pages: copy through whatever the payload bundles under
# share/man/ if anything.  Always part of the default install.
MAN_INSTALLED_COUNT=0
if [ -d "$EXTRACT/share/man" ]; then
    for mansec in "$EXTRACT/share/man/"man*; do
        [ -d "$mansec" ] || continue
        secname=$(basename "$mansec")
        mkdir -p "${DEST_MAN}/${secname}" \
            || die "could not create $DEST_MAN/$secname"
        for page in "$mansec/"*; do
            [ -f "$page" ] || continue
            pname=$(basename "$page")
            cat "$page" > "${DEST_MAN}/${secname}/${pname}" \
                || die "failed to install man page $pname"
            chmod 0644 "${DEST_MAN}/${secname}/${pname}" || true
            MAN_INSTALLED_COUNT=$((MAN_INSTALLED_COUNT + 1))
        done
    done
fi

# Verify the renamed primary actually runs.
INSTALLED_PRIMARY="${DEST_BIN}/${RENAME_BIN}"
[ -x "$INSTALLED_PRIMARY" ] || die "installed binary not executable: $INSTALLED_PRIMARY"

note "verifying $INSTALLED_PRIMARY --version"
if ! "$INSTALLED_PRIMARY" --version >/dev/null 2>&1; then
    # Some hosts (e.g. macOS with a Linux musl binary) cannot
    # execute the payload at all.  Treat that as a hard failure so
    # the install metadata is still useful, but exit non-zero.
    err "installed binary does not run: $INSTALLED_PRIMARY --version failed"
    err "the package may target a different OS or architecture than this host"
    exit 1
fi
"$INSTALLED_PRIMARY" --version || true

# PATH / shadowing checks.
PATH_OK=1
case ":${PATH:-}:" in
    *":${DEST_BIN}:"*) ;;
    *) PATH_OK=0 ;;
esac

OTHER_GIT=""
if [ "$RENAME_BIN" = "git" ]; then
    OLD_IFS=$IFS
    IFS=:
    for d in $PATH; do
        [ -n "$d" ] || continue
        if [ "$d" = "$DEST_BIN" ]; then
            break
        fi
        if [ -x "$d/git" ] && [ "$d/git" != "$INSTALLED_PRIMARY" ]; then
            OTHER_GIT="$d/git"
            break
        fi
    done
    IFS=$OLD_IFS
fi

# Post-install trailer.  Single block so the user sees a coherent
# summary even when piped through `sh`.
echo
echo "Installed: $INSTALLED_PRIMARY (gitredoxide $VERSION, $ARCH)"
echo "Bin dir:   $DEST_BIN"
echo "Files:     $INSTALLED_NAMES"
if [ "$MAN_INSTALLED_COUNT" -gt 0 ]; then
    echo "Man pages: $MAN_INSTALLED_COUNT under $DEST_MAN"
else
    echo "Man pages: (none bundled in this payload)"
fi
echo "License:   ${DEST_LIC}/LICENSE"
echo

if [ "$PATH_OK" -eq 0 ]; then
    echo "PATH note"
    echo "  ${DEST_BIN} is not currently on \$PATH."
    echo "  Add it to your shell profile, e.g.:"
    echo "    PATH=\"${DEST_BIN}:\$PATH\""
    echo
fi

if [ "$RENAME_BIN" = "git" ] && [ -n "$OTHER_GIT" ]; then
    echo "WARNING: another 'git' is ahead of $DEST_BIN on \$PATH:"
    echo "    $OTHER_GIT"
    echo "  Running plain 'git' will keep invoking that binary, not gitredoxide."
    echo "  Either remove the other git, reorder \$PATH so $DEST_BIN comes first,"
    echo "  or reinstall with the default --rename-bin git-redoxide."
    echo
fi

if [ "$WITH_HELPERS" -ne 1 ]; then
    echo "Helpers"
    echo "  Skipped server helpers (git-upload-pack, git-receive-pack)."
    if [ -n "$SKIPPED_HELPERS" ]; then
        echo "  Available in this payload: $SKIPPED_HELPERS"
    fi
    echo "  To enable hosting later, rerun with --with-helpers:"
    echo "    sh install.sh --prefix \"$PREFIX\" --with-helpers --no-prompt"
    echo
fi

if [ "$RENAME_BIN" != "git" ]; then
    echo "Takeover"
    echo "  The primary binary was installed as '$RENAME_BIN', NOT 'git',"
    echo "  so your system /usr/bin/git is untouched.  To take over plain"
    echo "  'git' later, rerun with:"
    echo "    sh install.sh --prefix \"$PREFIX\" --rename-bin git --no-prompt"
    echo
fi

echo "Compatibility matrix"
echo "  43 commands (29 porcelain, 12 plumbing, 2 global) are implemented."
echo "  See the project README for the current command matrix:"
echo "    https://github.com/stormj-UH/gitredoxide#implemented-commands"
echo
echo "Note: this installer never touches the system git binary or the"
echo "system /etc/gitconfig; only files under \$PREFIX are written."
echo

exit 0
