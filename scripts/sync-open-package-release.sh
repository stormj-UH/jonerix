#!/bin/sh
# Mirror freshly published jpkg assets into the currently open package release.
#
# State is stored as a GitHub release asset:
#   release: package-release-state
#   asset:   active-package-release.env
#
# When no active state asset exists this script is a no-op. When a tag is open,
# the selected package assets are uploaded to that tag, then the tag's INDEX is
# regenerated, stale superseded package assets are deleted, and INDEX.zst is
# signed.

set -eu

repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"
workspace="${GITHUB_WORKSPACE:-$(pwd)}"
pkg_dir="${PKG_DIR:-/var/cache/jpkg}"
recipes_root="${RECIPES_ROOT:-$workspace/packages}"
stale_list="${STALE_LIST:-$pkg_dir/.stale-assets}"
asset_list="${OPEN_RELEASE_ASSET_LIST:-}"
pkg_input="${PKG_INPUT:-}"
state_release="${PACKAGE_RELEASE_STATE_TAG:-package-release-state}"
state_asset="${PACKAGE_RELEASE_STATE_ASSET:-active-package-release.env}"
signer_image="${SIGNER_IMAGE:-ghcr.io/stormj-uh/jonerix:builder-amd64}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

state_dir="$tmp/state"
mkdir -p "$state_dir"
if ! gh release download "$state_release" \
    --repo "$repo" \
    --pattern "$state_asset" \
    --dir "$state_dir" >/dev/null 2>&1; then
    echo "No open package release state found; rolling packages only."
    exit 0
fi

state_file="$state_dir/$state_asset"
active_tag="$(sed -n 's/^tag=//p' "$state_file" | tr -d '\r' | sed -n '1p')"
if [ -z "$active_tag" ]; then
    echo "Open package release state has no tag= line." >&2
    exit 1
fi
if ! printf '%s\n' "$active_tag" | grep '^v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*$' >/dev/null 2>&1; then
    echo "Open package release tag is not a vX.Y.Z tag: $active_tag" >&2
    exit 1
fi

if ! gh release view "$active_tag" --repo "$repo" >/dev/null 2>&1; then
    gh release create "$active_tag" \
        --repo "$repo" \
        --title "jonerix ${active_tag#v}" \
        --notes "Open jonerix package release ${active_tag}." \
        --prerelease
fi

upload_dir="$tmp/upload"
mkdir -p "$upload_dir"
count=0
for pkg in "$pkg_dir"/*.jpkg; do
    [ -f "$pkg" ] || continue
    base="$(basename "$pkg")"
    if [ -s "$stale_list" ] && grep -Fx "$base" "$stale_list" >/dev/null 2>&1; then
        continue
    fi
    if [ -n "$asset_list" ]; then
        if [ ! -s "$asset_list" ] || ! grep -Fx "$base" "$asset_list" >/dev/null 2>&1; then
            continue
        fi
    elif [ -n "$pkg_input" ] && [ "$pkg_input" != "all" ]; then
        case "$base" in
            "$pkg_input"-*) ;;
            *) continue ;;
        esac
    fi
    cp "$pkg" "$upload_dir/$base"
    count=$((count + 1))
done

if [ "$count" -gt 0 ]; then
    gh release upload "$active_tag" \
        --repo "$repo" \
        --clobber \
        "$upload_dir"/*.jpkg
    echo "Mirrored $count package asset(s) into $active_tag."
else
    echo "No package assets matched for $active_tag; rebuilding its INDEX only."
fi

active_dir="$tmp/active"
mkdir -p "$active_dir"
gh release download "$active_tag" \
    --repo "$repo" \
    --pattern "*.jpkg" \
    --dir "$active_dir" >/dev/null 2>&1 || true

PKG_DIR="$active_dir" \
OUT_INDEX="$active_dir/INDEX" \
STALE_LIST="$active_dir/.stale-assets" \
RECIPES_ROOT="$recipes_root" \
    "$workspace/scripts/gen-index.sh"

if [ -s "$active_dir/.stale-assets" ]; then
    while read -r asset; do
        [ -n "$asset" ] || continue
        echo "Deleting stale asset from $active_tag: $asset"
        gh release delete-asset "$active_tag" "$asset" --yes --repo "$repo" || true
    done < "$active_dir/.stale-assets"
fi

zstd -19 -f -o "$active_dir/INDEX.zst" "$active_dir/INDEX"

if [ -z "${JPKG_SIGNING_KEY:-}" ]; then
    echo "JPKG_SIGNING_KEY is required to sign $active_tag INDEX.zst." >&2
    exit 1
fi

sign_dir="$tmp/sign"
mkdir -p "$sign_dir"
printf '%s' "$JPKG_SIGNING_KEY" | base64 -d > "$sign_dir/jpkg-sign.sec"
chmod 600 "$sign_dir/jpkg-sign.sec"

if command -v jpkg >/dev/null 2>&1; then
    jpkg sign "$active_dir/INDEX.zst" --key "$sign_dir/jpkg-sign.sec"
elif command -v docker >/dev/null 2>&1; then
    docker run --rm \
        -v "$active_dir:/work" \
        -v "$sign_dir:/key:ro" \
        --entrypoint /bin/sh \
        "$signer_image" \
        -c 'jpkg sign /work/INDEX.zst --key /key/jpkg-sign.sec'
else
    echo "Neither jpkg nor docker is available to sign INDEX.zst." >&2
    exit 1
fi

rm -f "$sign_dir/jpkg-sign.sec"

gh release upload "$active_tag" \
    --repo "$repo" \
    --clobber \
    "$active_dir/INDEX.zst" \
    "$active_dir/INDEX.zst.sig"

echo "Open package release $active_tag INDEX refreshed."
