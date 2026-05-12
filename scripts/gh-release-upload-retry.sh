#!/bin/sh
# Upload one or more files to a GitHub release with conservative handling for
# transient GitHub App installation API limits.
#
# SPDX-License-Identifier: MIT

set -eu

usage() {
    printf 'usage: %s <tag> <repo> <file> [file ...]\n' "$0" >&2
}

if [ "$#" -lt 3 ]; then
    usage
    exit 64
fi

tag=$1
repo=$2
shift 2

max_attempts=${GH_RELEASE_UPLOAD_ATTEMPTS:-5}
wait_seconds=${GH_RELEASE_UPLOAD_WAIT:-20}
tmp=${TMPDIR:-/tmp}/gh-release-upload.$$

cleanup() {
    rm -f "$tmp"
}
trap cleanup EXIT HUP INT TERM

attempt=1
while :; do
    set +e
    gh release upload "$tag" --repo "$repo" --clobber "$@" >"$tmp" 2>&1
    status=$?
    set -e

    if [ "$status" -eq 0 ]; then
        cat "$tmp"
        exit 0
    fi
    cat "$tmp" >&2

    if grep 'API rate limit exceeded for installation' "$tmp" >/dev/null 2>&1; then
        if [ "$attempt" -ge "$max_attempts" ]; then
            printf '::warning::GitHub installation API rate limit persisted after %s attempts; release upload skipped. Workflow artifact upload already preserved the built output.\n' "$attempt" >&2
            exit 0
        fi
        printf '::warning::GitHub installation API rate limit hit during release upload; retrying in %s seconds (attempt %s/%s).\n' "$wait_seconds" "$attempt" "$max_attempts" >&2
        sleep "$wait_seconds"
        attempt=$((attempt + 1))
        wait_seconds=$((wait_seconds * 2))
        if [ "$wait_seconds" -gt 300 ]; then
            wait_seconds=300
        fi
        continue
    fi

    exit "$status"
done
