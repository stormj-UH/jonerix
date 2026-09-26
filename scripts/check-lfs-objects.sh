#!/bin/sh
# Verify every Git LFS-tracked file is committed as an LFS pointer and that
# the object it points at exists on the LFS server.
#
# Usage:
#   sh scripts/check-lfs-objects.sh [REPO_ROOT]
#
# Env:
#   LFS_URL   LFS endpoint(s) to ask first, space-separated. The checkout's
#             own endpoint (`git lfs env`'s Endpoint, else the origin
#             remote's https URL + .git/info/lfs) is always asked after
#             them, so a pull request can name its head repository here
#             and objects already on the base repository still count.
#
# Catches the two ways a vendored LFS tarball silently goes missing:
#   * the file was committed without git-lfs installed, so the full archive
#     landed in Git history instead of a pointer;
#   * the pointer was pushed but its object never was (a clone without the
#     git-lfs pre-push hook), so every lfs:true checkout and `git lfs pull`
#     of that path fails.
# Only the batch API's object metadata is requested; nothing is downloaded,
# so this costs no LFS bandwidth and works with an `lfs: false` checkout.
#
# SPDX-License-Identifier: MIT

set -eu
# Nothing below needs pathname expansion; URLs and paths split literally.
set -f

ROOT=${1:-.}
cd "$ROOT"

LFS_SPEC="version https://git-lfs.github.com/spec/v1"
failures=0

fail() {
    printf '%s\n' "$*" >&2
    failures=$((failures + 1))
}

default_endpoint() {
    if git lfs version >/dev/null 2>&1; then
        endpoint=$(git lfs env 2>/dev/null |
            sed -n 's/^Endpoint=\([^ ]*\).*/\1/p' | head -n 1)
        if [ -n "$endpoint" ]; then
            printf '%s\n' "$endpoint"
            return 0
        fi
    fi
    remote=$(git config --get remote.origin.url 2>/dev/null || true)
    case "$remote" in
        git@*:*)
            host=${remote#git@}
            host=${host%%:*}
            path=${remote#*:}
            remote="https://${host}/${path}"
            ;;
        ssh://git@*)
            remote="https://${remote#ssh://git@}"
            ;;
        https://*) ;;
        *)
            return 1
            ;;
    esac
    remote=${remote%/}
    case "$remote" in
        *.git) ;;
        *) remote="${remote}.git" ;;
    esac
    printf '%s/info/lfs\n' "$remote"
}

endpoints=
for ep in ${LFS_URL:-} $(default_endpoint || true); do
    case " $endpoints " in
        *" $ep "*) ;;
        *) endpoints="${endpoints:+$endpoints }$ep" ;;
    esac
done
if [ -z "$endpoints" ]; then
    printf 'LFS: cannot determine the LFS endpoint; set LFS_URL\n' >&2
    exit 1
fi

paths=$(git ls-files ':(attr:filter=lfs)')
if [ -z "$paths" ]; then
    printf 'LFS: no LFS-tracked files\n'
    exit 0
fi

# LFS-tracked paths (sources/) contain no whitespace.
checked=0
for path in $paths; do
    pointer=$(git cat-file blob ":$path" 2>/dev/null | dd bs=512 count=1 2>/dev/null || true)
    first=$(printf '%s\n' "$pointer" | head -n 1)
    if [ "$first" != "$LFS_SPEC" ]; then
        fail "LFS: $path is tracked by .gitattributes but committed as a regular blob, not an LFS pointer (install git-lfs, then git rm --cached and re-add it)"
        continue
    fi
    oid=$(printf '%s\n' "$pointer" | sed -n 's/^oid sha256:\([0-9a-f]\{64\}\)$/\1/p')
    size=$(printf '%s\n' "$pointer" | sed -n 's/^size \([0-9][0-9]*\)$/\1/p')
    if [ -z "$oid" ] || [ -z "$size" ]; then
        fail "LFS: $path has a malformed LFS pointer"
        continue
    fi

    body="{\"operation\":\"download\",\"transfers\":[\"basic\"],\"objects\":[{\"oid\":\"$oid\",\"size\":$size}]}"
    found=
    problem=
    for ep in $endpoints; do
        if ! reply=$(curl -fsS --retry 3 --retry-delay 2 \
                -H 'Accept: application/vnd.git-lfs+json' \
                -H 'Content-Type: application/vnd.git-lfs+json' \
                -d "$body" "$ep/objects/batch"); then
            problem="batch request failed at $ep"
            continue
        fi
        case "$reply" in
            *'"error"'*)
                ;;
            *'"download"'*)
                found=$ep
                break
                ;;
            *)
                problem="unexpected batch reply from $ep: $reply"
                ;;
        esac
    done
    if [ -n "$found" ]; then
        checked=$((checked + 1))
    elif [ -n "$problem" ]; then
        fail "LFS: cannot confirm the object for $path (oid $oid): $problem"
    else
        fail "LFS: object for $path (oid $oid) is not on the server: push it with \`git lfs push --object-id origin $oid\`"
    fi
done

if [ "$failures" -ne 0 ]; then
    printf 'LFS object check failed: %s issue(s)\n' "$failures" >&2
    exit 1
fi

printf 'LFS object check passed: %s object(s) present (%s)\n' "$checked" "$endpoints"
