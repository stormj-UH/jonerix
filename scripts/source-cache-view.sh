#!/bin/sh
# source-cache-view.sh — build a JPKG_SOURCE_CACHE directory that leaves out
# Git LFS pointer files, without touching the checkout.
#
# Usage:
#   JPKG_SOURCE_CACHE=$(sh scripts/source-cache-view.sh SRC_DIR VIEW_DIR [LABEL])
#
# A checkout made without LFS content (actions/checkout `lfs: false`, or a
# build host with no git-lfs) has ~130-byte pointer files in sources/ in
# place of the LFS-tracked tarballs. jpkg's cache lookup would copy such a
# pointer and then abort on the sha256 check, so the pointers must not be
# visible in the cache. SRC_DIR is usually a host checkout bind-mounted
# into the builder container (scripts/local-build-*.sh), so it is never
# modified: VIEW_DIR (created if needed; it may only ever hold symlinks) is
# refilled with a symlink to every regular file in SRC_DIR except the LFS
# pointers. jpkg's fs::copy() and the recipes' `[ -f ]` lookups both follow
# symlinks.
#
# Prints VIEW_DIR on stdout; progress goes to stderr.
#
# SPDX-License-Identifier: MIT

set -eu

src_dir=${1:?usage: source-cache-view.sh SRC_DIR VIEW_DIR [LABEL]}
view_dir=${2:?usage: source-cache-view.sh SRC_DIR VIEW_DIR [LABEL]}
label=${3:-source-cache-view}

case "$view_dir" in
    /|/tmp|/var/tmp|'')
        printf '%s: refusing to use %s as the view directory\n' "$label" "$view_dir" >&2
        exit 2
        ;;
esac

src_dir=$(cd "$src_dir" && pwd -P)
mkdir -p "$view_dir"
view_dir=$(cd "$view_dir" && pwd -P)
if [ "$src_dir" = "$view_dir" ]; then
    printf '%s: view directory must differ from %s\n' "$label" "$src_dir" >&2
    exit 2
fi

# Start from an empty view so files dropped from SRC_DIR since the last run
# do not linger as dangling links. A view holds only symlinks, so anything
# else means VIEW_DIR is some other directory: stop rather than delete it.
for old in "$view_dir"/* "$view_dir"/.[!.]*; do
    if [ -L "$old" ]; then
        rm -f "$old"
    elif [ -e "$old" ]; then
        printf '%s: %s holds %s, which is not a symlink; refusing to reuse it as a source-cache view\n' \
            "$label" "$view_dir" "${old##*/}" >&2
        exit 2
    fi
done

linked=0
pointers=0
for src in "$src_dir"/*; do
    [ -f "$src" ] || continue
    first_line=
    IFS= read -r first_line < "$src" 2>/dev/null || true
    if [ "$first_line" = "version https://git-lfs.github.com/spec/v1" ]; then
        pointers=$((pointers + 1))
        continue
    fi
    ln -s "$src" "$view_dir/${src##*/}"
    linked=$((linked + 1))
done

if [ "$pointers" -gt 0 ]; then
    printf '%s: %s source(s) cached, %s LFS pointer(s) in %s left out (jpkg will fetch those from their upstream URLs)\n' \
        "$label" "$linked" "$pointers" "$src_dir" >&2
else
    printf '%s: %s source(s) cached from %s\n' "$label" "$linked" "$src_dir" >&2
fi

printf '%s\n' "$view_dir"
