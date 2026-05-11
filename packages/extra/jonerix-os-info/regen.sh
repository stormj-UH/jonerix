#!/bin/sh
# /share/jonerix-os-info/regen.sh — regenerate distro-marker files from
# /etc/os-release.
#
# Reads VERSION_ID from /etc/os-release, substitutes it into the templates
# under /share/jonerix-os-info/, and writes the result to /etc/.
#
# Safe to re-run.  A target file is left alone if it has been hand-edited
# (its sha256 differs from the recorded "we last shipped this" hash in
# /var/lib/jonerix-os-info/shipped.sha256).  Otherwise it's overwritten.
# Newly-created files are recorded so the next regen recognises them.
#
# Idempotent on repeat invocation against an unmodified target.
#
# POSIX shell.  /bin/sh on jonerix is mksh, on most build hosts is dash
# or bash; this script targets the intersection.

set -e

DEST_ROOT=${DEST_ROOT:-}
SHARE=${SHARE:-${DEST_ROOT}/share/jonerix-os-info}
STATE_DIR=/var/lib/jonerix-os-info
STATE_FILE="$STATE_DIR/shipped.sha256"

OS_RELEASE=${OS_RELEASE:-${DEST_ROOT}/etc/os-release}

# Pull VERSION_ID out of /etc/os-release.  Strip optional surrounding
# double quotes (POSIX-safe, no GNU sed -E).
if [ ! -r "$OS_RELEASE" ]; then
    echo "jonerix-os-info: cannot read $OS_RELEASE" >&2
    exit 1
fi
VERSION_ID=$(
    awk -F= '/^VERSION_ID=/ {
        v = $2
        gsub(/^"/, "", v)
        gsub(/"$/, "", v)
        print v
        exit
    }' "$OS_RELEASE"
)
if [ -z "$VERSION_ID" ]; then
    echo "jonerix-os-info: VERSION_ID not found in $OS_RELEASE" >&2
    exit 1
fi

mkdir -p "${DEST_ROOT}$STATE_DIR"

# sha256_of FILE — print just the hex digest (toybox sha256sum or
# openssl dgst as a fallback).  Empty string when FILE is missing.
sha256_of() {
    if [ ! -f "$1" ]; then
        printf ''
        return 0
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        # LibreSSL ships /bin/openssl.  `dgst -sha256` prints
        #   SHA2-256(file)= <hex>
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    fi
}

# lookup_shipped DEST_PATH — print the recorded sha256 for DEST_PATH
# from the state file, or empty if no record.
lookup_shipped() {
    [ -f "${DEST_ROOT}$STATE_FILE" ] || { printf ''; return 0; }
    awk -v path="$1" '$2 == path { print $1; exit }' "${DEST_ROOT}$STATE_FILE"
}

# record_shipped DEST_PATH HASH — rewrite the state file with the new
# (path, hash) pairing.  Removes any prior entry for the same path.
record_shipped() {
    _path=$1
    _hash=$2
    _state="${DEST_ROOT}$STATE_FILE"
    _tmp="${_state}.tmp.$$"
    if [ -f "$_state" ]; then
        awk -v path="$_path" '$2 != path' "$_state" > "$_tmp"
    else
        : > "$_tmp"
    fi
    printf '%s  %s\n' "$_hash" "$_path" >> "$_tmp"
    mv "$_tmp" "$_state"
}

# render TEMPLATE — write template content with ${VERSION_ID}
# substituted, to stdout.  Pure shell (no sed -i, no envsubst) so we
# don't depend on either tool's flavour.
render() {
    awk -v ver="$VERSION_ID" '{
        gsub(/\$\{VERSION_ID\}/, ver)
        print
    }' "$1"
}

# install_file TEMPLATE DEST_REL_PATH MODE
#
# Render TEMPLATE with the current VERSION_ID and write it to
# ${DEST_ROOT}/DEST_REL_PATH, but only if:
#   - the destination does not exist, OR
#   - the destination's current sha256 matches what we last shipped
#     (i.e. nobody hand-edited it).
# Otherwise leave the destination alone and log to stderr.  In all
# success paths, record the resulting (path, hash) so the next regen
# recognises it.
install_file() {
    _tmpl=$1
    _dest_rel=$2
    _mode=$3
    _dest="${DEST_ROOT}$_dest_rel"
    _tmp="${_dest}.jonerix-os-info.tmp.$$"

    if [ ! -f "$_tmpl" ]; then
        echo "jonerix-os-info: missing template $_tmpl" >&2
        return 1
    fi

    # Render to a temp file so we know the would-be hash before we
    # decide whether to overwrite.
    mkdir -p "$(dirname "$_dest")"
    render "$_tmpl" > "$_tmp"
    chmod "$_mode" "$_tmp"
    _new_hash=$(sha256_of "$_tmp")

    if [ -e "$_dest" ] && [ ! -L "$_dest" ]; then
        _cur_hash=$(sha256_of "$_dest")
        if [ "$_cur_hash" = "$_new_hash" ]; then
            # Already correct.  Update the state record so future
            # regens recognise this file even if it predates state.
            rm -f "$_tmp"
            record_shipped "$_dest_rel" "$_new_hash"
            return 0
        fi
        _shipped_hash=$(lookup_shipped "$_dest_rel")
        if [ -n "$_shipped_hash" ] && [ "$_cur_hash" != "$_shipped_hash" ]; then
            echo "jonerix-os-info: $_dest_rel is hand-edited, leaving alone" >&2
            rm -f "$_tmp"
            return 0
        fi
        if [ -z "$_shipped_hash" ] && [ "$_cur_hash" != "$_new_hash" ]; then
            # No state record + existing file we didn't write.  Be
            # conservative: don't clobber a file we don't recognise.
            echo "jonerix-os-info: $_dest_rel exists and is not tracked, leaving alone" >&2
            rm -f "$_tmp"
            return 0
        fi
    elif [ -L "$_dest" ]; then
        # Existing symlink at this path is unexpected for a regular
        # file target — leave it alone.
        echo "jonerix-os-info: $_dest_rel is a symlink, leaving alone" >&2
        rm -f "$_tmp"
        return 0
    fi

    mv "$_tmp" "$_dest"
    chmod "$_mode" "$_dest"
    record_shipped "$_dest_rel" "$_new_hash"
}

# ensure_symlink DEST_REL_PATH TARGET
#
# Manage /etc/system-release as a symlink to jonerix-release.  Same
# leave-alone-if-hand-edited rule as install_file, but the "hash" is
# the link target itself.
ensure_symlink() {
    _dest_rel=$1
    _target=$2
    _dest="${DEST_ROOT}$_dest_rel"
    _marker="symlink:$_target"

    if [ -L "$_dest" ]; then
        _cur=$(readlink "$_dest")
        if [ "$_cur" = "$_target" ]; then
            record_shipped "$_dest_rel" "$_marker"
            return 0
        fi
        _shipped=$(lookup_shipped "$_dest_rel")
        if [ -n "$_shipped" ] && [ "$_shipped" != "symlink:$_cur" ]; then
            echo "jonerix-os-info: $_dest_rel symlink target hand-edited, leaving alone" >&2
            return 0
        fi
    elif [ -e "$_dest" ]; then
        _shipped=$(lookup_shipped "$_dest_rel")
        if [ -z "$_shipped" ]; then
            echo "jonerix-os-info: $_dest_rel exists as a regular file, leaving alone" >&2
            return 0
        fi
    fi

    mkdir -p "$(dirname "$_dest")"
    rm -f "$_dest"
    ln -s "$_target" "$_dest"
    record_shipped "$_dest_rel" "$_marker"
}

install_file "$SHARE/lsb-release.tmpl"         /etc/lsb-release         644
install_file "$SHARE/jonerix-release.tmpl"     /etc/jonerix-release     644
install_file "$SHARE/system-release-cpe.tmpl"  /etc/system-release-cpe  644
install_file "$SHARE/issue.tmpl"               /etc/issue               644
install_file "$SHARE/issue.net.tmpl"           /etc/issue.net           644
ensure_symlink                                 /etc/system-release      jonerix-release

exit 0
