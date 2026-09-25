#!/bin/sh
# gen-index.sh — generate jpkg repository INDEX from a pool of .jpkg files.
#
# Reads .jpkg files from PKG_DIR, emits a deduplicated INDEX file and an
# optional list of stale (superseded) asset filenames to DELETE from the
# release. Exactly one entry per (name, arch) survives: the highest package
# version wins. Bare package versions are normalized as r0 for ordering, so
# 1.2.3-r1 correctly supersedes 1.2.3.
#
# Background: GitHub releases accumulate .jpkg assets across version bumps
# (filenames include the version, so `--clobber` doesn't help across a
# version change). Without dedup, two `[python3-aarch64]` sections end up
# in the INDEX and jpkg's TOML parser returns the FIRST one (= the older,
# stale version). This ambushed us three times in a single week with mksh,
# jmake, and python3. Duplicates are always a bug — per jonerix's package
# model, if you need two versions, use distinct package names.
#
# Recipe cross-checks (RECIPES_ROOT is the checked-out tree, main in CI):
#   - A package whose recipe is gone is an orphan and is flagged stale.
#   - A package built for an arch its recipe excludes (`arch = "..."`) is
#     flagged stale instead of indexed.
#   - A winner whose version is NEWER than its recipe was built from a tree
#     that is not the checked-out one (for example an unmerged branch). That
#     is an error: the script still writes the INDEX, then exits 1 so the
#     publish job stops before signing and uploading it. Set
#     ALLOW_INDEX_AHEAD_OF_RECIPE=1 to downgrade it to a warning.
#   - Runtime depends come from the recipe only when the recipe version
#     matches the indexed one; otherwise they come from the .jpkg's own
#     metadata, which describes the payload actually being served.
#
# Usage:
#   PKG_DIR=/var/cache/jpkg OUT_INDEX=/var/cache/jpkg/INDEX \
#   RECIPES_ROOT=/workspace/packages STALE_LIST=/tmp/stale.txt \
#     scripts/gen-index.sh

set -eu

PKG_DIR="${PKG_DIR:-/var/cache/jpkg}"
OUT_INDEX="${OUT_INDEX:-$PKG_DIR/INDEX}"
RECIPES_ROOT="${RECIPES_ROOT:-${GITHUB_WORKSPACE:-$PWD}/packages}"
STALE_LIST="${STALE_LIST:-$PKG_DIR/.stale-assets}"
ALLOW_INDEX_AHEAD_OF_RECIPE="${ALLOW_INDEX_AHEAD_OF_RECIPE:-0}"

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT INT TERM

DEDUP_DIR="$WORKDIR/dedup"
WINNERS="$WORKDIR/winners"
mkdir -p "$DEDUP_DIR"
: > "$WINNERS"
: > "$STALE_LIST"

# Read bytes 8..11 as little-endian uint32 (meta length in .jpkg header).
read_meta_len() {
    dd if="$1" bs=1 skip=8 count=4 2>/dev/null | od -An -tu4 | tr -d ' \n'
}

# Read the TOML metadata blob from a .jpkg.
read_meta() {
    _len="$(read_meta_len "$1")"
    [ -n "$_len" ] && [ "$_len" -gt 0 ] 2>/dev/null || return 1
    dd if="$1" bs=1 skip=12 count="$_len" 2>/dev/null
}

# Extract a "key = \"value\"" field from .jpkg metadata.
meta_field() {
    printf '%s' "$1" | grep "^$2 " | head -1 | sed 's/.*= *"\(.*\)"/\1/'
}

version_sort_key() {
    case "$1" in
        *-r[0-9]*) printf '%s\n' "$1" ;;
        *)         printf '%s-r0\n' "$1" ;;
    esac
}

# version_newer A B: succeed when version A sorts strictly above version B.
version_newer() {
    _vn_a="$(version_sort_key "$1")"
    _vn_b="$(version_sort_key "$2")"
    [ "$_vn_a" != "$_vn_b" ] || return 1
    [ "$(printf '%s\n%s\n' "$_vn_a" "$_vn_b" | sort -V | tail -n 1)" = "$_vn_a" ]
}

# recipe_field FILE KEY: first `KEY = "value"` line of a recipe (the recipes
# are not strict TOML, so no TOML parser; same extraction as ci-build-*.sh).
recipe_field() {
    grep "^$2[[:space:]]*=" "$1" | head -n 1 | sed 's/.*= *"\(.*\)".*/\1/'
}

# meta_runtime_depends META: the `runtime = [...]` array from the [depends]
# table of a .jpkg's own metadata, verbatim. sed only: the sparse builder
# image that also runs this script has no awk.
meta_runtime_depends() {
    printf '%s\n' "$1" | sed -n '/^\[depends\]$/,/^\[/{
/^runtime[[:space:]]*=/{
s/^runtime[[:space:]]*=[[:space:]]*//
p
q
}
}'
}

# --- Pass 1: collect (version, path) grouped by (name, arch) ---------------
for pkg in "$PKG_DIR"/*.jpkg; do
    [ -f "$pkg" ] || continue
    meta="$(read_meta "$pkg" || true)"
    if [ -z "$meta" ]; then
        echo "WARNING: could not read metadata from $(basename "$pkg"), skipping" >&2
        continue
    fi
    name="$(meta_field "$meta" name)"
    version="$(meta_field "$meta" version)"
    arch="$(meta_field "$meta" arch)"
    [ -n "$name" ] || { echo "WARNING: no name in $(basename "$pkg"), skipping" >&2; continue; }
    [ -n "$arch" ]    || arch="x86_64"
    [ -n "$version" ] || version="0"

    # Sanitize for use as a filename; name/arch shouldn't contain these but be safe.
    safe="$(printf '%s' "${name}__${arch}" | tr '/' '_')"
    printf '%s\t%s\t%s\n' "$(version_sort_key "$version")" "$version" "$pkg" >> "$DEDUP_DIR/$safe"
done

# --- Pass 2: pick highest version per (name, arch); list losers ------------
dup_groups=0
for group in "$DEDUP_DIR"/*; do
    [ -f "$group" ] || continue
    sorted="$(sort -t '	' -k1,1V "$group")"
    n="$(printf '%s\n' "$sorted" | wc -l | tr -d ' ')"

    # Winner is the last line (highest normalized version key).
    winner_line="$(printf '%s\n' "$sorted" | tail -n 1)"
    winner_version="$(printf '%s' "$winner_line" | cut -f2)"
    winner_path="$(printf '%s' "$winner_line" | cut -f3)"
    printf '%s\n' "$winner_path" >> "$WINNERS"

    if [ "$n" -gt 1 ]; then
        dup_groups=$((dup_groups + 1))
        gname="$(basename "$group" | sed 's/__/ (/' | sed 's/$/)/')"
        echo "::warning::duplicate versions for $gname — keeping $winner_version, dropping $((n - 1))"
        printf '%s\n' "$sorted" | head -n "$((n - 1))" | while IFS='	' read -r _key v p; do
            echo "  discarding: $(basename "$p") (version $v)"
            basename "$p" >> "$STALE_LIST"
        done
    fi
done

if [ "$dup_groups" -gt 0 ]; then
    echo "Found $dup_groups duplicate group(s); $(wc -l < "$STALE_LIST" | tr -d ' ') asset(s) flagged stale."
else
    echo "No duplicate groups — INDEX is clean."
fi

# --- Pass 3: emit INDEX with one section per winner ------------------------
{
    printf '[meta]\n'
    printf 'timestamp = "%s"\n\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
} > "$OUT_INDEX"

emitted=0
ahead=0
while read -r pkg; do
    [ -n "$pkg" ] || continue
    pkg_base="$(basename "$pkg")"
    meta="$(read_meta "$pkg" || true)"
    [ -n "$meta" ] || { echo "WARNING: meta gone for $pkg_base, skipping" >&2; continue; }

    name="$(meta_field "$meta" name)"
    version="$(meta_field "$meta" version)"
    license="$(meta_field "$meta" license)"
    desc="$(meta_field "$meta" description)"
    arch="$(meta_field "$meta" arch)"
    [ -n "$arch" ] || arch="x86_64"

    recipe_file=""
    for _pkgdir in core develop extra; do
        _candidate="$RECIPES_ROOT/$_pkgdir/$name/recipe.toml"
        if [ -f "$_candidate" ]; then
            recipe_file="$_candidate"
            break
        fi
    done
    if [ -z "$recipe_file" ]; then
        # No recipe means the package was removed from the repo.
        # Flag the orphan .jpkg for deletion instead of indexing it.
        echo "orphan: no recipe for $name — flagging $pkg_base for removal" >&2
        printf '%s\n' "$pkg_base" >> "$STALE_LIST"
        continue
    fi

    # A recipe pinned to one arch never publishes the other; an asset for the
    # excluded arch predates the pin and must not stay in the INDEX.
    recipe_arch="$(recipe_field "$recipe_file" arch)"
    if [ -n "$recipe_arch" ] && [ "$recipe_arch" != "$arch" ]; then
        echo "arch-excluded: $name is arch=$recipe_arch — flagging $pkg_base for removal" >&2
        printf '%s\n' "$pkg_base" >> "$STALE_LIST"
        continue
    fi

    # Prefer recipe.toml for runtime-depends (source of truth) — the .jpkg
    # metadata may have been written by an older jpkg that didn't record them.
    # That only holds while the recipe describes this exact build; when the
    # versions differ, the .jpkg's own [depends] describes the payload.
    recipe_version="$(recipe_field "$recipe_file" version)"
    if [ -z "$recipe_version" ] || [ "$(version_sort_key "$recipe_version")" = "$(version_sort_key "${version:-0}")" ]; then
        depends_arr="$(grep '^runtime = ' "$recipe_file" | head -1 | sed 's/^runtime = //')"
    else
        if version_newer "${version:-0}" "$recipe_version"; then
            ahead=$((ahead + 1))
            if [ "$ALLOW_INDEX_AHEAD_OF_RECIPE" = "1" ]; then
                echo "::warning::$name ($arch): indexed $version is newer than recipe $recipe_version ($recipe_file); it was built from a different tree"
            else
                echo "::error::$name ($arch): indexed $version is newer than recipe $recipe_version ($recipe_file); it was built from a different tree (land that recipe first, or set ALLOW_INDEX_AHEAD_OF_RECIPE=1)"
            fi
        else
            echo "note: $name ($arch): recipe $recipe_version is not published yet; indexing $version"
        fi
        depends_arr="$(meta_runtime_depends "$meta")"
        # A .jpkg written by an old jpkg may not record its depends at all;
        # the recipe's list is then the best available approximation.
        [ -n "$depends_arr" ] || depends_arr="$(grep '^runtime = ' "$recipe_file" | head -1 | sed 's/^runtime = //')"
    fi
    [ "$depends_arr" = "[]" ] && depends_arr=""

    sha256="$(sha256sum "$pkg" | cut -d' ' -f1)"
    size_val="$(stat -c %s "$pkg" 2>/dev/null || echo 0)"

    [ -n "$name" ]    || name="$(echo "$pkg_base" | sed 's/-[^-]*-[^-]*\.jpkg$//')"
    [ -n "$version" ] || version="0"

    # Section key is "name-arch" so x86_64 and aarch64 coexist.
    # jpkg's repo_find_package() tries "name-arch" before falling back to "name".
    section="${name}-${arch}"
    {
        printf '[%s]\n' "$section"
        printf 'version = "%s"\n' "$version"
        [ -n "$license" ]     && printf 'license = "%s"\n' "$license"
        [ -n "$desc" ]        && printf 'description = "%s"\n' "$desc"
        printf 'arch = "%s"\n' "$arch"
        [ -n "$depends_arr" ] && printf 'depends = %s\n' "$depends_arr"
        [ -n "$sha256" ]      && printf 'sha256 = "%s"\n' "$sha256"
        printf 'size = %s\n' "$size_val"
        printf '\n'
    } >> "$OUT_INDEX"

    echo "  indexed: ${section}-${version}"
    emitted=$((emitted + 1))
done < "$WINNERS"

echo "INDEX generation done: $emitted package(s)."

if [ "$ahead" -gt 0 ]; then
    if [ "$ALLOW_INDEX_AHEAD_OF_RECIPE" = "1" ]; then
        echo "::warning::$ahead indexed package(s) are newer than their recipes (allowed by ALLOW_INDEX_AHEAD_OF_RECIPE=1)"
    else
        echo "::error::$ahead indexed package(s) are newer than their recipes in $RECIPES_ROOT; refusing to bless this INDEX"
        exit 1
    fi
fi
