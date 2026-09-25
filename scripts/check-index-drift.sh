#!/bin/sh
# check-index-drift.sh — compare the published package INDEX with the recipes.
#
# The rolling `packages` release is what every jonerix host installs from,
# and it is filled by hand-dispatched workflows and local uploads. This
# script reports where it has drifted from the recipes in this tree:
#
#   ERROR   index-ahead     INDEX version is newer than the recipe: the build
#                           came from a tree that is not this one (e.g. an
#                           unmerged branch), so this tree cannot rebuild what
#                           hosts run, and the next regen from here fails.
#   ERROR   arch-excluded   INDEX carries an arch the recipe's `arch =` pin
#                           excludes.
#   ERROR   no-recipe       INDEX carries a package with no recipe here.
#   WARN    unpublished     recipe version is newer than the INDEX.
#   WARN    missing         recipe (for this arch) has never been published.
#
# GPL/LGPL/AGPL recipes are skipped: the CI build gate never publishes them.
# Exit status: 0 when there are no ERRORs (1 with STRICT=1 and any WARN),
# 1 on drift, 2 on usage or fetch errors.
#
# Usage:
#   scripts/check-index-drift.sh                   # fetch the live INDEX.zst
#   INDEX_FILE=/path/INDEX scripts/check-index-drift.sh
#   INDEX_FILE=/path/INDEX.zst scripts/check-index-drift.sh
#
# Environment:
#   INDEX_URL     default https://github.com/stormj-UH/jonerix/releases/download/packages/INDEX.zst
#   INDEX_FILE    use a local INDEX (plain or .zst) instead of fetching
#   RECIPES_ROOT  default <repo>/packages
#   ARCHES        default "aarch64 x86_64"
#   STRICT        1 = WARN findings also fail the run
#
# POSIX sh; needs curl (unless INDEX_FILE is plain), zstd, awk, sort -V.
#
# SPDX-License-Identifier: 0BSD

set -eu

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
RECIPES_ROOT="${RECIPES_ROOT:-$REPO_ROOT/packages}"
INDEX_URL="${INDEX_URL:-https://github.com/stormj-UH/jonerix/releases/download/packages/INDEX.zst}"
INDEX_FILE="${INDEX_FILE:-}"
ARCHES="${ARCHES:-aarch64 x86_64}"
STRICT="${STRICT:-0}"

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT INT TERM

errors=0
warnings=0

report() {
    # report LEVEL KIND MESSAGE
    if [ "$1" = ERROR ]; then
        errors=$((errors + 1))
    else
        warnings=$((warnings + 1))
    fi
    if [ "${GITHUB_ACTIONS:-}" = "true" ]; then
        if [ "$1" = ERROR ]; then
            printf '::error::%s: %s\n' "$2" "$3"
        else
            printf '::warning::%s: %s\n' "$2" "$3"
        fi
    else
        printf '%-5s %-13s %s\n' "$1" "$2" "$3"
    fi
}

die() {
    printf 'check-index-drift: %s\n' "$1" >&2
    exit 2
}

# Same normalisation as scripts/gen-index.sh: a bare version sorts as -r0.
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

recipe_field() {
    grep "^$2[[:space:]]*=" "$1" | head -n 1 | sed 's/.*= *"\(.*\)".*/\1/'
}

# ── Load the INDEX ──────────────────────────────────────────────────────────
index="$WORKDIR/INDEX"
if [ -n "$INDEX_FILE" ]; then
    [ -f "$INDEX_FILE" ] || die "INDEX_FILE not found: $INDEX_FILE"
    case "$INDEX_FILE" in
        *.zst) zstd -dcq "$INDEX_FILE" > "$index" || die "cannot decompress $INDEX_FILE" ;;
        *)     cp "$INDEX_FILE" "$index" ;;
    esac
    index_src="$INDEX_FILE"
else
    command -v curl >/dev/null 2>&1 || die "curl is required to fetch $INDEX_URL"
    curl -fsSL --retry 3 -o "$WORKDIR/INDEX.zst" "$INDEX_URL" || die "cannot fetch $INDEX_URL"
    zstd -dcq "$WORKDIR/INDEX.zst" > "$index" || die "cannot decompress $INDEX_URL"
    index_src="$INDEX_URL"
fi

# section<TAB>version for every [name-arch] table.
entries="$WORKDIR/entries"
awk '
    /^\[/ {
        sec = $0; sub(/^\[/, "", sec); sub(/\].*$/, "", sec); next
    }
    sec != "" && sec != "meta" && /^version[[:space:]]*=/ {
        v = $0; sub(/^[^"]*"/, "", v); sub(/".*$/, "", v)
        printf "%s\t%s\n", sec, v; sec = ""
    }
' "$index" > "$entries"

stamp=$(awk -F'"' '/^timestamp[[:space:]]*=/ { print $2; exit }' "$index")
printf 'INDEX: %s (timestamp %s, %s entries)\n' "$index_src" "${stamp:-unknown}" "$(wc -l < "$entries" | tr -d ' ')"
printf 'Recipes: %s\n\n' "$RECIPES_ROOT"

# ── Recipes vs INDEX ────────────────────────────────────────────────────────
names="$WORKDIR/recipe-names"
: > "$names"
checked=0
for recipe in "$RECIPES_ROOT"/*/*/recipe.toml; do
    [ -f "$recipe" ] || continue
    name=$(recipe_field "$recipe" name)
    [ -n "$name" ] || name=$(basename "$(dirname "$recipe")")
    version=$(recipe_field "$recipe" version)
    license=$(recipe_field "$recipe" license)
    pin=$(recipe_field "$recipe" arch)
    printf '%s\t%s\n' "$name" "$pin" >> "$names"

    case "$license" in
        GPL-*|LGPL-*|AGPL-*) continue ;;
    esac
    [ -n "$version" ] || continue

    for arch in $ARCHES; do
        if [ -n "$pin" ] && [ "$pin" != "$arch" ]; then
            continue
        fi
        checked=$((checked + 1))
        published=$(awk -F'\t' -v s="$name-$arch" '$1 == s { print $2; exit }' "$entries")
        if [ -z "$published" ]; then
            report WARN missing "$name $version ($arch) has never been published"
        elif [ "$(version_sort_key "$published")" = "$(version_sort_key "$version")" ]; then
            :
        elif version_newer "$published" "$version"; then
            report ERROR index-ahead "$name ($arch): INDEX has $published but the recipe is $version; it was published from another tree"
        else
            report WARN unpublished "$name ($arch): recipe $version is not published (INDEX has $published)"
        fi
    done
done

# ── INDEX entries with no recipe, or for an excluded arch ───────────────────
while IFS='	' read -r section published; do
    arch=""
    for a in $ARCHES; do
        case "$section" in
            *-"$a") arch="$a"; name=${section%-"$a"}; break ;;
        esac
    done
    if [ -z "$arch" ]; then
        report WARN unknown "INDEX section [$section] has no recognised arch suffix"
        continue
    fi
    pin=$(awk -F'\t' -v n="$name" '$1 == n { print $2; found = 1; exit } END { if (!found) print "-none-" }' "$names")
    if [ "$pin" = "-none-" ]; then
        report ERROR no-recipe "INDEX has $name $published ($arch) but no recipe exists for it"
    elif [ -n "$pin" ] && [ "$pin" != "$arch" ]; then
        report ERROR arch-excluded "INDEX has $name $published for $arch but the recipe is arch = \"$pin\""
    fi
done < "$entries"

printf '\nChecked %s recipe/arch pairs: %s error(s), %s warning(s).\n' "$checked" "$errors" "$warnings"

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    {
        printf '### Package INDEX drift\n\n'
        printf 'INDEX timestamp `%s`; %s recipe/arch pairs checked.\n\n' "${stamp:-unknown}" "$checked"
        printf '%s\n' "- errors: $errors" "- warnings: $warnings"
    } >> "$GITHUB_STEP_SUMMARY"
fi

if [ "$errors" -gt 0 ]; then
    exit 1
fi
if [ "$STRICT" = "1" ] && [ "$warnings" -gt 0 ]; then
    exit 1
fi
exit 0
